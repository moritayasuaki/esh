# Prototype syntax

Version 0.1 accepts one `(script ... expression)` form. S-expressions use parentheses, JSON-style quoted strings, and semicolon line comments. Spans use one-based Unicode character columns and an exclusive end position. Source text is retained verbatim for the editor; no formatter rewrites it.

## Declarations

```lisp
(cap region-name (read write mkdir remove))
(cap inferred-region (read) infer)
(returns (Result Unit FsError))
(effects (fs.read region-name) (fs.write region-name))
```

Declarations precede the single body expression. Permission subsets are allowed. Capability names and permissions must be unique within their declarations. `infer` registers that capability as a candidate; `_` is a marker at a capability argument position. It is not a general term-synthesis request.

The `returns` and `effects` declarations are optional for this script-only prototype. An effect annotation is an upper bound and may contain unused effects. It cannot mention undeclared authority. Exported functions and their mandatory signatures are not yet available.

## Expressions

| Expression | Meaning |
| --- | --- |
| `"text"`, `true`, `false`, `unit` | Literals |
| `name` | Lexically scoped immutable binding |
| `(path "relative/name")` | Validated relative path; `"."` denotes the root |
| `(let name value body)` | Evaluate value, then body with a binding |
| `(let-ok name result body)` | Evaluate body only for `Ok`; body must also return `Result` |
| `(if condition yes no)` | Boolean condition and equally typed branches |
| `(text.concat a b)` | Concatenate two texts |
| `(text.eq a b)` | Compare two texts |
| `(ok value)` | Construct a successful `Result` |
| `(error T "message")` | Construct a user error with success type `T` |

`let-ok` is lexical composition, not a hidden jump out of an arbitrary caller. `let` does not implicitly handle or propagate a `Result`. Bindings may shadow earlier local bindings, but cannot use capability names or reserved literal names.

Paths reject absolute paths, empty components, parent components, interior dot components, backslashes, and control characters. Text is not implicitly converted into a path. Dynamic path construction is not yet implemented.

## Filesystem operations

| Expression | Result | Required permission | Effect |
| --- | --- | --- | --- |
| `(fs.read-text cap path)` | `Result Text FsError` | `read` | `fs.read region` |
| `(fs.write-text cap path text)` | `Result Unit FsError` | `write` | `fs.write region` |
| `(fs.list cap path)` | `Result (List Text) FsError` | `read` | `fs.read region` |
| `(fs.mkdir cap path)` | `Result Unit FsError` | `mkdir` | `fs.mkdir region` |
| `(fs.remove cap path)` | `Result Unit FsError` | `remove` | `fs.remove region` |

`fs.list` returns sorted immediate entry names. `fs.mkdir` creates one directory and does not create missing parents. `fs.remove` accepts regular files only; recursive deletion and directory removal are not supported.

`fs.write-text` writes a temporary regular file in the destination directory and replaces the target name. Existing targets must be regular files. This creates a new inode, so other hard links keep their contents. New files use mode `0600`; existing ordinary permission bits are copied. Ownership, ACLs, extended attributes, and other metadata are not preserved. Crash durability and multi-operation transactions are not promised.

Reads and traversal reject symbolic links in script paths. The selected grant root is trusted configuration. See [assurance boundaries](assurance.md) for concurrent namespace mutation and OS assumptions.

Errors carry a code and message. Primitive errors return typed `Err` values; they do not change the statically inferred success type. Source checking limits input to 256 KiB, 50,000 tokens, and nesting depth 128. These are implementation limits, not claims about the eventual language.
