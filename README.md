# esh

A typed S-expression language for filesystem automation that a human can review
before running. The current implementation is Rust: parser, type/effect
inference, interpreter, internal commands, local server, and browser editor.
Browser application logic compiles to WebAssembly; JavaScript bindings are
build-generated.

The first practical workflows cover configuration updates, batch transformations,
file organization, backup rotation, version migrations, and manifests. Scripts
can state the filesystem conditions they require, what they must preserve, and
what must hold before success is reported. The viewer shows overwrite targets,
execution conditions, repeated operations, and unresolved paths.

**Formal verification is partial.** Actual Rust policy code is extracted with
Charon/Aeneas and proved in Lean. The complete Rust compiler, interpreter,
filesystem adapter, and review projection are not yet proved. The earlier Lean
kernel remains a separate reference model. See [assurance](docs/assurance.md).

## Build and inspect

Use Rust/Cargo (tested toolchain: 1.98.1) and a POSIX host. Cargo dependencies are
locked. No Python or Lean executable is required to run the Rust application.

```sh
cargo build --locked --release -p esh-cli
./target/release/esh check examples/config-update.esh
./target/release/esh check examples/config-update.esh --state examples/expected-config.state
cargo test --locked --workspace
```

`check` parses and analyzes source and its local imports. It does not inspect the
script's target filesystem. Declared initial conditions are premises until a run
validates them. Reports use JSON and capability-relative paths.

## Run an update

Start with a disposable fixture; choose a destination that does not already exist:

```sh
mkdir -p .local/demo
cp -R examples/fixtures/config .local/demo/config
./target/release/esh run examples/config-update.esh \
  --state examples/expected-config.state \
  --grant workspace read,write .local/demo/config
```

The script replaces `workspace:/app.conf` only if the expected development
configuration and other mandatory conditions hold. It protects the `secrets`
directory and its entire subtree. A second run fails its old-configuration
precondition. A declaration never grants access: callers supply roots and rights.

```lisp
(script
  (cap workspace (read write))
  (requires (exact-text workspace "app.conf" "mode=development\n")
            (directory workspace "secrets"))
  (preserves (target workspace "secrets"))
  (ensures (exact-text workspace "app.conf" "mode=production\n"))
  (returns (Result Unit FsError))
  (effects (fs.read workspace) (fs.write workspace))
  (do
    (let config (path "app.conf"))
    (fs.write-text workspace config "mode=production\n")))
```

Every mandatory initial check must pass before mutation. The runtime rechecks
relevant conditions before each mutation and checks `ensures` before success.
Rechecking detects drift but does not prove absence of concurrent interference.
Failures report completed changes; there is no automatic rollback.

[Run all six workflows](docs/examples.md), including their failure cases.
[Update-contract semantics](docs/update-contracts.md) explain alias roots,
preservation, check budgets, and conditional guarantees.

## Rust/Wasm review editor

Install the Wasm target and a binding generator matching the locked Rust crate:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked --root .local/wasm-tools
./target/release/esh build-web
./target/release/esh serve examples/config-update.esh --state examples/expected-config.state
```

Open the loopback address printed by `serve`. The entry buffer is editable;
imports are read-only source snapshots. Check explicitly after edits: old review
results are invalidated immediately. File-impact targets link to their source.
Download saves the buffer as a source file. The editor has no script-execution
endpoint. `--port N` changes the local port.

The editor model and review analysis are independent of the browser renderer,
so a future GPUI interface can reuse them. This release is a browser interface.

## Language scope

- Explicit `do`/`let` sequencing, including pure calls; `let-ok` stops on `Err`.
- Named and anonymous higher-order functions, authority captures, rank-1
  type/region/effect polymorphism, and mandatory exported contracts.
- Definition-only local modules and transparent, nonrecursive type aliases.
- Shared type substructures and a report graph with references to earlier nodes.
- Finite `List` values, ordered `list.map`/`list.fold`, and first-error
  `list.map-ok`/`list.fold-ok`; repeated behavior is displayed using restricted `E*`.
- Text and byte I/O, strict UTF-8 conversion, checked dynamic paths, sorted
  directory listing, directory creation, and regular-file removal.
- Explicit capabilities, opt-in `_` inference, ambiguity errors, and visible
  permission narrowing.

Imports resolve relative to their importing file and stay within
`--project-root DIR` (default: the entry file's parent). Imports cannot follow
symlinks. Each analysis/run uses the source bytes it loaded.

General recursion, unbounded loops, general `mu`, external processes/pipelines,
OS-enforced isolation, and the agreed parallel runtime are later stages.
See [syntax](docs/prototype.md), [functions/modules](docs/functions.md),
[architecture](docs/architecture.md), and [roadmap](docs/roadmap.md).

## Proofs and limits

```sh
scripts/extract-policy.sh --check  # requires the pinned Charon/Aeneas tools
scripts/check-proofs.sh           # checks the extracted Rust proofs
lake build                       # checks the separate original Lean model
```

[Verifier setup and exact coverage](verification/README.md) are separate from
normal builds. The historical Python frontend/host and handwritten UI are
retained only under [reference](reference/README.md).

CLI limits: `--max-bytes` (default 8 MiB per operation/value), `--max-steps`
(1,000,000 semantic checkpoints), `--max-checks` (100,000 observations), and
`--max-check-bytes` (64 MiB cumulative checked content). Mandatory validation
that exceeds its budget blocks the update. These limits are not a whole-process
memory bound or an OS-operation timeout. Exit status is 0 for success, 1 for an
execution/contract failure, and 2 for source or setup errors.
