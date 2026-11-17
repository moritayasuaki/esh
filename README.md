# esh

An experimental typed S-expression scripting language for inspecting and running filesystem automation.

The first executable slice uses a Lean kernel for typed expressions, permission-bearing requests, effect-bound certificates, and execution. A small Python host connects those requests to POSIX filesystem operations. A local editor displays the same checker's types, effects, inferred arguments, and permission narrowing.

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

  (let-ok content (fs.read-text _ (path "source.txt"))
    (fs.write-text output (path "copy.txt")
      (text.concat "Copied by esh\n" content))))
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
```

Open the local address printed by the command. The editor rechecks source as it changes, links review rows back to source spans, and can download the edited source. It has no execution endpoint. A displayed effect set covers possible operations across all branches; it is not a concrete execution plan.

## Implemented in this slice

- `Unit`, `Bool`, `Text`, `RelPath`, `List T`, and `Result T FsError` in the core.
- Immutable lexical bindings, conditional expressions, text equality/concatenation, and `Result` propagation.
- Explicit capability arguments, opt-in inference candidates, ambiguity errors, and narrowing explanations.
- Optional script result/effect annotations, checked against inferred results.
- Text reads and replacement writes, directory listing/creation, and regular-file removal.
- Kernel-checked request effect bounds, authorization, pure traces, and capability restriction laws.
- Abstract file-content store laws, explicitly separate from concrete OS correctness.

Source-level functions, modules, type/region/effect polymorphism, explicit-state loops, general recursion, `div`, external commands, and generated runtime contracts are **not implemented yet**. Unsupported forms are rejected. These remain part of the design; the first-order slice is not a decision to remove them.

See [prototype syntax](docs/prototype.md) and [implementation stages](docs/roadmap.md).
