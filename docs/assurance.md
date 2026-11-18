# Assurance and proof coverage

The Rust application is executable and tested, but **the request for a fully
proved Rust language/runtime/editor is not complete**. This document separates
source-linked proofs from tests, models, and assumptions. A successful build or
a static review is not a blanket safety certificate.

## Source-linked Rust proofs

`crates/esh-policy/src/lib.rs` is extracted by pinned Charon and translated by
Aeneas into `verification/EshRust/Generated/`. Handwritten proofs import those
actual generated definitions. `scripts/extract-policy.sh --check` re-extracts
from current source and fails on a difference. Generated files are not hand-edited.

`verification/EshRust/PolicyProofs.lean` establishes:

- The mandatory preflight scan terminates without indexing/overflow failure;
  accepting it implies every supplied check is `Passed`.
- Mutation admission requires passed initial checks, passed rechecks, and no
  reported preservation conflict.
- Reporting success requires body success and passed postconditions.
- Permission bits correspond to read/write/mkdir/remove; mask inclusion rejects
  high bits and requires every requested bit to be present.
- The extracted editor state machine advances revisions without wrapping,
  invalidates reviews on edits, rejects stale results without changing state,
  and accepts a review of the current revision.
- The component-byte scan terminates and, on acceptance, excludes separators,
  backslashes, DEL, and ASCII control bytes.

These are statements about the extracted implementations, not replacement
functions written only for proof. They do not establish that every caller passes
truthful input to the guards. The full relative-path validator and filesystem
identity/protection computation have tests but are not wholly proved yet.

`EshRust/Audit.lean` prints theorem axiom dependencies. The audited statements use
Lean's standard `propext`, `Classical.choice`, and `Quot.sound`; no project-specific
axiom or admitted proof discharges them. The trusted chain still includes Lean's
kernel, the extraction/translation tools, their library models, and the Rust
compiler/runtime that executes the original code.

## Coverage matrix

| Component/property | Current evidence | Remaining formal obligation |
| --- | --- | --- |
| Pure admission and permission decisions | Extracted Rust + Lean theorems | Connect guard premises to complete host executions. |
| Preflight scan and component-byte scan | Termination and acceptance theorems | Complete path splitting/traversal and observation correctness. |
| Editor revision policy | Extracted Rust + Lean; stale-result tests | Rendering/source-span correspondence and projection soundness. |
| Parser, type/effect inference, modules, captures | Rust regression tests | Source soundness, generalization, elaboration correspondence. |
| Sequential interpreter and finite collection operations | Rust execution tests | Preservation/progress, ordered traces and cancellation prefixes for this Rust interpreter. |
| Update state evolution and alias/subtree protection | Preflight, drift, alias, budget, and partial-failure tests | Refine identities/namespace changes to the environmental filesystem model. |
| Static file-impact projection | Conservative unknowns and targeted regressions | Prove every possible relevant action is represented or explicitly unknown. |
| POSIX adapter, SHA-256, Wasm/DOM, dependencies | Explicit trusted/tested boundaries | Appropriate implementation/refinement proofs where feasible. |

## Runtime enforcement

The host independently checks declared capabilities against supplied grants, then
checks each operation against that active subset and the script's checked effect bound. Automatic contract observations also require a matching read effect. Script paths are relative;
descriptor traversal rejects symlinks and special files. Reads and values have
size limits. Writes replace a regular-file directory entry with a prepared file;
they do not mutate the contents of a different hard link. Removal targets regular
files, not arbitrary directory trees.

Mandatory initial conditions precede all mutations. Rechecks run before each
mutation; failed or unfinished checks stop execution. Protected directories cover
their subtrees even through another capability root. The current implementation
rechecks all retained conditions rather than relying on an unproved optimization.
Observed postconditions must pass before success. Earlier successful mutations
remain after a later failure; the report records them in order.

This is conditional on no conflicting external changes to relevant contents or
namespace during the update. Rechecking is not proof of isolation. An already
opened directory can be relocated externally; descriptor identity and current
path containment are different properties. OS-enforced exclusion/isolation,
crash consistency, network filesystems, durability across power loss, rollback,
and whole-program memory/time bounds are not claimed.

Permission effects cover operations/regions; they do not equate repeated writes,
authorize reordering, or describe a callback's internal operation count. Runtime
host responses are shape-checked before dependent values are used. That checks
representation, not the truth of an untrusted backend's data.

## Original Lean reference model

The root `Esh/` files remain checked with the original toolchain. Their
intrinsically typed execution certificates cover request bounds, authorization,
pure traces, substitution, and preservation of repeated ordered requests.
Abstract file-store laws include read-after-write and frame properties.

Those theorems apply to the reference kernel, **not automatically to Rust**. The
reference interpreter's evaluator uses `partial` definitions, and the abstract
store is not a verified POSIX implementation. Its Python adapter/frontend and
historical UI are isolated under `reference/python/` and excluded from the Rust
application. [Historical details](../reference/lean-assurance.md) remain available.

## Reproduction

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
scripts/extract-policy.sh --check
scripts/check-proofs.sh
lake build
(cd reference/python && python3 -B -m unittest discover -s tests -v)
```

[Verifier instructions](../verification/README.md) pin Aeneas, Charon, and the
separate proof toolchain. Tests, proof checking, regeneration, and browser
interaction checks are separate evidence and should be reported separately.
