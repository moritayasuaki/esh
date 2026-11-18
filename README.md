# esh

An experimental typed S-expression scripting language for inspecting and running filesystem automation.

The executable uses a Lean kernel for typed functions and closures, capability values, permission-bearing requests, and effect-bound certificates. A Python frontend infers rank-1 types and loads local modules; the Lean kernel independently rechecks its executable representation. A small Python host connects requests to POSIX filesystem operations. The local editor displays checked types, effects, evaluation order, retained authority, and imported source.

**This is a prototype, not a complete implementation of the language design.** See [assurance boundaries](docs/assurance.md) before relying on the proofs, and [the language design](docs/design.md) for the planned language.

## Build and test

Requirements: the Lean toolchain pinned in `lean-toolchain`, Lake, Python 3.9 or newer, and a POSIX host with descriptor-relative file operations. There are no third-party Python packages or Lean package dependencies.

```sh
lake build
python3 -m unittest discover -s tests -v
```

The default build includes `Esh.Proofs`, so changes to the executable kernel also check its proof entry points. Tests use temporary directories; they include permission errors, ambiguous inference, path traversal, symlinks, hard links, UTF-8 failures, and error propagation.

## Inspect a script

```sh
python3 esh.py check examples/copy.esh
python3 esh.py check examples/copy.esh --json
```

The example contains an explicitly registered inference candidate:

```lisp
(script
  (cap input (read write) infer)
  (cap output (write))
  (returns (Result Unit FsError))
  (effects (fs.read input) (fs.write output))

  (do
    (let source (path "source.txt"))
    (let target (path "copy.txt"))
    (let-ok content (fs.read-text _ source))
    (let decorated (text.concat "Copied by esh\n" content))
    (fs.write-text output target decorated)))
```

The checker resolves `_` to `input`, reports narrowing from `{read, write}` to `{read}`, and checks the declared effect upper bound. If multiple registered capabilities qualify, checking fails. Narrower permissions do not win a tie.

## Run with explicit grants

Capability declarations state requirements; they do not give a script access to files. The host supplies roots and permissions separately.

```sh
mkdir -p .local/demo/input .local/demo/output
printf 'hello\n' > .local/demo/input/source.txt
python3 esh.py run examples/copy.esh \
  --grant input read,write .local/demo/input \
  --grant output write .local/demo/output
```

`let-ok` returns an error without executing its body when the bound operation fails. `write-text` replaces the entire target file. The CLI returns status 0 on success, 1 for a top-level `Err`, and 2 for a source, grant, or host setup error.

The default file-content and directory-name payload limit is 8 MiB, configurable with `--max-bytes`. This is an operation resource limit, not the proposed budget for supplementary expectation checks.

## Review editor

```sh
python3 esh.py serve
# Review functions and imports with a file location:
python3 esh.py serve --file examples/functions.esh
```

Open the local address printed by the command. The editor rechecks source as it changes, links review rows back to source spans, and can download the edited source. Imported files are displayed read-only. Evaluation steps, function contracts, capture inference, narrowing, and transparent aliases are visible. It has no execution endpoint. A displayed effect set covers possible operations across all branches; a callback's permission bound alone does not specify its internal order or operation count.

Imports are relative to their importing file and confined to the entry file's directory by default. Set `--project-root DIR` on `check`, `run`, or `serve` to choose a different boundary. Importing loads definitions without calling their bodies. Each check/run retains the source bytes it checked.

## Functions and modules

```lisp
(module functions
  (def apply
    (export)
    (forall (A Type) (B Type) (E Effect))
    (params (callback (Fn (A) B (effects E))) (value A))
    (returns B)
    (effects E)
    (body (callback value))))
```

Private functions may infer their signatures. Exported functions require complete contracts and are checked even when unused. Closures declare authority captures with `(captures root callback)` or request an expansion with `(captures _)`. Calls provide all arguments; partial application uses an explicit `fn`. Intermediate calls, including pure calls, require `let` or `let-ok`.

See [the module example](examples/functions.esh), its [library](examples/lib/io.esh), and [function and module syntax](docs/functions.md).

## Implemented in this slice

- `Unit`, `Bool`, `Text`, `RelPath`, `List T`, `Result T FsError`, capabilities, and function values.
- Named/anonymous higher-order functions, explicit captures, and rank-1 type/region/effect polymorphism.
- Static local modules with required public contracts, definition-only imports, and acyclic forward references.
- Transparent nonrecursive type aliases and a shared type representation.
- Explicit `do`/`let` sequencing, conditionals, text operations, and `Result` propagation.
- Explicit capability arguments, opt-in inference candidates, ambiguity errors, and narrowing explanations.
- Optional script result/effect annotations, checked against inferred results.
- Text reads and replacement writes, directory listing/creation, and regular-file removal.
- Kernel-checked request effect bounds, authorization, pure traces, and capability restriction laws.
- Abstract file-content store laws, explicitly separate from concrete OS correctness.

Explicit-state loops, general recursion, `div`, restricted effect iteration, the parallel runtime, external commands, and generated runtime contracts are **not implemented yet**. Their accepted design policies remain in force. Inference is conservative: unsupported effect constraints require annotations rather than guessed partitions or unrestricted effects.

See [prototype syntax](docs/prototype.md) and [implementation stages](docs/roadmap.md).
