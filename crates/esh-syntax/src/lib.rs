//! Source forms and spans for native and WebAssembly consumers.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_FORMS: usize = 50_000;
pub const MAX_NESTING: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Form {
    pub value: FormValue,
    pub span: Span,
    pub source: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum FormValue {
    Atom(String),
    Text(String),
    List(Vec<Form>),
}

impl Form {
    pub fn atom(&self) -> Option<&str> {
        match &self.value {
            FormValue::Atom(s) => Some(s),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<&str> {
        match &self.value {
            FormValue::Text(s) => Some(s),
            _ => None,
        }
    }
    pub fn list(&self) -> Option<&[Form]> {
        match &self.value {
            FormValue::List(xs) => Some(xs),
            _ => None,
        }
    }
    pub fn is_atom(&self, name: &str) -> bool {
        self.atom() == Some(name)
    }
    pub fn head(&self) -> Option<&str> {
        self.list()?.first()?.atom()
    }
    pub fn has_head(&self, name: &str) -> bool {
        self.head() == Some(name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub source: usize,
    pub span: Span,
}

impl Diagnostic {
    pub fn at(form: &Form, code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            source: form.source,
            span: form.span,
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "{}:{}: {}: {}",
            self.span.line, self.span.column, self.code, self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

struct Parser<'a> {
    input: &'a str,
    source: usize,
    offset: usize,
    line: usize,
    column: usize,
    forms: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.offset += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    fn mark(&self) -> (usize, usize, usize) {
        (self.offset, self.line, self.column)
    }

    fn span(&self, start: (usize, usize, usize)) -> Span {
        Span {
            start: start.0,
            end: self.offset,
            line: start.1,
            column: start.2,
            end_line: self.line,
            end_column: self.column,
        }
    }

    fn error(&self, start: (usize, usize, usize), code: &str, message: &str) -> Diagnostic {
        Diagnostic {
            code: code.into(),
            message: message.into(),
            source: self.source,
            span: self.span(start),
        }
    }

    fn skip_space(&mut self) {
        loop {
            match self.peek() {
                Some(c) if source_space(c) => {
                    self.advance();
                }
                Some(';') => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.advance();
                    }
                }
                _ => return,
            }
        }
    }

    fn form(&mut self, depth: usize) -> Result<Form, Diagnostic> {
        self.skip_space();
        let start = self.mark();
        self.forms += 1;
        if self.forms > MAX_FORMS || depth > MAX_NESTING {
            return Err(self.error(
                start,
                "syntax",
                "Source exceeds the token or nesting limit.",
            ));
        }
        let c = self
            .advance()
            .ok_or_else(|| self.error(start, "syntax", "Expected an expression."))?;
        let value = match c {
            '(' => {
                let mut values = Vec::new();
                loop {
                    self.skip_space();
                    match self.peek() {
                        None => return Err(self.error(start, "syntax", "Unclosed list.")),
                        Some(')') => {
                            self.advance();
                            break;
                        }
                        _ => values.push(self.form(depth + 1)?),
                    }
                }
                FormValue::List(values)
            }
            ')' => return Err(self.error(start, "syntax", "Unexpected ')'.")),
            '"' => {
                let mut closed = false;
                while let Some(next) = self.advance() {
                    if next == '"' {
                        closed = true;
                        break;
                    }
                    if next == '\\' && self.advance().is_none() {
                        break;
                    }
                }
                if !closed {
                    return Err(self.error(start, "syntax", "Unclosed string."));
                }
                let raw = &self.input[start.0..self.offset];
                let text: String = serde_json::from_str(raw).map_err(|_| {
                    self.error(start, "syntax", "Invalid JSON string escape or character.")
                })?;
                FormValue::Text(text)
            }
            _ => {
                while self
                    .peek()
                    .is_some_and(|c| !source_space(c) && !matches!(c, '(' | ')' | ';' | '"'))
                {
                    self.advance();
                }
                let text = &self.input[start.0..self.offset];
                if text.chars().any(|c| (c as u32) < 32 || c == '\u{7f}') {
                    return Err(self.error(start, "syntax", "Control character in identifier."));
                }
                FormValue::Atom(text.into())
            }
        };
        Ok(Form {
            value,
            span: self.span(start),
            source: self.source,
        })
    }
}

fn source_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}

pub fn parse(input: &str, source: usize) -> Result<Form, Diagnostic> {
    let mut parser = Parser {
        input,
        source,
        offset: 0,
        line: 1,
        column: 1,
        forms: 0,
    };
    if input.len() > MAX_SOURCE_BYTES {
        return Err(parser.error(parser.mark(), "source-limit", "Source exceeds 256 KiB."));
    }
    let form = parser.form(0)?;
    parser.skip_space();
    if parser.peek().is_some() {
        return Err(parser.error(
            parser.mark(),
            "syntax",
            "Expected a single script or module.",
        ));
    }
    Ok(form)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_json_strings_and_unicode_spans() {
        let form = parse("; comment\n(script (text.concat \"λ\\n\" \"\\u0041\"))", 3).unwrap();
        assert!(form.has_head("script"));
        assert_eq!((form.source, form.span.line, form.span.column), (3, 2, 1));
        let call = &form.list().unwrap()[1];
        let args = call.list().unwrap();
        assert_eq!(args[1].text(), Some("λ\n"));
        assert_eq!(args[2].text(), Some("A"));
        assert_eq!(args[2].span.column, 28);
    }

    #[test]
    fn malformed_sources_are_diagnostics() {
        for source in [
            "",
            "(script unit",
            "(script unit) unit",
            "(script \"bad\\q\")",
            "(script \"\\uD800\")",
            "(script \"unfinished)",
            "(script bad\u{1b}name)",
            ")",
        ] {
            assert_eq!(parse(source, 0).unwrap_err().code, "syntax", "{source:?}");
        }
    }

    #[test]
    fn structural_and_size_limits() {
        assert_eq!(
            parse(&" ".repeat(MAX_SOURCE_BYTES + 1), 0)
                .unwrap_err()
                .code,
            "source-limit"
        );
        let nested = format!("{}unit{}", "(".repeat(130), ")".repeat(130));
        assert_eq!(parse(&nested, 0).unwrap_err().code, "syntax");
        let many = format!("({})", "x ".repeat(MAX_FORMS));
        assert_eq!(parse(&many, 0).unwrap_err().code, "syntax");
    }

    #[test]
    fn existing_examples_parse_without_grammar_changes() {
        for source in [
            include_str!("../../../examples/copy.esh"),
            include_str!("../../../examples/functions.esh"),
            include_str!("../../../examples/lib/io.esh"),
        ] {
            parse(source, 0).unwrap();
        }
    }
}
