# Language design

esh targets the work commonly performed by shell and Python scripts, especially filesystem automation. Programs may be generated automatically; a human should be able to review their structure and possible effects. Reviewability takes priority over minimizing source text.

## Types and effects

Internal expressions are inferred where possible. Exported functions require type and effect annotations that act as checked public contracts. The prototype supports rank-1 polymorphism over value types, resource regions, and effects.

The next language stage uses rank-1 polymorphism. A polymorphic definition may be instantiated independently at different call sites. A function received as an argument has one instantiation within that call; higher-rank argument types are deferred.

Named types and shared representations are the first approach to keeping contracts compact. The function stage implements transparent, nonrecursive parameterized aliases and shared type tables. Repetition support will add a restricted `E*` operator to ordered behavior descriptions with verified rules; general user-written fixed-point expressions `mu` are deferred. Detailed iteration rules remain future work.

Generalization is permitted only when the expression's immediate evaluation is provably effect-free. Only type, region, and effect variables not fixed by the environment may be generalized. Creating a function value can therefore be generalized even when invoking that function performs filesystem operations. Generalization never freshens the region of a retained capability. An unresolved immediate effect is not assumed to be empty.

Effects describe a conservative upper bound on operations. Branches contribute their possible effects even when only one branch will execute. Execution order, predicates, and path dependencies belong to the typed program structure rather than an unordered effect set.

An `Effect` parameter ranges over permission bounds; `(effects E1 E2 (fs.read R))` is their union. Function types normalize duplicate and reordered bounds. This never authorizes reordering or eliminating execution steps. Callback ordering and multiplicity remain unknown from a nonempty permission contract alone. Unsupported inference constraints require annotations rather than guessed partitions or unrestricted authority.

Contracts use two layers: an unconditional effect bound and more precise conditional guarantees. Conditions such as absence of specified interference may be declared even when unresolved, but the viewer must identify them as assumptions. They do not silently replace the unconditional bound. Using a type guarantee that depends on a condition requires evidence adequate for its scope; an unchecked assumption is not such evidence. The condition syntax, evidence mechanisms, and detailed composition rules remain to be designed.

Ordinary failures use `Result`. Success-path composition must expose where an error short-circuits later operations. General loops and recursion are allowed in the design. Potential computational nontermination is tracked with `div`; external blocking and waiting need separate treatment.

## Functions and modules

Functions are first-class values: they may be passed as arguments and returned as results. Named and anonymous functions are included in the next stage. Function types describe effects of invoking the function; creating a closure is distinct from executing its body.

Declarations use named fields: `def`, an optional `(export)` marker, `forall`, `params`, `returns`, `effects`, and `body`. Public contracts sit with their definitions. Private definitions may omit inferred annotations. Anonymous `fn` expressions use the same `params` and `body` fields and the applicable `captures` declaration. `(Fn (A) B (effects E))` denotes a one-argument function from `A` to `B` with invocation-effect bound `E`; quantified parameters distinguish `Type`, `Region`, and `Effect`.

Closures require explicit source declarations for captured values carrying authority, including authority held indirectly by another closure. Ordinary value captures may be inferred. The review interface displays retained authority and automatic permission narrowing.

Functions and unconstrained type variables are potentially authority-bearing unless established otherwise, even with an empty invocation effect. Capability parameters may opt into argument inference with `(params (root (Cap R (read)) infer))`. Other locally produced capabilities are initially passed explicitly. Calls require their full arity; partial application uses an explicit `fn`.

An explicit declaration such as `(captures workspace callback)` names authority-bearing values to retain. `(captures _)` explicitly requests inference of all referenced outer authority-bearing values; the viewer displays its expansion. Omitting the declaration permits no authority capture. Capture inference enumerates referenced bindings, unlike capability-argument inference, which must choose exactly one eligible candidate. These two uses of `_` have distinct resolution rules.

Importing a module loads definitions without running initialization code. Initialization is an explicit function call with a type and effect contract. Exported functions require signatures, as above.

The first module implementation uses static local imports with explicit aliases, such as `(import text "./text.esh")` and `(text.trim value)`. Relative paths resolve from the importing file and remain within a designated project root. Import cycles are errors. Cyclic-module support and external package resolution are deferred; these restrictions describe the initial implementation scope.

`--project-root DIR` sets that boundary, defaulting to the entry file's directory. Modules contain imports, aliases, and function definitions; only exports are visible to importers. Authority arrives through arguments. Acyclic forward references are allowed; recursive definition cycles are deferred with recursion. The editor needs a file location to resolve imports.

## Resources

Resource values carry both a region identity and permissions. Region identities do not imply disjoint physical locations. Two roots may alias or overlap.

Resource arguments are explicit in the executable core. Source may contain a marker requesting inference, but only explicitly registered candidates participate. All applicable candidates count: zero or multiple candidates produce a type error. Names, declaration order, and smaller permission sets do not resolve ambiguity.

Permissions may be narrowed automatically to the callee's requirement. The review interface exposes both argument resolution and permission restriction. Restriction does not revoke rights held by another value or by the original capability.

## State and control flow

Values are immutable by default. Repetition carries an explicit input state and produces a next state or a final result. General reassignment and mutable references are deferred. The review interface may present the resulting program as familiar procedural steps while retaining links to the checked structure.

Evaluation order is explicit through `do`, `let`, and `let-ok`, including pure calls. A call's callee and arguments are values; intermediate calls receive bindings instead of being nested inside another call. A call may be the final computation of a block. Conditional tests are values, with computation blocks for their branches. A `do` block elaborates to lexical bindings and returns its final expression; a failed `let-ok` returns `Err` from that block and skips its remaining steps. Function creation remains distinct from invocation.

The editor and viewer may collapse administrative bindings into readable presentations, while exposing their original evaluation order, dependencies, and source mapping. Presentation must not change evaluation order. This policy is accepted for the next language stage; the prototype has not yet implemented the new block syntax or its restrictions.

Concurrent execution uses explicit structured `parallel` blocks. Ordinary `do`/`let` remains sequential, each child preserves its own evaluation order, and every child is joined before its block exits. Individual `spawn`/`await` operations and detached background tasks are outside the initial scope.

The execution has one shared job limit, including nested parallel blocks and imported function calls. Its default is one job; callers may explicitly increase it. A larger number of declared children does not require simultaneous execution of all children. Parameter spelling remains to be decided. The limit controls execution capacity; admission order follows the scheduler rule below, and the limit does not establish absence of interference from other actors.

Computation, in-flight I/O, and cleanup retain one job slot. A caller and its own in-flight operation share that slot; they are not counted twice. A parent that is only joining children, with no in-flight work of its own, holds no slot. It reacquires a slot before resuming ordinary work. Waiting for an owned I/O operation is therefore different from waiting only for children. This accounting applies throughout nested blocks and allows children to use the capacity released by a joining parent even when the limit is one.

The initial scheduler uses one global FIFO queue. Newly eligible children and resuming parents enter the tail; children created by one `parallel` block are enqueued in source declaration order. The oldest eligible entry receives the next free slot. There are no user priorities or preferential admission for resuming parents. Running jobs retain their slot until completion or a join-only suspension; cancellation checkpoints do not rotate running jobs. Cooperative round-robin scheduling is deferred.

Fairness applies to slot admission: later arrivals do not overtake an already queued, eligible task. Given finitely many predecessors, continuing scheduler service, and continuing slot availability, it is eventually admitted unless cancelled. No bounded waiting time or equal CPU share is promised when computation or I/O can hold slots indefinitely. FIFO admission does not make effect order or outcomes deterministic. This fixes the initial concurrency scope; implementation and proof work should preserve these rules without adding scheduling modes.

When a child returns `Err` to its `parallel` block, the block stops queued children, requests cooperative cancellation of running siblings, and joins them before returning failure. In-flight operations and cleanup are awaited; completed changes are not rolled back. A locally handled error that does not leave the child as `Err` does not trigger this rule. Cancellation is a request to stop, not a guarantee of immediate interruption of a blocking operation. Concrete result encoding and cleanup contracts remain to be designed.

Cancellation is checked at semantic boundaries: before each `do`/`let` computation step, including pure work, at function entry and loop iteration boundaries, before starting a child, before resuming work after waiting, and before admitting a host operation. These checks inspect the cancellation flag; they do not imply a filesystem rescan. In the initial implementation, an admitted built-in operation runs through result handling and its cleanup before cancellation can stop subsequent ordinary work. Additional checkpoints inside built-ins require separately specified safe boundaries and are deferred. The built-in's outcome and any completed effects must remain visible; delaying cancellation does not make its implementation an atomic transaction.

Parallel results preserve the types and names of individual children. Complete success returns `Ok` with the named values. Failure returns `Err` with a typed report containing each child's successful value, failure, cancellation, or not-started outcome. Successful partial values are available to explicit error handling; missing results are not replaced by fabricated values. The concrete type constructors and syntax remain to be specified.

Failure reports retain all errors actually observed through joining, including cleanup failures, with task identity, nesting path, and phase. The default display follows source declaration order; the cancellation trigger and observation order are also retained. This presentation does not make the set of observed errors independent of scheduling. Reporting observed errors does not require running cancelled or unstarted work to discover hypothetical additional failures.

## Built-ins and external commands

Basic filesystem operations should have verified internal implementations with explicit models and assumptions for the OS boundary. Proofs must state their coverage, including failure behavior, instead of equating a verified request protocol with a verified operating system.

An external command introduces an explicit unrestricted effect. Command declarations may also generate runtime checks. Passing such checks does not remove the unrestricted effect.

Type-establishing validation remains necessary before exposing a value at its expected type. Supplementary expectation checks run within an aggregate additional-cost budget. A detected violation is an error; an incomplete check alone does not stop execution. Checked, violated, and incomplete states remain distinguishable. Expensive global properties are explicit requirements rather than properties silently added to ordinary types.

## Review tools

The viewer and executor share elaboration results. Review tools expose inferred types/effects, resolved capabilities, narrowed permissions, generated checks, branch conditions, state transitions, and links to source. Initial work focuses on static bounds. Concrete change planning and pre-execution diffs are future extensions.

## Open details

Further inference completeness, restricted iteration rules, state-dependent refinements, conditional-contract syntax and proof rules, concrete parallel result types, cleanup contracts, recursion analysis, external-command contracts, and concrete filesystem refinement proofs remain future work. Functions, captures, rank-1 polymorphism, local imports, explicit sequencing, aliases, and their review views are implemented in the current stage. The two-layer-contract and concurrency policies are accepted design commitments whose runtime and proofs are still deferred. Scheduler implementation must preserve the accepted transition, capacity, and conditional-fairness rules without reopening its basic design.
