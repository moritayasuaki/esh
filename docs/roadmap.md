# Implementation stages

The source-to-checked-core-to-host path and local review editor are implemented.
Accepted design policies and implemented syntax are distinguished below.

1. **Source functions and modules — implemented.** Required exported signatures, private inference, higher-order closures, explicit sequencing, local modules, transparent aliases, and rank-1 type/region/effect polymorphism. See [current syntax](functions.md). The solver conservatively diagnoses unsupported constraints. Source-frontend correctness and interpreter-step correspondence remain formalization work.
2. **Structured state transitions.** Add finite iteration, general loops, recursion, and `div`. Extend the semantics and proofs to finite prefixes of potentially divergent executions.
3. **Verified filesystem algorithms.** Specify directories, aliasing, errors, partial completion, and namespace interference. Connect abstract operations to the concrete adapter rather than treating store laws as an OS proof.
4. **External-command contracts.** Keep an explicit unrestricted effect. Generate type-establishing validation and budgeted supplementary checks with distinct checked, violated, and incomplete outcomes.
5. **Review tools.** Extend the current source-linked type/effect view to binding provenance, data flow, branch conditions, and explicit state transitions. Validate the views on realistic scripts before expanding syntax.
6. **Concrete planning.** Investigate read/prepare/apply stages and reviewed change plans as a later extension. Static effect bounds remain distinct from concrete plans.
