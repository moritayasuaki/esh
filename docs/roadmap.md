# Implementation stages

The first slice establishes a source-to-checked-core-to-host path and a review editor. Features below remain part of the design but are not accepted syntax yet.

1. **Source functions and modules.** Add required exported signatures, private inference, and type/region/effect polymorphism. Specify permission inclusion and generalization before implementing the inference algorithm.
2. **Structured state transitions.** Add finite iteration, general loops, recursion, and `div`. Extend the semantics and proofs to finite prefixes of potentially divergent executions.
3. **Verified filesystem algorithms.** Specify directories, aliasing, errors, partial completion, and namespace interference. Connect abstract operations to the concrete adapter rather than treating store laws as an OS proof.
4. **External-command contracts.** Keep an explicit unrestricted effect. Generate type-establishing validation and budgeted supplementary checks with distinct checked, violated, and incomplete outcomes.
5. **Review tools.** Extend the current source-linked type/effect view to binding provenance, data flow, branch conditions, and explicit state transitions. Validate the views on realistic scripts before expanding syntax.
6. **Concrete planning.** Investigate read/prepare/apply stages and reviewed change plans as a later extension. Static effect bounds remain distinct from concrete plans.
