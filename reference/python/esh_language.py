"""Source elaboration for esh. Executable terms are rechecked by the Lean kernel.

This module owns source diagnostics, rank-1 inference, module loading, and review
metadata. It does not execute script computations or filesystem primitives.
"""
from dataclasses import dataclass, field
from pathlib import Path
import json
import os
import stat


MAX_SOURCE = 256 * 1024
RIGHTS = ("read", "write", "mkdir", "remove")
BUILTINS = {"text.concat", "text.eq", "fs.read-text", "fs.write-text",
            "fs.list", "fs.mkdir", "fs.remove"}


class Diagnostic(Exception):
    def __init__(self, form, code, message):
        self.form, self.code, self.message = form, code, message
        super().__init__(message)

    def report(self):
        result = {"ok": False, "diagnostic": {"code": self.code,
                "message": self.message, "span": self.form.span,
                "source": self.form.source}}
        if hasattr(self, "sources"):
            result["sources"] = self.sources
            result["entry"] = next((s["name"] for s in self.sources if s["entry"]), "<buffer>")
        return result


@dataclass
class Form:
    value: object
    span: dict
    source: str
    quoted: bool = False

    def atom(self, text=None):
        return isinstance(self.value, str) and not self.quoted and (text is None or self.value == text)

    def head(self, text=None):
        return isinstance(self.value, list) and bool(self.value) and self.value[0].atom(text)


def parse(source, name="<buffer>"):
    pos, line, column, count = 0, 1, 1, 0

    def at(start=None):
        sl, sc = start or (line, column)
        return {"line": sl, "column": sc, "endLine": line, "endColumn": column}

    def fail(code, message, start=None):
        raise Diagnostic(Form("", at(start), name), "syntax" if code in ("parse", "lex") else code, message)

    if len(source.encode("utf-8")) > MAX_SOURCE:
        fail("source-limit", "Source exceeds 256 KiB.")

    def advance():
        nonlocal pos, line, column
        c = source[pos]
        pos += 1
        if c == "\n":
            line, column = line + 1, 1
        else:
            column += 1
        return c

    def whitespace():
        while pos < len(source):
            if source[pos].isspace():
                advance()
            elif source[pos] == ";":
                while pos < len(source) and source[pos] != "\n":
                    advance()
            else:
                break

    def form(depth):
        nonlocal count
        whitespace()
        start = line, column
        count += 1
        if count > 50000 or depth > 128:
            fail("syntax", "Source exceeds the token or nesting limit.", start)
        if pos == len(source):
            fail("parse", "Expected an expression.", start)
        c = advance()
        if c == "(":
            values = []
            while True:
                whitespace()
                if pos == len(source):
                    fail("parse", "Unclosed list.", start)
                if source[pos] == ")":
                    advance()
                    return Form(values, at(start), name)
                values.append(form(depth + 1))
        if c == ")":
            fail("parse", "Unexpected ')'.", start)
        if c == '"':
            raw = ['"']
            while pos < len(source):
                c = advance()
                raw.append(c)
                if c == '"':
                    try:
                        text = json.loads("".join(raw))
                        text.encode("utf-8")
                    except (ValueError, UnicodeError):
                        fail("lex", "Invalid JSON string escape or character.", start)
                    return Form(text, at(start), name, True)
                if c == "\\":
                    if pos == len(source):
                        break
                    raw.append(advance())
            fail("lex", "Unclosed string.", start)
        text = c
        while pos < len(source) and not source[pos].isspace() and source[pos] not in '();"':
            text += advance()
        if any(ord(x) < 32 or ord(x) == 127 for x in text):
            fail("lex", "Control character in identifier.", start)
        return Form(text, at(start), name)

    result = form(0)
    whitespace()
    if pos != len(source):
        fail("parse", "Expected a single script or module.")
    return result


def variable(x):
    return isinstance(x, tuple) and x and x[0] in ("var", "rvar", "evar", "pvar")


def free(x):
    if variable(x):
        return {x}
    if isinstance(x, tuple):
        result = set()
        for item in x:
            result |= free(item)
        return result
    return set()


class Solver:
    def __init__(self):
        self.next = 0
        self.sub = {}
        self.rigid = set()
        self.names = {}
        self.rights = {}

    def fresh(self, kind="var", name=None, rigid=False):
        n = self.next
        self.next += 1
        v = kind, n
        self.names[v] = name or {"var": "T", "rvar": "R", "evar": "E", "pvar": "P"}[kind] + str(n)
        if rigid:
            self.rigid.add(v)
        if kind == "pvar":
            self.rights[v] = set()
        return v

    def resolve(self, x):
        if variable(x):
            if x[0] == "pvar":
                return tuple(p for p in RIGHTS if p in self.rights[x])
            if x in self.sub:
                return self.resolve(self.sub[x])
            return x
        if isinstance(x, tuple):
            if x and x[0] == "fn":
                return ("fn", tuple(self.resolve(t) for t in x[1]), self.resolve(x[2]), self.effects(x[3]))
            return tuple(self.resolve(t) for t in x)
        return x

    def effects(self, effects):
        result = []
        for e in effects:
            if variable(e) and e[0] == "evar" and e in self.sub:
                terms = self.effects(self.sub[e])
            else:
                terms = (self.resolve(e),)
            for term in terms:
                if term not in result:
                    result.append(term)
        return tuple(result)

    def bind(self, v, x, form):
        x = self.resolve(x)
        if v == x:
            return
        if v in self.rigid:
            raise Diagnostic(form, "type-mismatch", "A universally quantified parameter cannot be specialized inside its definition.")
        if v in free(x):
            raise Diagnostic(form, "recursive-type", "Recursive types are outside this stage.")
        self.sub[v] = x

    def region(self, a, b, form):
        a, b = self.resolve(a), self.resolve(b)
        if a == b:
            return
        if variable(a) and a not in self.rigid:
            self.bind(a, b, form)
        elif variable(b) and b not in self.rigid:
            self.bind(b, a, form)
        else:
            raise Diagnostic(form, "region-mismatch", "Capability regions do not match.")

    def effect_equal(self, a, b, form):
        a, b = self.effects(a), self.effects(b)
        if set(a) == set(b):
            return
        if len(a) == 1 and variable(a[0]) and a[0] not in self.rigid:
            self.bind(a[0], b, form)
        elif len(b) == 1 and variable(b[0]) and b[0] not in self.rigid:
            self.bind(b[0], a, form)
        else:
            raise Diagnostic(form, "effect-annotation-required", "Cannot determine an effect equation without choosing a partition; add parameter/effect annotations.")

    def include(self, actual, allowed, form):
        actual, allowed = self.effects(actual), self.effects(allowed)
        if len(allowed) == 1 and variable(allowed[0]) and allowed[0] not in self.rigid:
            self.bind(allowed[0], actual, form)
            return
        if not set(actual) <= set(allowed):
            code = "effect-annotation-required" if any(variable(x) and x not in self.rigid for x in actual + allowed) else "effect-bound"
            raise Diagnostic(form, code, "The effect bound does not establish all operations performed here; add a sufficient annotation.")

    def unify(self, a, b, form):
        a, b = self.resolve(a), self.resolve(b)
        if a == b:
            return
        if variable(a) and a[0] == "var" and a not in self.rigid:
            self.bind(a, b, form)
            return
        if variable(b) and b[0] == "var" and b not in self.rigid:
            self.bind(b, a, form)
            return
        if isinstance(a, tuple) and isinstance(b, tuple) and a[0] == b[0]:
            if a[0] in ("list", "result"):
                self.unify(a[1], b[1], form)
                return
            if a[0] == "cap" and a[2] == b[2]:
                self.region(a[1], b[1], form)
                return
            if a[0] == "fn" and len(a[1]) == len(b[1]):
                for x, y in zip(a[1], b[1]):
                    self.unify(x, y, form)
                self.unify(a[2], b[2], form)
                self.effect_equal(a[3], b[3], form)
                return
        raise Diagnostic(form, "type-mismatch", f"Expected {self.show(b)}, got {self.show(a)}.")

    def show(self, t):
        t = self.resolve(t)
        if isinstance(t, str):
            return t
        if variable(t):
            return self.names[t]
        if t[0] == "cap":
            return f"(Cap {self.show(t[1])} ({' '.join(t[2])}))"
        if t[0] == "fn":
            return f"(Fn ({' '.join(self.show(x) for x in t[1])}) {self.show(t[2])} {self.show_effects(t[3])})"
        if t[0] == "list":
            return f"(List {self.show(t[1])})"
        if t[0] == "result":
            return f"(Result {self.show(t[1])} FsError)"
        raise ValueError("invalid internal type")

    def show_effects(self, es):
        def term(e):
            return self.names[e] if variable(e) else f"(fs.{e[2]} {self.show(e[1])})"
        return "(effects" + "".join(" " + term(e) for e in self.effects(es)) + ")"


@dataclass
class Scheme:
    type: object
    quant: tuple = ()


@dataclass
class Binding:
    id: int
    name: str
    scheme: Scheme
    infer: bool = False
    authority: bool = True
    pinned: set = field(default_factory=set)
    form: object = None


@dataclass
class Term:
    type: object
    effects: tuple
    ir: dict
    authority: bool = False
    pinned: set = field(default_factory=set)


@dataclass
class Module:
    name: str
    path: object
    source: str
    form: Form
    imports: dict = field(default_factory=dict)
    definitions: dict = field(default_factory=dict)
    aliases: dict = field(default_factory=dict)
    caps: dict = field(default_factory=dict)
    headers: dict = field(default_factory=dict)
    body: object = None
    exports: set = field(default_factory=set)


def fields(form, allowed, offset=1):
    result = {}
    for f in form.value[offset:]:
        if not f.head() or f.value[0].value not in allowed:
            raise Diagnostic(f, "declaration", "Unexpected declaration field.")
        name = f.value[0].value
        if name in result:
            raise Diagnostic(f, "annotation", f"Duplicate {name} field.")
        if name == "export" and len(f.value) != 1:
            raise Diagnostic(f, "export", "Expected (export).")
        result[name] = f
    return result


def one(form, field_name):
    if len(form.value) != 2:
        raise Diagnostic(form, "annotation", f"{field_name} requires one value.")
    return form.value[1]


def refs(ir):
    if ir["op"] == "ref":
        return {ir["binding"]}
    if ir["op"] in ("let", "let-ok"):
        return refs(ir["value"]) | (refs(ir["body"]) - {ir["binding"]})
    if ir["op"] == "fn":
        return set().union(*(refs(v) for v in ir["captures"])) if ir["captures"] else set()
    result = set()
    for value in ir.values():
        if isinstance(value, dict) and "op" in value:
            result |= refs(value)
        elif isinstance(value, list):
            for child in value:
                if isinstance(child, dict) and "op" in child:
                    result |= refs(child)
    return result


def replace_ref(ir, old, new):
    if ir["op"] == "ref":
        return dict(ir, binding=new) if ir["binding"] == old else ir
    result = dict(ir)
    for key, value in result.items():
        if isinstance(value, dict) and "op" in value:
            result[key] = replace_ref(value, old, new)
        elif isinstance(value, list):
            result[key] = [replace_ref(v, old, new) if isinstance(v, dict) and "op" in v else v for v in value]
    return result


def substitute(t, mapping):
    if variable(t):
        return mapping.get(t, t)
    if isinstance(t, tuple):
        if t and t[0] == "fn":
            effects = []
            for e in t[3]:
                if variable(e) and e in mapping:
                    effects.extend(mapping[e])
                else:
                    effects.append(substitute(e, mapping))
            return ("fn", tuple(substitute(a, mapping) for a in t[1]), substitute(t[2], mapping), tuple(effects))
        return tuple(substitute(a, mapping) for a in t)
    return t


class Engine:
    def __init__(self, source, file=None, project_root=None):
        self.solver = Solver()
        self.modules = {}
        self.source_texts = {}
        self.loading = []
        self.compiling = []
        self.alias_stack = []
        self.bindings = {}
        self.definitions = {}
        self.initializers = []
        self.nodes = []
        self.alias_views = []
        self.type_table = []
        self.type_ids = {}
        self.type_order = []
        self.next_binding = 0
        self.total_source = 0
        self.file = Path(file).resolve() if file else None
        self.root = Path(project_root).resolve() if project_root else (self.file.parent if self.file else None)
        try:
            self.entry = self.load(self.file, source)
        except Diagnostic as error:
            error.sources = self.source_reports()
            raise

    def source_reports(self):
        entry = self.display_path(self.file)
        return [{"name": name, "source": source, "entry": name == entry}
                for name, source in self.source_texts.items()]

    def binding(self, name, scheme, form, infer=False, authority=True, pinned=None):
        b = Binding(self.next_binding, name, scheme, infer, authority, set(pinned or ()), form)
        self.next_binding += 1
        self.bindings[b.id] = b
        return b

    def name(self, form):
        if not form.atom() or form.value in {"_", "true", "false", "unit"} or "." in form.value:
            raise Diagnostic(form, "binding", "Expected a non-reserved local name without '.'.")
        return form.value

    def display_path(self, path):
        if not path:
            return "<buffer>"
        try:
            return path.relative_to(self.root).as_posix()
        except ValueError:
            return "<outside-root>"

    def module_bytes(self, path):
        """Open canonical in-root components without following replacement links."""
        parts = path.relative_to(self.root).parts
        if not parts:
            raise OSError("A module must be a regular file")
        directory_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
        fd = os.open(self.root, directory_flags)
        try:
            for part in parts[:-1]:
                child = os.open(part, directory_flags, dir_fd=fd)
                os.close(fd)
                fd = child
            flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
            with os.fdopen(os.open(parts[-1], flags, dir_fd=fd), "rb") as stream:
                if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                    raise OSError("A module must be a regular file")
                return stream.read(MAX_SOURCE + 1)
        finally:
            os.close(fd)

    def load(self, path, source=None, origin=None):
        key = str(path) if path else "<buffer>"
        if path and self.root:
            try:
                path.relative_to(self.root)
            except ValueError:
                raise Diagnostic(origin or Form("", {"line": 1, "column": 1, "endLine": 1, "endColumn": 1}, "<entry>"),
                                 "project-root", "Source file escapes the designated project root.")
        if key in self.loading:
            raise Diagnostic(origin, "import-cycle", "Import cycle: " + " -> ".join(self.display_path(Path(p)) for p in self.loading + [key]))
        if key in self.modules:
            return self.modules[key]
        if source is None:
            try:
                raw = self.module_bytes(path)
                if len(raw) > MAX_SOURCE:
                    raise Diagnostic(origin, "source-limit", "Imported source exceeds 256 KiB.")
                source = raw.decode("utf-8")
            except (OSError, UnicodeError):
                raise Diagnostic(origin, "import-read", "Cannot read the imported UTF-8 source file.") from None
        name = self.display_path(path)
        self.source_texts[name] = source
        root = parse(source, name)
        self.total_source += len(source.encode("utf-8"))
        if self.total_source > 4 * 1024 * 1024 or len(self.modules) >= 128:
            raise Diagnostic(root, "source-limit", "Module graph exceeds 4 MiB or 128 files.")
        if not (root.head("script") or root.head("module")):
            raise Diagnostic(root, "script", "Expected (script ...) or (module name ...).")
        module = Module(name, path, source, root)
        self.modules[key] = module
        self.loading.append(key)
        try:
            children = root.value[1:]
            if root.head("module"):
                if not children:
                    raise Diagnostic(root, "module", "A module requires a name.")
                self.name(children[0])
                children = children[1:]
            for i, child in enumerate(children):
                if child.head("import"):
                    if len(child.value) != 3 or not child.value[2].quoted:
                        raise Diagnostic(child, "import", 'Expected (import alias "./file.esh").')
                    alias = self.name(child.value[1])
                    if alias in module.imports or alias in module.definitions or alias in module.aliases or alias in module.caps:
                        raise Diagnostic(child, "duplicate-name", "Duplicate module name or import alias.")
                    if not path:
                        raise Diagnostic(child, "import-context", "Local imports require an entry-file location.")
                    target_text = child.value[2].value
                    if not target_text or Path(target_text).is_absolute() or "\x00" in target_text:
                        raise Diagnostic(child, "import-path", "Import paths must be relative local paths.")
                    try:
                        target = (path.parent / target_text).resolve()
                    except (OSError, RuntimeError, ValueError):
                        raise Diagnostic(child, "import-path", "Cannot resolve the relative import path.") from None
                    imported = self.load(target, origin=child)
                    if not imported.form.head("module"):
                        raise Diagnostic(child, "import-module", "Only definition-only modules can be imported.")
                    module.imports[alias] = imported
                elif child.head("def") or child.head("type"):
                    if len(child.value) < 3:
                        raise Diagnostic(child, "declaration", "Incomplete definition.")
                    symbol = self.name(child.value[1])
                    if symbol in module.definitions or symbol in module.aliases or symbol in module.imports or symbol in module.caps:
                        raise Diagnostic(child, "duplicate-name", f"Duplicate declaration: {symbol}.")
                    target = module.definitions if child.head("def") else module.aliases
                    target[symbol] = child
                    if any(f.head("export") for f in child.value[2:]):
                        module.exports.add(symbol)
                elif child.head("cap") and root.head("script"):
                    if len(child.value) not in (3, 4):
                        raise Diagnostic(child, "capability", "Expected (cap name (rights ...) [infer]).")
                    symbol = self.name(child.value[1])
                    if symbol in module.caps or symbol in module.definitions or symbol in module.aliases or symbol in module.imports:
                        raise Diagnostic(child, "capability", "Duplicate capability or declaration.")
                    ps = self.parse_rights(child.value[2])
                    if len(child.value) == 4 and not child.value[3].atom("infer"):
                        raise Diagnostic(child, "capability", "Only infer may follow capability permissions.")
                    b = self.binding(symbol, Scheme(("cap", symbol, ps)), child, len(child.value) == 4)
                    module.caps[symbol] = b
                elif root.head("script") and (child.head("returns") or child.head("effects")):
                    key = child.value[0].value
                    if key in module.headers:
                        raise Diagnostic(child, "annotation", f"Duplicate {key} annotation.")
                    module.headers[key] = child
                elif root.head("script") and i == len(children) - 1:
                    module.body = child
                else:
                    raise Diagnostic(child, "declaration", "Modules contain imports, type aliases, and function definitions; scripts finish with one body.")
            if root.head("script") and module.body is None:
                raise Diagnostic(root, "script", "A script requires an executable body.")
        finally:
            self.loading.pop()
        return module

    def parse_rights(self, form):
        if not isinstance(form.value, list) or any(not x.atom() or x.value not in RIGHTS for x in form.value):
            raise Diagnostic(form, "permission", "Expected read, write, mkdir, or remove permissions.")
        ps = [x.value for x in form.value]
        if len(ps) != len(set(ps)):
            raise Diagnostic(form, "permission", "Duplicate permission.")
        return tuple(p for p in RIGHTS if p in ps)

    def quantifiers(self, f, rigid=True):
        result = {}
        if f is None:
            return result
        for q in f.value[1:]:
            if not isinstance(q.value, list) or len(q.value) != 2:
                raise Diagnostic(q, "forall", "Expected (name Type|Region|Effect).")
            name = self.name(q.value[0])
            kind = q.value[1].value
            if kind not in ("Type", "Region", "Effect") or name in result:
                raise Diagnostic(q, "forall", "Quantifier names must be unique and kinds must be Type, Region, or Effect.")
            result[name] = self.solver.fresh({"Type": "var", "Region": "rvar", "Effect": "evar"}[kind], name, rigid)
        return result

    def region_type(self, form, module, qs):
        if form.atom() and form.value in qs:
            v = qs[form.value]
            if v[0] == "rvar":
                return v
        elif form.atom() and form.value in module.caps:
            return form.value
        raise Diagnostic(form, "region", "Expected a declared Region parameter or script capability region.")

    def effect_type(self, form, module, qs):
        result = []
        if not form.head("effects"):
            raise Diagnostic(form, "effects", "Expected (effects ...).")
        for e in form.value[1:]:
            if e.atom() and e.value in qs and qs[e.value][0] == "evar":
                result.append(qs[e.value])
            elif e.head() and len(e.value) == 2 and e.value[0].value.startswith("fs.") and e.value[0].value[3:] in RIGHTS:
                result.append(("fs", self.region_type(e.value[1], module, qs), e.value[0].value[3:]))
            else:
                raise Diagnostic(e, "effects", "Expected an Effect parameter or (fs.permission Region).")
        return self.solver.effects(tuple(result))

    def type(self, form, module, qs):
        if form.atom():
            if form.value in ("Unit", "Bool", "Text", "RelPath"):
                return form.value
            if form.value in qs:
                if qs[form.value][0] != "var":
                    raise Diagnostic(form, "kind", "This parameter is not a value type.")
                return qs[form.value]
            return self.alias_type(form.value, [], form, module, qs)
        if not form.head():
            raise Diagnostic(form, "unsupported-type", "Expected a type.")
        head, xs = form.value[0].value, form.value[1:]
        if head == "List" and len(xs) == 1:
            return "list", self.type(xs[0], module, qs)
        if head == "Result" and len(xs) == 2 and xs[1].atom("FsError"):
            return "result", self.type(xs[0], module, qs)
        if head == "Cap" and len(xs) == 2:
            return "cap", self.region_type(xs[0], module, qs), self.parse_rights(xs[1])
        if head == "Fn" and len(xs) == 3 and isinstance(xs[0].value, list):
            return ("fn", tuple(self.type(t, module, qs) for t in xs[0].value),
                    self.type(xs[1], module, qs), self.effect_type(xs[2], module, qs))
        return self.alias_type(head, xs, form, module, qs)

    def target(self, name, module, form):
        if "." not in name:
            return module, name
        prefix, name = name.split(".", 1)
        if prefix not in module.imports:
            raise Diagnostic(form, "unknown-module", f"Unknown import alias: {prefix}.")
        target = module.imports[prefix]
        if name not in target.exports:
            raise Diagnostic(form, "private-name", f"The imported module does not export {name}.")
        return target, name

    def alias_type(self, name, args, form, module, qs):
        target, name = self.target(name, module, form)
        if name not in target.aliases:
            raise Diagnostic(form, "unsupported-type", f"Unknown type: {name}.")
        key = target.name, name
        if key in self.alias_stack:
            raise Diagnostic(form, "type-cycle", "Recursive type aliases are outside this stage.")
        declaration = target.aliases[name]
        fs = fields(declaration, {"export", "forall", "alias"}, 2)
        if "alias" not in fs:
            raise Diagnostic(declaration, "type-alias", "A type declaration requires an alias field.")
        aq = self.quantifiers(fs.get("forall"))
        if len(aq) != len(args):
            raise Diagnostic(form, "type-arity", "Wrong number of type-alias arguments.")
        actual = {}
        for (key_name, var), arg in zip(aq.items(), args):
            if var[0] == "var":
                actual[key_name] = self.type(arg, module, qs)
            elif var[0] == "rvar":
                actual[key_name] = self.region_type(arg, module, qs)
            else:
                actual[key_name] = self.effect_type(arg, module, qs)
        self.alias_stack.append(key)
        try:
            body = self.type(one(fs["alias"], "alias"), target, aq)
            mapping = {aq[n]: actual[n] for n in aq}
            result = substitute(body, mapping)
            self.alias_views.append({"name": name, "source": form.source, "span": form.span, "type": result})
            return result
        finally:
            self.alias_stack.pop()

    def authority(self, t):
        t = self.solver.resolve(t)
        if variable(t):
            return True
        if isinstance(t, str):
            return False
        if t[0] in ("cap", "fn"):
            return True
        return self.authority(t[1])

    def generalize(self, term, env, explicit=()):
        t = self.solver.resolve(term.type)
        pinned = set(term.pinned)
        for b in env.values():
            pinned |= free(self.solver.resolve(b.scheme.type)) - set(b.scheme.quant)
            pinned |= b.pinned
        variables = free(t) - pinned
        quant = set(explicit)
        if not self.solver.effects(term.effects):
            quant |= {v for v in variables if v not in self.solver.rigid and v[0] != "pvar"}
        self.solver.rigid |= quant
        return Scheme(t, tuple(sorted(quant)))

    def instantiate(self, binding):
        mapping = {}
        for v in binding.scheme.quant:
            fresh = self.solver.fresh(v[0])
            mapping[v] = (fresh,) if v[0] == "evar" else fresh
        t = substitute(binding.scheme.type, mapping)
        ir = {"op": "ref", "binding": binding.id}
        if mapping:
            ir = {"op": "inst", "value": ir, "mapping": mapping}
        return Term(t, (), ir, binding.authority, set(binding.pinned))

    def record(self, form, kind, term, details=None):
        self.nodes.append({"source": form.source, "span": form.span, "kind": kind,
                           "type": term.type, "effects": term.effects, "details": details or {}})
        term.ir.setdefault("location", {"source": form.source, "span": form.span, "kind": kind})
        return term

    def builtin(self, name, form):
        key = "<builtin>", name
        if key in self.definitions:
            return self.definitions[key]
        q = ()
        if name.startswith("text."):
            types = ("Text", "Text")
            ret = "Text" if name == "text.concat" else "Bool"
            eff = ()
        else:
            r = self.solver.fresh("rvar", "R", True)
            q = (r,)
            right = {"fs.read-text": "read", "fs.list": "read", "fs.write-text": "write",
                     "fs.mkdir": "mkdir", "fs.remove": "remove"}[name]
            types = (("cap", r, (right,)), "RelPath") + (("Text",) if right == "write" else ())
            ret = ("result", "Text" if name == "fs.read-text" else ("list", "Text") if name == "fs.list" else "Unit")
            eff = (("fs", r, right),)
        params = [self.binding("arg" + str(i), Scheme(t), form, authority=self.authority(t)) for i, t in enumerate(types)]
        rs = [{"op": "ref", "binding": b.id} for b in params]
        if name.startswith("text."):
            body = {"op": "concat" if name == "text.concat" else "eq", "left": rs[0], "right": rs[1]}
        else:
            body = {"op": {"fs.read-text": "read", "fs.write-text": "write"}.get(name, name[3:]), "cap": rs[0], "path": rs[1]}
            if name == "fs.write-text":
                body["text"] = rs[2]
        t = ("fn", types, ret, eff)
        ir = {"op": "fn", "params": [b.id for b in params], "paramTypes": types,
              "captureIds": [], "captureTypes": [], "captures": [], "result": ret,
              "effects": eff, "body": body}
        binding = self.binding(name, Scheme(t, q), form, authority=False)
        self.definitions[key] = binding
        self.initializers.append((binding, ir))
        return binding

    def lookup(self, name, module, env, form):
        if name in env:
            return env[name]
        if name in BUILTINS:
            return self.builtin(name, form)
        target, name = self.target(name, module, form)
        if name not in target.definitions:
            raise Diagnostic(form, "unknown-variable", f"Unknown variable or function: {name}.")
        return self.compile_def(target, name)

    def compile_def(self, module, name):
        key = module.name, name
        if key in self.definitions:
            return self.definitions[key]
        declaration = module.definitions[name]
        if key in self.compiling:
            raise Diagnostic(declaration, "definition-cycle", "Recursive definition cycles are deferred to the recursion stage.")
        self.compiling.append(key)
        try:
            env = dict(module.caps)
            term, explicit, details = self.function(declaration, module, env, {}, named=True)
            scheme = self.generalize(term, env, explicit)
            b = self.binding(name, scheme, declaration, authority=term.authority, pinned=term.pinned)
            self.definitions[key] = b
            self.initializers.append((b, term.ir))
            details["name"] = name
            details["quantifiers"] = scheme.quant
            details["exported"] = name in module.exports
            self.record(declaration, "definition", term, details)
            return b
        finally:
            self.compiling.pop()

    def cap_requirement(self, t, required, form):
        # Permission metavariables collect all uses of an inferred parameter.
        raw = t
        while variable(raw) and raw in self.solver.sub:
            raw = self.solver.sub[raw]
        if variable(raw) and raw[0] == "var" and raw not in self.solver.rigid:
            r, ps = self.solver.fresh("rvar"), self.solver.fresh("pvar")
            self.solver.rights[ps].update(required)
            self.solver.sub[raw] = ("cap", r, ps)
            return r, tuple(required)
        if isinstance(raw, tuple) and raw[0] == "cap" and variable(raw[2]):
            self.solver.rights[raw[2]].update(required)
            return raw[1], tuple(required)
        t = self.solver.resolve(t)
        if not isinstance(t, tuple) or t[0] != "cap":
            raise Diagnostic(form, "type-mismatch", "Expected a capability value.")
        if not set(required) <= set(t[2]):
            raise Diagnostic(form, "permission-denied", "Capability does not grant " + ", ".join(required) + ".")
        return t[1], t[2]

    def coerce(self, term, wanted, form):
        target = self.solver.resolve(wanted)
        if isinstance(target, tuple) and target[0] == "cap":
            region, granted = self.cap_requirement(term.type, target[2], form)
            self.solver.region(region, target[1], form)
            ir = {"op": "narrow", "value": term.ir, "rights": target[2]}
            if tuple(granted) != tuple(target[2]):
                self.nodes.append({"source": form.source, "span": form.span, "kind": "narrow",
                    "type": wanted, "effects": (), "details": {"granted": granted, "required": target[2], "narrowed": True}})
            return Term(wanted, term.effects, ir, True, term.pinned)
        actual = self.solver.resolve(term.type)
        if isinstance(actual, tuple) and actual[0] == "fn" and isinstance(target, tuple) and target[0] == "fn":
            if len(actual[1]) != len(target[1]):
                raise Diagnostic(form, "arity", "Function arities do not match.")
            for a, b in zip(actual[1], target[1]):
                self.solver.unify(a, b, form)
            self.solver.unify(actual[2], target[2], form)
            self.solver.include(actual[3], target[3], form)
            return Term(wanted, term.effects, {"op": "widen", "value": term.ir, "effects": target[3]}, term.authority, term.pinned)
        self.solver.unify(term.type, wanted, form)
        return Term(wanted, term.effects, term.ir, term.authority, term.pinned)

    def value_form(self, form):
        if isinstance(form.value, str) or form.head("fn"):
            return
        raise Diagnostic(form, "evaluation-order", "Bind this computation with let/let-ok before using it as a callee or argument, including pure calls.")

    def function(self, form, module, env, outer_qs, named=False):
        fs = fields(form, {"export", "forall", "params", "returns", "effects", "body", "captures"}, 2 if named else 1)
        for required in ("params", "body"):
            if required not in fs:
                raise Diagnostic(form, "function", f"A function requires {required}.")
        if not named and ("export" in fs or "forall" in fs):
            raise Diagnostic(form, "rank-1", "Anonymous functions infer rank-1 schemes at their binding; export/forall belong to named declarations.")
        qs = dict(outer_qs)
        explicit = self.quantifiers(fs.get("forall"))
        qs.update(explicit)
        params, scope, names = [], dict(env), set()
        for p in fs["params"].value[1:]:
            if p.atom():
                name, ty, opt = self.name(p), self.solver.fresh(), False
                if "export" in fs:
                    raise Diagnostic(p, "public-signature", "Every exported parameter requires a type.")
            elif isinstance(p.value, list) and len(p.value) in (2, 3):
                name, ty = self.name(p.value[0]), self.type(p.value[1], module, qs)
                opt = len(p.value) == 3
                if opt and (not p.value[2].atom("infer") or not (isinstance(ty, tuple) and ty[0] == "cap")):
                    raise Diagnostic(p, "capability", "infer marks an annotated capability parameter.")
            else:
                raise Diagnostic(p, "parameter", "Expected name or (name Type [infer]).")
            if name in names:
                raise Diagnostic(p, "duplicate-name", "Duplicate parameter.")
            names.add(name)
            b = self.binding(name, Scheme(ty), p, opt, self.authority(ty))
            params.append(b)
            scope[name] = b
        if "export" in fs and not {"returns", "effects"} <= fs.keys():
            raise Diagnostic(form, "public-signature", "Exported functions require returns and effects annotations.")
        body = self.expression(one(fs["body"], "body"), module, scope, qs)
        if "returns" in fs:
            body = self.coerce(body, self.type(one(fs["returns"], "returns"), module, qs), fs["returns"])
        bound = body.effects
        if "effects" in fs:
            bound = self.effect_type(fs["effects"], module, qs)
            self.solver.include(body.effects, bound, fs["effects"])
        used = refs(body.ir) - {b.id for b in params}
        captured = [self.bindings[i] for i in sorted(used)]
        authority = [b for b in captured if b.authority and self.authority(b.scheme.type)]
        declaration = fs.get("captures")
        inferred = declaration is not None and len(declaration.value) == 2 and declaration.value[1].atom("_")
        declared = set()
        if declaration and not inferred:
            for f in declaration.value[1:]:
                if not f.atom() or f.atom("_"):
                    raise Diagnostic(f, "captures", "Use a list of names or the single marker (captures _).")
                b = self.lookup(f.value, module, env, f)
                if b.id in declared:
                    raise Diagnostic(f, "captures", "Duplicate capture declaration.")
                declared.add(b.id)
        missing = [b.name for b in authority if b.id not in declared]
        if missing and not inferred:
            raise Diagnostic(form, "capture-required", "Declare authority captures: " + ", ".join(missing) + "; or use (captures _).")
        capture_values, capture_ids, capture_types, retained = [], [], [], []
        pinned = set()
        for b in captured:
            t = self.solver.resolve(b.scheme.type)
            value = {"op": "ref", "binding": b.id}
            cid = b.id
            note = {"name": b.name, "authority": b in authority, "inferred": inferred and b in authority}
            if isinstance(t, tuple) and t[0] == "cap":
                needed = self.capture_rights(body.ir, b.id, t[2])
                required = tuple(p for p in RIGHTS if p in needed)
                note.update({"region": t[1], "granted": t[2], "required": required, "narrowed": required != t[2]})
                if required != t[2]:
                    t = ("cap", t[1], required)
                    narrowed = self.binding(b.name, Scheme(t), b.form, b.infer, True, b.pinned)
                    cid = narrowed.id
                    body.ir = replace_ref(body.ir, b.id, cid)
                    value = {"op": "narrow", "value": value, "rights": required}
            elif b in authority:
                note["retainedAuthority"] = "potential; not established by the invocation effect bound"
            capture_values.append(value)
            capture_ids.append(cid)
            capture_types.append(t)
            retained.append(note)
            pinned |= (free(t) - set(b.scheme.quant)) | b.pinned
        ty = ("fn", tuple(b.scheme.type for b in params), body.type, bound)
        ir = {"op": "fn", "params": [b.id for b in params], "paramTypes": ty[1], "result": body.type,
              "effects": bound, "captureIds": capture_ids, "captureTypes": capture_types,
              "captures": capture_values, "body": body.ir}
        details = {"captures": retained, "captureInference": inferred,
                   "invocationEffects": bound, "creationEffects": [], "arity": len(params)}
        return Term(ty, (), ir, bool(authority), pinned), tuple(explicit.values()), details

    def capture_rights(self, ir, binding, all_rights):
        if ir["op"] == "narrow" and ir["value"]["op"] == "ref" and ir["value"]["binding"] == binding:
            return set(ir["rights"])
        if ir["op"] == "ref":
            return set(all_rights) if ir["binding"] == binding else set()
        result = set()
        values = ir["captures"] if ir["op"] == "fn" else ir.values()
        for v in values:
            if isinstance(v, dict) and "op" in v:
                result |= self.capture_rights(v, binding, all_rights)
            elif isinstance(v, list):
                for child in v:
                    if isinstance(child, dict) and "op" in child:
                        result |= self.capture_rights(child, binding, all_rights)
        return result

    def expression(self, form, module, env, qs):
        if form.quoted:
            return self.record(form, "literal", Term("Text", (), {"op": "text", "value": form.value}))
        if form.atom():
            if form.value in ("true", "false"):
                return self.record(form, "literal", Term("Bool", (), {"op": "bool", "value": form.value == "true"}))
            if form.value == "unit":
                return self.record(form, "literal", Term("Unit", (), {"op": "unit"}))
            b = self.lookup(form.value, module, env, form)
            return self.record(form, "variable", self.instantiate(b), {"name": form.value})
        if not form.value:
            raise Diagnostic(form, "expression", "Empty application.")
        xs = form.value
        if xs[0].atom() and xs[0].value in {"loop", "lambda", "proc.run", "parallel", "spawn", "await"}:
            raise Diagnostic(form, "unsupported-form", "This form is outside the current implementation stage.")
        if form.head("fn"):
            term, _, details = self.function(form, module, env, qs)
            return self.record(form, "fn", term, details)
        if form.head("do"):
            if len(xs) == 1:
                raise Diagnostic(form, "do", "A do block needs a final expression.")
            return self.block(xs[1:], module, env, qs)
        if form.head("let") or form.head("let-ok"):
            if len(xs) != 4:
                raise Diagnostic(form, "binding", "Use (let name value body), or a binding step inside do.")
            return self.let_expr(xs[0].value, xs[1], xs[2], [xs[3]], form, module, env, qs)
        if form.head("if"):
            if len(xs) != 4:
                raise Diagnostic(form, "arity", "if requires a condition and two branches.")
            self.value_form(xs[1])
            for branch in xs[2:]:
                if isinstance(branch.value, list) and not any(branch.head(k) for k in ("fn", "do", "let", "let-ok")):
                    raise Diagnostic(branch, "evaluation-order", "Put branch computations inside their own do block.")
            c = self.coerce(self.expression(xs[1], module, env, qs), "Bool", xs[1])
            a = self.expression(xs[2], module, env, qs)
            b = self.expression(xs[3], module, env, qs)
            at, bt = self.solver.resolve(a.type), self.solver.resolve(b.type)
            if isinstance(at, tuple) and isinstance(bt, tuple) and at[0] == bt[0] == "fn" and len(at[1]) == len(bt[1]):
                target = ("fn", at[1], at[2], self.solver.effects(at[3] + bt[3]))
                a, b = self.coerce(a, target, xs[2]), self.coerce(b, target, xs[3])
            elif isinstance(at, tuple) and isinstance(bt, tuple) and at[0] == bt[0] == "cap":
                target = ("cap", at[1], tuple(p for p in RIGHTS if p in at[2] and p in bt[2]))
                a, b = self.coerce(a, target, xs[2]), self.coerce(b, target, xs[3])
            else:
                b = self.coerce(b, a.type, xs[3])
            return self.record(form, "if", Term(a.type, c.effects + a.effects + b.effects,
                {"op": "if", "condition": c.ir, "yes": a.ir, "no": b.ir}, a.authority or b.authority, a.pinned | b.pinned),
                {"branches": "alternatives; only the selected branch executes"})
        if form.head("path"):
            if len(xs) != 2 or not xs[1].quoted:
                raise Diagnostic(form, "path", "path requires a literal relative path.")
            text = xs[1].value
            parts = [] if text == "." else text.split("/")
            if any(not x or x in (".", "..") or "\\" in x or any(ord(c) < 32 or ord(c) == 127 for c in x) for x in parts):
                raise Diagnostic(form, "path", "Expected a relative path without empty, '.', '..', backslash, or control components.")
            return self.record(form, "path", Term("RelPath", (), {"op": "path", "value": text}))
        if form.head("ok"):
            if len(xs) != 2:
                raise Diagnostic(form, "arity", "ok requires one value.")
            self.value_form(xs[1])
            v = self.expression(xs[1], module, env, qs)
            return self.record(form, "ok", Term(("result", v.type), v.effects, {"op": "ok", "value": v.ir}, v.authority, v.pinned))
        if form.head("error"):
            if len(xs) != 3 or not xs[2].quoted:
                raise Diagnostic(form, "error", 'Expected (error Type "message").')
            t = self.type(xs[1], module, qs)
            return self.record(form, "error", Term(("result", t), (), {"op": "error", "type": t, "message": xs[2].value}))
        return self.call(form, module, env, qs)

    def block(self, forms, module, env, qs):
        if len(forms) == 1:
            return self.expression(forms[0], module, env, qs)
        step = forms[0]
        if not (step.head("let") or step.head("let-ok")) or len(step.value) != 3:
            raise Diagnostic(step, "evaluation-order", "Every non-final computation in do needs a let or let-ok binding.")
        return self.let_expr(step.value[0].value, step.value[1], step.value[2], forms[1:], step, module, env, qs)

    def let_expr(self, kind, binder, value, rest, form, module, env, qs):
        name = self.name(binder)
        if name in module.caps:
            raise Diagnostic(binder, "binding", "A local binding cannot shadow a script capability.")
        v = self.expression(value, module, env, qs)
        if kind == "let-ok":
            ty = self.solver.resolve(v.type)
            if variable(ty) and ty[0] == "var" and ty not in self.solver.rigid:
                self.solver.bind(ty, ("result", self.solver.fresh()), value)
                ty = self.solver.resolve(ty)
            if not isinstance(ty, tuple) or ty[0] != "result":
                raise Diagnostic(value, "type-mismatch", "let-ok requires Result.")
            bound_value = Term(ty[1], v.effects, v.ir, v.authority, v.pinned)
        else:
            bound_value = v
        scheme = self.generalize(bound_value, env)
        b = self.binding(name, scheme, binder, authority=bound_value.authority, pinned=bound_value.pinned)
        scope = dict(env, **{name: b})
        body = self.block(rest, module, scope, qs)
        if kind == "let-ok":
            bt = self.solver.resolve(body.type)
            if variable(bt) and bt[0] == "var" and bt not in self.solver.rigid:
                self.solver.bind(bt, ("result", self.solver.fresh()), form)
                bt = self.solver.resolve(bt)
            if not isinstance(bt, tuple) or bt[0] != "result":
                raise Diagnostic(form, "type-mismatch", "The remainder of a let-ok block must return Result.")
        term = Term(body.type, v.effects + body.effects, {"op": kind, "binding": b.id, "value": v.ir, "body": body.ir}, body.authority, body.pinned)
        details = {"name": name, "quantifiers": scheme.quant, "evaluation": "once, before the remaining block"}
        if kind == "let-ok":
            details["onError"] = "return Err from this block; skip its remaining steps"
        details["blockResult"] = body.type
        result = self.record(form, kind, term, details)
        self.nodes[-1]["type"] = scheme.type
        self.nodes[-1]["effects"] = v.effects
        return result

    def call(self, form, module, env, qs):
        callee, arg_forms = form.value[0], form.value[1:]
        self.value_form(callee)
        f = self.expression(callee, module, env, qs)
        t = self.solver.resolve(f.type)
        if variable(t) and t[0] == "var" and t not in self.solver.rigid:
            signature = ("fn", tuple(self.solver.fresh() for _ in arg_forms), self.solver.fresh(), (self.solver.fresh("evar"),))
            self.solver.bind(t, signature, callee)
            t = signature
        if not isinstance(t, tuple) or t[0] != "fn":
            raise Diagnostic(callee, "not-callable", "The callee is not a function.")
        if len(t[1]) != len(arg_forms):
            raise Diagnostic(form, "arity", "Supply every argument; use an explicit fn for partial application.")
        args = [None] * len(arg_forms)
        original_types = [None] * len(arg_forms)
        resolutions = []
        # Non-marker arguments establish region/type constraints before candidate selection.
        for i, (source, expected) in enumerate(zip(arg_forms, t[1])):
            if source.atom("_"):
                continue
            self.value_form(source)
            arg = self.expression(source, module, env, qs)
            original_types[i] = arg.type
            args[i] = self.coerce(arg, expected, source)
        for i, (source, expected) in enumerate(zip(arg_forms, t[1])):
            if args[i] is not None:
                continue
            expected = self.solver.resolve(expected)
            if not isinstance(expected, tuple) or expected[0] != "cap":
                raise Diagnostic(source, "capability", "_ requires a known capability parameter; add an annotation.")
            candidates = []
            for b in env.values():
                bt = self.solver.resolve(b.scheme.type)
                if not b.infer or not isinstance(bt, tuple) or bt[0] != "cap" or not set(expected[2]) <= set(bt[2]):
                    continue
                region = self.solver.resolve(expected[1])
                if variable(region) and region not in self.solver.rigid or region == self.solver.resolve(bt[1]):
                    candidates.append(b)
            if len(candidates) != 1:
                code = "capability-missing" if not candidates else "capability-ambiguous"
                raise Diagnostic(source, code, "Capability inference requires exactly one eligible opt-in candidate" +
                                 (": " + ", ".join(b.name for b in candidates) if candidates else "."))
            b = candidates[0]
            original_types[i] = b.scheme.type
            args[i] = self.coerce(self.instantiate(b), expected, source)
            resolutions.append({"argument": i + 1, "name": b.name, "region": expected[1], "inferred": True})
        effects = f.effects + tuple(e for a in args for e in a.effects) + t[3]
        ret = self.solver.resolve(t[2])
        pinned = set().union(*(a.pinned for a in args), f.pinned)
        term = Term(ret, effects, {"op": "call", "function": f.ir, "args": [a.ir for a in args],
                                  "reviewEffects": t[3]}, self.authority(ret), pinned)
        name = callee.value if callee.atom() else "anonymous function"
        details = {"callee": name, "arguments": resolutions, "invocations": 1,
                   "internalOrder": "not specified by the permission contract", "internalCount": "unknown from the permission contract"}
        if isinstance(name, str) and name.startswith("fs."):
            cap_type = self.solver.resolve(args[0].type)
            original_type = self.solver.resolve(original_types[0])
            details.update({"region": cap_type[1], "inferred": arg_forms[0].atom("_"),
                            "granted": original_type[2], "required": cap_type[2], "narrowed": original_type[2] != cap_type[2]})
        return self.record(form, name if name in BUILTINS else "call", term, details)

    def region_json(self, r):
        r = self.solver.resolve(r)
        return r[1] if variable(r) else r

    def bound_json(self, es):
        return [{"var": e[1]} if variable(e) else
                {"region": self.region_json(e[1]), "permission": e[2]} for e in self.solver.effects(es)]

    def type_json(self, t):
        t = self.solver.resolve(t)
        if t not in self.type_ids:
            encoded = self._type_json(t)
            self.type_ids[t] = len(self.type_table)
            self.type_table.append(encoded)
            self.type_order.append(t)
        return {"ref": self.type_ids[t]}

    def _type_json(self, t):
        t = self.solver.resolve(t)
        if isinstance(t, str):
            return t
        if t[0] == "var":
            return {"tag": "var", "id": t[1]}
        if t[0] == "cap":
            return {"tag": "cap", "region": self.region_json(t[1]), "rights": list(t[2])}
        if t[0] == "fn":
            return {"tag": "fn", "args": [self.type_json(a) for a in t[1]],
                    "result": self.type_json(t[2]), "effects": self.bound_json(t[3])}
        return {"tag": t[0], "element" if t[0] == "list" else "success": self.type_json(t[1])}

    def subst_json(self, mapping):
        result = {"types": [], "regions": [], "effects": []}
        for v, replacement in mapping.items():
            if v[0] == "var":
                result["types"].append([v[1], self.type_json(replacement)])
            elif v[0] == "rvar":
                result["regions"].append([v[1], self.region_json(replacement)])
            else:
                result["effects"].append([v[1], self.bound_json(replacement)])
        return result

    def lower(self, ir, context):
        op = ir["op"]
        if op == "ref":
            return {"op": "var", "index": context.index(ir["binding"])}
        if op in ("let", "let-ok"):
            return {"op": op, "value": self.lower(ir["value"], context),
                    "body": self.lower(ir["body"], [ir["binding"]] + context)}
        if op == "fn":
            return {"op": "fn", "args": [self.type_json(t) for t in ir["paramTypes"]],
                    "result": self.type_json(ir["result"]), "effects": self.bound_json(ir["effects"]),
                    "captureTypes": [self.type_json(t) for t in ir["captureTypes"]],
                    "captures": [self.lower(v, context) for v in ir["captures"]],
                    "body": self.lower(ir["body"], ir["params"] + ir["captureIds"])}
        if op == "inst":
            return {"op": "inst", "value": self.lower(ir["value"], context), "subst": self.subst_json(ir["mapping"])}
        result = {"op": op}
        for key, value in ir.items():
            if key in ("op", "location", "reviewEffects"):
                continue
            if key == "type":
                result[key] = self.type_json(value)
            elif key == "effects":
                result[key] = self.bound_json(value)
            elif isinstance(value, dict) and "op" in value:
                result[key] = self.lower(value, context)
            elif isinstance(value, list):
                result[key] = [self.lower(v, context) if isinstance(v, dict) and "op" in v else v for v in value]
            else:
                result[key] = value
        return result

    def effects_report(self, es):
        return [{"operation": self.solver.names[e], "region": "polymorphic", "variable": True} if variable(e) else
                {"operation": "fs." + e[2], "region": self.solver.show(e[1])} for e in self.solver.effects(es)]

    def details_report(self, details):
        result = {}
        for k, v in details.items():
            if k in ("invocationEffects", "creationEffects"):
                result[k] = self.effects_report(v)
            elif k == "region":
                result[k] = self.solver.show(v)
            elif k == "blockResult":
                result[k] = self.solver.show(v)
            elif k == "quantifiers":
                result[k] = [{"name": self.solver.names[x], "kind": {"var": "Type", "rvar": "Region", "evar": "Effect"}[x[0]]} for x in v]
            elif isinstance(v, list) and v and isinstance(v[0], dict):
                result[k] = [self.details_report(x) for x in v]
            else:
                result[k] = v
        return result

    def structure(self, ir):
        op = ir["op"]
        if op in ("inst", "narrow", "widen"):
            return self.structure(ir["value"])
        result = {"kind": op}
        if "location" in ir:
            result.update(ir["location"])
        if op in ("let", "let-ok"):
            result.update({"binding": self.bindings[ir["binding"]].name,
                           "value": self.structure(ir["value"]), "next": self.structure(ir["body"])})
        elif op == "if":
            result.update({k: self.structure(ir[k]) for k in ("condition", "yes", "no")})
        elif op == "call":
            result["callee"] = self.structure(ir["function"])
            result["arguments"] = [self.structure(a) for a in ir["args"]]
            result["invocations"] = 1
            result["effects"] = self.effects_report(ir["reviewEffects"])
            result["callbackInternals"] = "order/count unknown from permission bound alone" if result["effects"] else "no filesystem requests"
        elif op == "fn":
            result["bodyOnInvocation"] = self.structure(ir["body"])
        elif op == "ref":
            result["name"] = self.bindings[ir["binding"]].name
        return result

    def build(self):
        entry = self.entry
        for module in list(self.modules.values()):
            for name in module.definitions:
                self.compile_def(module, name)
            # Check unused aliases as well, with rigid arguments of the declared kinds.
            for name, declaration in module.aliases.items():
                fs = fields(declaration, {"export", "forall", "alias"}, 2)
                qs = self.quantifiers(fs.get("forall"))
                if "alias" not in fs:
                    raise Diagnostic(declaration, "type-alias", "A type declaration requires alias.")
                self.alias_stack.append((module.name, name))
                try:
                    self.type(one(fs["alias"], "alias"), module, qs)
                finally:
                    self.alias_stack.pop()
        term = self.expression(entry.body, entry, dict(entry.caps), {}) if entry.body else Term("Unit", (), {"op": "unit"})
        if "returns" in entry.headers:
            try:
                term = self.coerce(term, self.type(one(entry.headers["returns"], "returns"), entry, {}), entry.headers["returns"])
            except Diagnostic as error:
                if error.code == "type-mismatch":
                    error.code = "return-type"
                raise
        actual = self.solver.effects(term.effects)
        declared = None
        if "effects" in entry.headers:
            declared = self.effect_type(entry.headers["effects"], entry, {})
            self.solver.include(actual, declared, entry.headers["effects"])
        if free(actual):
            raise Diagnostic(entry.body or entry.form, "effect-annotation-required", "The executable script still has unresolved effect or target-region variables.")
        ir = term.ir
        for binding, value in reversed(self.initializers):
            ir = {"op": "let", "binding": binding.id, "value": value, "body": ir}
        for binding in reversed(list(entry.caps.values())):
            ty = binding.scheme.type
            ir = {"op": "let", "binding": binding.id,
                  "value": {"op": "cap", "region": ty[1], "rights": list(ty[2])}, "body": ir}
        policy = [{"region": b.name, "permission": p} for b in entry.caps.values() for p in b.scheme.type[2]]
        nodes = [{**n, "type": self.solver.show(n["type"]), "effects": self.effects_report(n["effects"]),
                  "details": self.details_report(n["details"]), "typeRef": self.type_json(n["type"])["ref"]} for n in self.nodes]
        report = {"ok": True, "version": "0.2", "source": entry.source, "entry": entry.name,
                  "sources": [{"name": m.name, "source": m.source, "entry": m is entry} for m in self.modules.values()],
                  "type": self.solver.show(term.type), "effects": self.effects_report(actual),
                  "declaredEffects": None if declared is None else self.effects_report(declared),
                  "capabilities": [{"name": b.name, "permissions": b.scheme.type[2], "infer": b.infer, "span": b.form.span} for b in entry.caps.values()],
                  "nodes": nodes, "structure": self.structure(term.ir),
                  "aliases": [{**a, "type": self.solver.show(a["type"]), "typeRef": self.type_json(a["type"])["ref"]} for a in self.alias_views],
                  "scope": "Permission bounds; ordered computations; callback internals require stronger contracts."}
        program = self.lower(ir, [])
        result_type = self.type_json(term.type)
        report["typeGraph"] = [{"display": self.solver.show(t), "shape": shape}
                               for t, shape in zip(self.type_order, self.type_table)]
        return {"policy": policy, "program": program, "type": result_type,
                "types": self.type_table,
                "bound": self.bound_json(actual if declared is None else declared), "report": report}


def elaborate(source, file=None, project_root=None):
    engine = Engine(source, file, project_root)
    try:
        return engine.build()
    except Diagnostic as error:
        error.sources = engine.source_reports()
        raise
