# Prototype syntax

Version 0.2 accepts one `(script ... expression)` or definition-only `(module name ...)` form per file. S-expressions use parentheses, JSON-style quoted strings, and semicolon line comments. Spans use one-based Unicode character columns and an exclusive end position. Source text is retained verbatim for the editor; no formatter rewrites it. See [functions and modules](functions.md) for declarations, polymorphism, aliases, captures, and imports.

## Declarations

```lisp
(cap region-name (read write mkdir remove))
(cap inferred-region (read) infer)
(returns (Result Unit FsError))
(effects (fs.read region-name) (fs.write region-name))
```

Declarations precede the single body expression. Permission subsets are allowed. Capability names and permissions must be unique within their declarations. `infer` registers that capability as a candidate; `_` is a marker at a capability argument position. It is not a general term-synthesis request.

Script `returns` and `effects` declarations are optional. An effect annotation is an upper bound and may contain unused effects. It cannot mention undeclared regions. Exported functions require complete type and effect signatures.

## Expressions

| Expression | Meaning |
| --- | --- |
| `"text"`, `true`, `false`, `unit` | Literals |
| `name` | Lexically scoped immutable binding |
| `(path "relative/name")` | Validated relative path; `"."` denotes the root |
| `(let name value body)` | Evaluate value, then body with a binding |
| `(let-ok name result body)` | Evaluate body only for `Ok`; body must also return `Result` |
| `(do (let name value) ... final)` | Explicit sequential binding steps and a final computation |
| `(if condition yes no)` | Boolean condition and equally typed branches |
| `(text.concat a b)` | Concatenate two texts |
| `(text.eq a b)` | Compare two texts |
| `(ok value)` | Construct a successful `Result` |
| `(error T "message")` | Construct a user error with success type `T` |

`let-ok` is lexical composition, not a hidden jump out of an arbitrary caller. `let` does not implicitly handle or propagate a `Result`. Bindings may shadow earlier local bindings, but cannot use capability names or reserved literal names.

Callees, arguments, and conditional tests are values. Bind intermediate calls,
including pure calls, with `let`/`let-ok`. Branch computations have their own
`do` or lexical-binding blocks. For example, bind `(path "file")` before passing
that path to `fs.read-text`; nested call syntax is rejected with an ordering
diagnostic.

Paths reject absolute paths, empty components, parent components, interior dot components, backslashes, and control characters. Text is not implicitly converted into a path. `(path.parse text)` returns `Result RelPath FsError`; `(path.join base component)` appends one checked component and returns the same Result type. Both are pure but must be sequenced like other calls.

## Filesystem operations

| Expression | Result | Required permission | Effect |
| --- | --- | --- | --- |
| `(fs.read-bytes cap path)` | `Result Bytes FsError` | `read` | `fs.read region` |
| `(fs.write-bytes cap path bytes)` | `Result Unit FsError` | `write` | `fs.write region` |
| `(fs.read-text cap path)` | `Result Text FsError` | `read` | `fs.read region` |
| `(fs.write-text cap path text)` | `Result Unit FsError` | `write` | `fs.write region` |
| `(fs.list cap path)` | `Result (List Text) FsError` | `read` | `fs.read region` |
| `(fs.mkdir cap path)` | `Result Unit FsError` | `mkdir` | `fs.mkdir region` |
| `(fs.remove cap path)` | `Result Unit FsError` | `remove` | `fs.remove region` |

`fs.list` returns sorted immediate entry names. `fs.mkdir` creates one directory and does not create missing parents. `fs.remove` accepts regular files only; recursive deletion and directory removal are not supported.

`fs.write-text` writes a temporary regular file in the destination directory and replaces the target name. Existing targets must be regular files. This creates a new inode, so other hard links keep their contents. New files use mode `0600`; existing ordinary permission bits are copied. Ownership, ACLs, extended attributes, and other metadata are not preserved. Crash durability and multi-operation transactions are not promised.

Reads and traversal reject symbolic links in script paths. The selected grant root is trusted configuration. See [assurance boundaries](assurance.md) for concurrent namespace mutation and OS assumptions.

Errors carry a code and message. Primitive errors return typed `Err` values; they do not change the statically inferred success type. Source checking limits each file to 256 KiB, 50,000 forms, and nesting depth 128; a module graph is limited to 128 files and 4 MiB. These are implementation limits, not claims about the eventual language.

## Bytes and finite collections

`(utf8.encode text)` returns `Bytes`. `(utf8.decode bytes)` returns
`Result Text FsError`, rejecting invalid UTF-8 rather than replacing characters.
`fs.write-bytes` has the same replacement semantics as `fs.write-text`.

A list literal declares its element type: `(list Text "a" "b")` or `(list Text)`.
Elements are values, as for call arguments.

| Operation | Callback | Result |
| --- | --- | --- |
| `(list.map callback items)` | `A → B` with Effect E | `List B` with Effect E |
| `(list.map-ok callback items)` | `A → Result B FsError` with E | `Result (List B) FsError` with E |
| `(list.fold callback initial items)` | `(S, A) → S` with E | `S` with E |
| `(list.fold-ok callback initial items)` | `(S, A) → Result S FsError` with E | `Result S FsError` with E |

Callbacks run in list order. Fold receives the accumulator first. The `-ok`
variants stop at the first error, keeping earlier external changes. Empty lists
invoke no callbacks; folds return the initial accumulator. The permission bound
is a union while the behavior view retains repeated execution (`E*` for unknown
finite lengths). General recursion, infinite loops, and user-written `mu` are
not part of these operations.

## Update specifications

`requires`, `preserves`, and `ensures` belong to the script header. An additional
`(state condition ...)` file can be supplied through `--state` to check, run, or
serve. Conditions require read authority and contribute observation effects.
See [update contracts](update-contracts.md) for the predicate grammar and runtime
phases. Mandatory validation cannot be skipped when its budget is exhausted.
