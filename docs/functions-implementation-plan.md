# Functions, modules, and polymorphism: implementation decisions

Status: accepted and implemented for the function stage. This document records
the decisions behind [the current syntax](functions.md) and the
[language design](design.md). Concurrency design is closed for this stage;
the scheduler is not part of this implementation.

## 1. Meaning of an effect parameter

Accepted contract: `Effect` parameters range over permission bounds.
`(effects E1 E2 (fs.read R))` is their union with the named operation on `R`.
Each callback keeps its own parameter; it is not silently assigned a common
effect variable with other callbacks.

The checked computation structure separately preserves sequence, branches,
calls, and repeated calls. The viewer exposes that structure and the runtime
trace preserves actual order and duplicates. Permission membership never
justifies reordering, eliminating, or merging operations. A callback's permission
contract alone does not establish its internal ordering or operation count;
the report must say when these are unknown.

Restricted `E*` would belong to a future ordered-behavior description when
repetition is implemented. Applying star to an idempotent permission union
would not express the intended repeated behavior. Conditional refinements
remain a separate extension to the unconditional contract.

For the permission-bound scope, inference composes known bounds
and preserves independent variables. Public signatures are checked universally,
not specialized to whichever calls happen to occur in a script. Unresolved
constraints require sufficient annotations or a diagnostic; the solver never
guesses a partition of a union, treats uncertainty as purity, or substitutes
unrestricted authority. Inference completeness is not promised.

## 2. Calls, captures, and compact types

Accepted rules:

- Calls supply every argument at once. Partial application uses an explicit
  `fn`; there is no automatic currying. Ordinary built-in functions may also be
  passed as values; syntax forms such as `if` and `let` are not functions.
- All calls obey the accepted explicit-sequencing rule, including pure calls.
  Intermediate computations use `let` or `let-ok`; the final computation may
  be a call. A function literal is a value: creating it does not run its body.
  Existing examples and diagnostics are updated for this syntax rather than
  silently inserting bindings with an unreviewed evaluation order.
- Function values and unconstrained type variables are potentially
  authority-bearing unless the checker can establish otherwise. Capturing them
  requires `captures` or `captures _`, even when their invocation effect is
  empty. This rule also follows authority through containers and type aliases.
  Retained regions stay fixed under generalization, including authority hidden
  behind a function type. Parameters of the current function are not captures.
- Capability parameters can opt into `_` argument inference with the existing
  `infer` marker, for example `(params (root (Cap R (read)) infer))`.
  Eligibility is lexical, and resolving a marker still requires exactly one
  suitable candidate. An outer candidate selected inside a closure is also
  subject to the capture declaration. Other locally produced capabilities are
  passed explicitly in this stage; no implicit registration is added.
- Named types initially mean transparent, nonrecursive, parameterized aliases.
  Names are preserved for display and their definitions are shared internally.
  An alias does not create a distinct nominal type or hide authority.
  Type, region, and effect parameters use the accepted `forall` kinds.
- `Cap` is a first-class type, written `(Cap R (read write))`, with the accepted
  automatic narrowing of rights. The existing `Result T FsError` remains the
  error type supported by this stage. Generic error types, new data-type
  declarations, recursive types, and the parallel-result type family are later
  extensions rather than prerequisites for ordinary function polymorphism.

Alias syntax, using the same named-field style as functions:

```lisp
(type ReadResult
  (export)
  (forall (A Type))
  (alias (Result A FsError)))
```

`(ReadResult Text)` has the same type as `(Result Text FsError)`. Private aliases
omit `(export)`. Exported contracts must remain understandable from the public
type definitions; a private alias is expanded when presenting a public contract.

Type, region, and effect arguments to rank-1 definitions are inferred at call
sites. Explicit universal binders belong to declaration schemes, not to
higher-rank callback parameter types. No new general implicit conversions or
implicit term synthesis are introduced.

## 3. Module boundaries and stage scope

Module shape:

```lisp
(module text
  (def identity
    (export)
    (forall (A Type))
    (params (value A))
    (returns A)
    (effects)
    (body value)))
```

Modules contain imports, type aliases, and function definitions. They do not
declare ambient host capabilities or execute top-level initializers. Authority
comes through function arguments. A script may contain local definitions and
imports alongside its capability declarations and one executable body.

An import exposes only exported names through its explicit alias. Definitions
may refer forward to other definitions in an acyclic dependency graph.
Recursive definition cycles are diagnosed in this stage; general recursion
and `div` remain in the subsequent stage of the roadmap. Imports do not run
function bodies, and loading the same source twice does not create a second
module instance.

The CLI uses `--project-root DIR`, defaulting to the entry file's directory.
Imports resolve relative to their importer and canonical targets must remain
within that root. A check or run retains the exact source bytes it checked;
execution does not silently reload a changed module. This is a source snapshot,
not a guarantee that the whole project filesystem was read atomically.

The editor uses an explicit entry-file context for imports. A buffer with no
file context can still be checked as a standalone script, but cannot resolve
local imports by guessing a directory. Source links and diagnostics retain
module identity.

This implementation includes higher-order functions, closures, explicit
sequencing, static modules, rank-1 type/region/effect polymorphism, named aliases,
and review support for these features. Loops, recursion, restricted `E*`,
conditional-contract syntax, and the parallel runtime remain later stages.
Their accepted policies remain in force.

## Validation obligations

- Extend the Lean core and its effect/authority guarantees to function values,
  calls, and substitutions without proof holes or untyped execution fallbacks.
- Verify that a public scheme is valid for all its permitted instantiations;
  preserve the existing distinction between immediate and invocation effects.
- Exercise independent callback effects, captured-region stability, explicit
  captures, capability narrowing, and ambiguous inference diagnostics.
- Exercise import visibility, cycles, project-root confinement, and source
  consistency between checking and execution.
- Exercise explicit sequencing and failure short-circuiting, including pure
  calls, and show the checked order in the viewer.
- Keep the existing assurance boundaries explicit. Extending request-trace
  proofs does not by itself verify source elaboration or operating-system code.
