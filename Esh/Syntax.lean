import Esh.Core

namespace Esh
open Lean

structure Position where
  line : Nat := 1
  column : Nat := 1
  deriving Repr, Inhabited

def Position.advance (p : Position) (c : Char) : Position :=
  if c == '\n' then ⟨p.line + 1, 1⟩ else ⟨p.line, p.column + 1⟩

structure Span where
  start : Position
  stop : Position
  deriving Repr, Inhabited

def Span.json (s : Span) : Lean.Json := Lean.Json.mkObj [
  ("line", toJson s.start.line), ("column", toJson s.start.column),
  ("endLine", toJson s.stop.line), ("endColumn", toJson s.stop.column)]

structure Diagnostic where
  span : Span
  code : String
  message : String
  deriving Repr, Inhabited

def Diagnostic.json (d : Diagnostic) : Lean.Json := Lean.Json.mkObj [
  ("span", d.span.json), ("code", toJson d.code), ("message", toJson d.message)]

inductive TokenKind where
  | open | close | atom (text : String) | string (text : String)
  deriving Repr

structure Token where
  kind : TokenKind
  span : Span
  deriving Repr

inductive Form where
  | atom (span : Span) (text : String)
  | string (span : Span) (text : String)
  | list (span : Span) (items : List Form)
  deriving Repr, Inhabited

def Form.span : Form → Span
  | .atom s _ | .string s _ | .list s _ => s

def syntaxError (span : Span) (message : String) : Diagnostic := ⟨span, "syntax", message⟩

private def takeAtom (pos : Position) (acc : List Char) : List Char → String × Position × List Char
  | [] => (String.ofList acc.reverse, pos, [])
  | c :: cs =>
      if c.isWhitespace || c == '(' || c == ')' || c == ';' || c == '"' then
        (String.ofList acc.reverse, pos, c :: cs)
      else takeAtom (pos.advance c) (c :: acc) cs

private def skipComment (pos : Position) : List Char → Position × List Char
  | [] => (pos, [])
  | c :: cs => if c == '\n' then (pos.advance c, cs) else skipComment (pos.advance c) cs

private def takeString (start pos : Position) (acc : List Char) :
    List Char → Except Diagnostic (String × Position × List Char)
  | [] => .error (syntaxError ⟨start, pos⟩ "unterminated string")
  | '\\' :: c :: cs => takeString start ((pos.advance '\\').advance c) (c :: '\\' :: acc) cs
  | '"' :: cs => do
      let stop := pos.advance '"'
      let raw := String.ofList ('"' :: acc).reverse
      let j ← (Lean.Json.parse raw).mapError (syntaxError ⟨start, stop⟩)
      let s ← j.getStr?.mapError (syntaxError ⟨start, stop⟩)
      pure (s, stop, cs)
  | c :: cs => takeString start (pos.advance c) (c :: acc) cs

private partial def tokenizeAux (pos : Position) (tokens : List Token) :
    List Char → Except Diagnostic (List Token)
  | [] => .ok tokens.reverse
  | c :: cs => do
      if c.isWhitespace then tokenizeAux (pos.advance c) tokens cs
      else if c == ';' then
        let (next, rest) := skipComment (pos.advance c) cs
        tokenizeAux next tokens rest
      else if c == '(' || c == ')' then
        let stop := pos.advance c
        let kind := if c == '(' then TokenKind.open else TokenKind.close
        tokenizeAux stop (⟨kind, ⟨pos, stop⟩⟩ :: tokens) cs
      else if c == '"' then
        let (s, stop, rest) ← takeString pos (pos.advance c) ['"'] cs
        tokenizeAux stop (⟨.string s, ⟨pos, stop⟩⟩ :: tokens) rest
      else
        let (s, stop, rest) := takeAtom pos [] (c :: cs)
        if s.toList.any (fun ch => ch.toNat < 32 || ch.toNat == 127) then
          throw (syntaxError ⟨pos, stop⟩ "control characters are not allowed in identifiers")
        tokenizeAux stop (⟨.atom s, ⟨pos, stop⟩⟩ :: tokens) rest

mutual
  private partial def parseOne (depth : Nat) :
      List Token → Except Diagnostic (Form × List Token)
    | [] => .error (syntaxError default "expected an expression")
    | token :: rest => do
        if depth > 128 then throw (syntaxError token.span "nesting exceeds 128 levels")
        match token.kind with
        | .atom s => pure (.atom token.span s, rest)
        | .string s => pure (.string token.span s, rest)
        | .close => throw (syntaxError token.span "unexpected closing parenthesis")
        | .open => parseItems depth token.span.start [] rest

  private partial def parseItems (depth : Nat) (start : Position) (acc : List Form) :
      List Token → Except Diagnostic (Form × List Token)
    | [] => .error (syntaxError ⟨start, start⟩ "unclosed parenthesis")
    | token :: rest => do
        match token.kind with
        | .close => pure (.list ⟨start, token.span.stop⟩ acc.reverse, rest)
        | _ =>
            let (item, remaining) ← parseOne (depth + 1) (token :: rest)
            parseItems depth start (item :: acc) remaining
end

def parse (source : String) : Except Diagnostic Form := do
  if source.utf8ByteSize > 262144 then
    throw (syntaxError default "source exceeds 256 KiB")
  let tokens ← tokenizeAux {} [] source.toList
  if tokens.length > 50000 then
    throw (syntaxError default "source exceeds the token limit (50000)")
  let (form, rest) ← parseOne 0 tokens
  match rest with
  | [] => pure form
  | t :: _ => throw (syntaxError t.span "expected exactly one script form")

end Esh
