# Rust implementation architecture

The application path is Rust throughout. The browser renderer is Rust compiled
to Wasm; only its binding/bootstrap code is generated JavaScript.

| Crate | Responsibility |
| --- | --- |
| `esh-policy` | Pure permission, path-byte, admission, success, and revision decisions; Aeneas extraction target. |
| `esh-syntax` | Bounded S-expression parser and Unicode-aware source spans. |
| `esh-lang` | Type/region/effect inference, captures, module checks, shared types, ordered IR, contracts, static impact projection. |
| `esh-host` | Explicit grants and descriptor-relative POSIX internal commands; bounded byte/text I/O and observations. |
| `esh-runtime` | Sequential evaluation, ordered finite collection operations, cancellation checkpoints, update checks, preservation and partial-change journal. |
| `esh-cli` | Source snapshot loading, command-line interface, loopback HTTP transport, Wasm asset building. |
| `esh-editor` | Renderer-independent document revisions, analysis, supplied-state integration, and stale-review rejection. |
| `esh-web` | Browser DOM rendering and editing, implemented with Rust/Wasm. |

`Program` contains typed metadata and ordered IR. Permission unions summarize
possible operations; the interpreter traverses the original sequence. Repeated
calls are never removed because their permission labels happen to be equal.
Transparent aliases and `Arc` substructures avoid copying whole type trees. The
review type graph interns subterms; parent entries refer to earlier entries.

The source loader confines imports to a project root, refuses symlinks/special
files, and provides an immutable in-memory snapshot to compilation. The browser
edits the entry buffer over that module snapshot. New imports require restarting
with an updated project snapshot; the browser cannot request arbitrary files.

The impact projection follows known closures and paths, keeps branch conditions,
and marks unknown callback bodies, runtime paths, and truncated analysis. It is
bounded and does not execute the script or inspect target files. A successful
static check never means live preconditions have passed.

Update enforcement is independent of the rendering layer. Per-request host
permission/path checks remain in place after compilation. Condition observations
and state evolution occur in `UpdateHost`; filesystem effects go through `Host`.
Rust error paths preserve the completed-success journal and do not roll back.

The browser server binds loopback, checks Host/Origin, serves a fixed allowlist
of assets, and offers no run/save endpoint. Source content is rendered as text.
Per-render event listeners are removed when the view is replaced. A future GPUI
renderer can reuse `esh-editor` without putting browser types into the model.

## Verification boundary

Charon extracts the supported Rust policy subset; Aeneas translates it to Lean.
Generated definitions are checked for reproducibility and imported by handwritten
proofs. `verification/` pins its own Lean version required by Aeneas. The original
`Esh/` kernel uses the root toolchain and remains a separate semantic reference.
Neither model silently certifies unproved Rust compiler or host code.

See [assurance](assurance.md) for what is proved and [the roadmap](roadmap.md) for
remaining source correspondence, filesystem refinement, and review-soundness work.
