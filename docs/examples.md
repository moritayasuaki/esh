# Practical filesystem workflows

All fixtures contain synthetic data. Run each workflow against a fresh copy of
its fixture; scripts intentionally reject some already-updated states.

```sh
cargo build --locked --release -p esh-cli
mkdir -p .local/demo
cp -R examples/fixtures/config .local/demo/config
./target/release/esh check examples/config-update.esh
./target/release/esh run examples/config-update.esh \
  --grant workspace read,write .local/demo/config
```

Choose a fresh destination when repeating the fixture-copy command. A
`--grant` opens the chosen directory as an explicit capability root. Review all
replacement/removal targets before giving a script authority.

| Script | Fixture directory | Required rights | Behavior and deliberate failure case |
| --- | --- | --- | --- |
| `config-update.esh` | `config` | `read,write` | Replace the known development configuration; protect `secrets/`. Changed starting text or a supplied maintenance lock blocks all changes. |
| `batch-transform.esh` | `batch` | `read,write` | Prefix each UTF-8 file from sorted `inbox/` into `outbox/`; preserve originals. Invalid UTF-8 stops after earlier completed outputs. Existing destination contents are replaced. |
| `organize-files.esh` | `organize` | `read,write,remove` | Copy each file's bytes into `archive/`, then delete that source only after successful copying. A failed destination write leaves that source. This is copy-then-delete, not atomic rename. |
| `rotate-backups.esh` | `backup` | `read,write` | Read current and first generation before replacing second and first; preserve current. A missing required generation blocks the whole update at preflight. |
| `migrate-v1-v2.esh` | `migration` | `read,write` | Update settings before publishing version 2; protect `data/`. An unexpected initial version blocks all changes. Readers require the stated non-interference protocol. |
| `manifest.esh` | `manifest` | `read,write` | Fold sorted entry names into a new manifest; preserve input. An already-existing manifest blocks the run. |

For another row, copy its fixture and change the script and rights:

```sh
cp -R examples/fixtures/organize .local/demo/organize
./target/release/esh run examples/organize-files.esh \
  --grant workspace read,write,remove .local/demo/organize
```

The two `.keep` files preserve empty destination directories in Git; they are
not source inputs. `fs.list` includes ordinary dot-prefixed names. Directory
entries in a batch input cause the file-read operation to return `Err`; this
stage does not add implicit recursive directory walking.

## Additional initial state

```sh
./target/release/esh check examples/config-update.esh \
  --state examples/expected-config.state
./target/release/esh serve examples/config-update.esh \
  --state examples/expected-config.state
```

The supplied state adds an absent `maintenance.lock` requirement. It does not
replace conditions embedded in the script. `check` and the viewer show premises;
`run` validates them against granted roots.

## Tests and review

```sh
cargo test --locked -p esh-cli --test examples
```

The suite executes the actual distributed source files and imports through the
CLI, in disposable directories. It checks successful contents, preserved data,
operation order, no mutation on initial failure, and partial completion after a
later failure. Unit/integration suites additionally cover alias-root protection,
external drift, budgets, malformed responses, and type/effect regressions.

Known literal paths are shown precisely in file-impact review. A name discovered
by `fs.list` remains a runtime path, with the callback's behavior displayed under
restricted `E*`. That summary does not authorize reordering or coalescing calls.
