# Implementation stages

The current executable application is Rust. Its browser editor runs Rust/Wasm.
The original Lean kernel remains a separately checked reference.

1. **Implemented: functions, modules, and rank-1 polymorphism.** Public contracts,
   private inference, higher-order closures, explicit authority captures,
   `do`/`let`, static definition-only imports, and transparent shared aliases.
2. **Implemented: first filesystem-update workflows.** Ordered finite map/fold,
   first-error variants, Bytes/UTF-8, checked dynamic paths, initial-state inputs,
   mandatory preflight, rechecks, protected directory subtrees, postconditions,
   partial-change reports, and six tested workflows with fixtures.
3. **Implemented: Rust review tools and initial extracted proofs.** Source-linked
   file-impact review, explicit unknowns, runtime-check obligations, shared editor
   state, Rust/Wasm rendering, and Aeneas→Lean proofs for selected policy loops
   and decisions. The [assurance matrix](assurance.md) is authoritative.
4. **Remaining: full Rust correspondence and filesystem refinement.** Prove the
   source compiler/interpreter, descriptor traversal, update state evolution,
   alias/preservation checks, and review projection. Connect OS assumptions to
   the command implementation. Existing kernel theorems are not inherited by a
   rewrite without a correspondence proof.
5. **Later: richer automation.** General recursion/unbounded loops and divergence,
   richer errors/data types, external-command contracts and pipelines, and
   budgeted supplementary checks. External processes retain explicitly
   unrestricted effects; wrappers must not erase them.
6. **Later: native editor and concurrency.** A GPUI renderer can reuse the Rust
   document/review model. The accepted structured-parallel/FIFO design remains
   unchanged: shared global job budget, default one, cancel-and-join, typed
   partial results, and no automatic rollback. Its runtime is not implemented.

Finite-list behavior uses restricted `E*` in review summaries, preserving order
inside each iteration. General user-written `mu` is deferred. Effect bounds remain
permission projections, separate from ordered behavior and state contracts.

The next proof work should close the explicit gaps in stage 4 before advertising
the application as fully verified. OS isolation, source-level refined types, and
external-command syntax require their own reviewed designs before implementation.
