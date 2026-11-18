# Function and effect design notes

The function stage implements higher-order functions, rank-1 type/region/effect
polymorphism, explicit sequencing, local modules, transparent aliases, and review
support. `Effect` parameters describe permission bounds; ordered computation
structure remains separate. See [current syntax](functions.md) and
[implementation decisions](functions-implementation-plan.md).

The notes below retain the rationale and future design candidates. Restricted
iteration, conditional guarantees, and the parallel runtime remain future work.
Concurrency policy is accepted and is not reopened by this implementation. The
language's commitments are recorded in [the design](design.md).

## Definition syntax

The declaration structure in this section and the explicit-sequencing policy below are accepted.

Use named fields for function declarations. An export marker and its full public contract sit with the definition. A private definition uses the same form without `(export)` and may omit inferred annotations.

```lisp
(module functions
  (def apply
    (export)
    (forall (A Type) (B Type) (E Effect))
    (params (f (Fn (A) B (effects E))) (x A))
    (returns B)
    (effects E)
    (body (f x)))

  (def identity
    (params x)
    (body x)))
```

`Fn` has an argument-type list, a result type, and an invocation-effect bound. For example, `(Fn (A) B (effects E))` accepts one `A`, returns `B`, and may perform effects in `E` when called. `forall` binders explicitly distinguish `Type`, `Region`, and `Effect` parameters. Public annotations contain the full signature; private inference does not silently add public parameters or effects.

The following factory illustrates region polymorphism and the distinction between constructing a closure and invoking it:

```lisp
(def make-reader
  (export)
  (forall (R Region))
  (params (root (Cap R (read))))
  (returns (Fn (RelPath) (Result Text FsError)
    (effects (fs.read R))))
  (effects)
  (body
    (fn
      (params path)
      (captures root)
      (body (fs.read-text root path)))))
```

Calling `make-reader` retains the supplied capability but does not read a file. Calling its result can read from that capability's region. An effect-free creation step does not imply an authority-free closure. The viewer displays both retained authority and the invocation-effect bound.

The already accepted `(captures _)` form may replace the explicit capture declaration. Source spans link inferred captures and permission narrowing back to their declarations and uses.

## Effect composition

Implemented semantics: effects are conservative sets of possible operations. Variables denote effect sets, and `(effects E1 E2 (fs.read R))` denotes their union. Ordering and duplicate entries do not change this bound. Execution traces still retain the actual order and repeated requests.

```lisp
(def run-both
  (export)
  (forall (E1 Effect) (E2 Effect))
  (params
    (first (Fn () (Result Unit FsError) (effects E1)))
    (second (Fn () (Result Unit FsError) (effects E2))))
  (returns (Result Unit FsError))
  (effects E1 E2)
  (body
    (let-ok ignored (first)
      (second))))
```

Each callback keeps its own effect bound. If one can read `input` and the other can write `output`, the combined bound contains both operations. `let-ok` skips the second callback if the first returns an error; the static upper bound still includes both. The type does not force both callbacks to share a single effect variable.

An inferred body bound must be included in its annotation. A smaller invocation-effect bound may satisfy a larger expected bound; this widening grants no authority. Capability permissions still obey their separate, already accepted restriction rules.

Union does not determine a unique partition: knowing `E1 ∪ E2` does not in general determine `E1` and `E2`. Inference must preserve sound constraints or request sufficient annotations where it cannot establish the needed result. It must not invent a partition, silently replace uncertainty with unrestricted effects, or treat an unresolved immediate effect as pure. The precise supported constraint forms and solver completeness remain implementation work; generalization and all accepted programs still require a sound justification.

Union and inclusion on concrete finite effect sets are decidable. The difficulty is solving for unknown effects while retaining useful polymorphic contracts. For instance, both `E1 = {read}, E2 = {write}` and `E1 = {read, write}, E2 = {}` satisfy `E1 ∪ E2 = {read, write}`. Multiple solutions do not themselves require an error: a polymorphic type can retain variables and constraints representing the solutions. Forward composition from known callback bounds does not require such a partition. Filesystem observations can constrain a particular execution, but do not by themselves identify the independent contracts of arbitrary callbacks.

This differs from a common-row design that gives both callbacks a shared effect upper bound. That alternative can simplify inference, but it can lose the distinction between individual callback contracts. Koka's discussion of [polymorphic effect inference](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/paper-20.pdf) explains the underlying union/unification tradeoff. Its soundness results do not establish soundness of this proposed language.

[Flix's effect-polymorphism documentation](https://doc.flix.dev/effect-polymorphism.html) illustrates a different approach with independent effect variables and set-formula composition. The relevant design problem is the chosen constraint language and inference procedure, not a general impossibility of inferring set-based effects. No external language's solver or proof is assumed to apply unchanged to region polymorphism, captured authority, or the sub-effecting rules proposed here.

## Order and multiplicity

The accepted union semantics is an operation-permission summary, not an algebra of execution. A full behavioral description may need a distinct sequential product and branch join. In mathematical notation:

```text
seq(E1, E2)       first E1, then E2
join(E1, E2)      an upper bound on alternative branches
permissions(seq(E1, E2)) = permissions(E1) union permissions(E2)
```

The permission projection forgets order and repetition deliberately. It does not assert that the underlying operations commute or are idempotent, and does not justify reordering or eliminating requests.

An upper-bound count illustrates a commutative but non-idempotent sequential operation: sequential counts combine with addition, so `1 + 1 = 2`; counts across alternative branches combine with maximum. For order-sensitive behavior, concatenation of traces distinguishes `read;write` from `write;read`, and `write;write` from `write`. Counting summaries describe usage, not a general commutativity law for the corresponding physical operations.

[Sequential effect systems](https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ECOOP.2017.13) provide a foundation for keeping sequencing distinct from joins. The implementation retains an ordered, source-linked computation structure alongside permission summaries; it does not yet derive count summaries. The function stage chooses the permission projection for public `Effect` parameters. Richer behavioral contracts, including restricted iteration, remain a later extension.

Failure and conditional execution must remain explicit: a failed `let-ok` executes only the prefix through the failing operation. An ordered description must include that path rather than asserting that every later operation occurs. Repetition requires a count bound or an explicit unbounded approximation; it must not collapse multiple executions into one.

A callback's permission set alone does not bound how often it uses a permitted operation. Without a stronger contract or a justified analysis of its body, multiplicity remains unknown or unbounded. An unbounded count is distinct from unrestricted authority. Symbolic repeated callback invocations must remain visible even when their projected permission sets are identical.

Reasoning about commutativity or idempotence of actual operations needs a specified observational equivalence covering relevant values, state, errors, and external observations. Different region names alone do not justify such laws because roots may alias. No optimizer may infer those laws merely from a permission set or a counting summary.

## Compact representations and fixed points

Accepted scope: use named type constructors and shared representations first, then add restricted `E*` with repetition support; defer general user-written `mu` expressions. These choices keep contracts readable without erasing their distinctions. For example, a schematic `ParallelResult Jobs` can derive both the complete success record and the typed partial-result report from one named, finite job schema. Each job retains its own value and error types. The spelling, schema syntax, and implementation of these type families are proposals, not additions already accepted by the parser. This need not introduce general row polymorphism or erase values into `Any` or JSON. Repeated structure can be shared internally and expanded on demand by the viewer. Smaller representations do not eliminate the memory needed to retain actual partial-result values.

Fixed points address a related but different source of growth: recursive and repeated behavior. In a trace-language interpretation, a finite-iteration operator can be expressed schematically as:

```text
E* = mu X. (epsilon join seq(E, X))
```

Here `mu` denotes a least fixed point, `epsilon` contains the empty trace, `join` combines alternatives, and `seq` concatenates behavior in order. This denotes zero or more finite completed iterations. It does not assert termination or replace `div`. For `E = read;write`, the order within each iteration and the possibility of multiple iterations remain significant. This expression does not equate two writes with one. In a count projection an unbounded iteration may require an infinite upper bound, rather than a count of one. Finite prefixes ending during an iteration, failures, cancellation, and cleanup require their own modeled paths or an appropriate prefix closure.

The trace example is a choice of semantic model, not an iteration law assumed for every effect domain. [Work on sequential effects](https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ECOOP.2017.13) distinguishes sequential composition from joins and studies the additional conditions needed for iteration. In particular, rewrite laws must be proved for the chosen model: in trace languages `(A join B)*` is not generally `A* join B*`, since the former permits mixed repetitions.

[Mathlib's fixed-point development](https://leanprover-community.github.io/mathlib4_docs/Mathlib/Order/FixedPoints.html) provides least fixed points for order-preserving maps on complete lattices. A possible Lean proof would establish the chosen semantic domain, monotonicity of the effect functional, and the connection to execution. A checkable bound `B` satisfying `F(B) <= B` can justify `mu F <= B` without computing an exact least fixed point. Scope and interference assumptions still have to be carried through this argument.

Existence of a semantic fixed point is not a terminating inference algorithm. General recursive trace equations can describe context-free languages, whose unrestricted inclusion problem is undecidable; [language-inclusion research](https://ris.utwente.nl/ws/files/28131311/1997ctit31.tr.pdf) contrasts this with regular-language inclusion. Positive or monotone recursion supports the semantic construction but does not alone settle inclusion checking. Supported inference would need a restricted fragment, suitable abstractions or proof certificates, or annotations when it cannot establish the necessary bound. It must not silently replace unresolved behavior with unrestricted effects.

Named/shared expressions followed by restricted iteration are the accepted direction; the semantic model and inference rules above still need design and proof. General recursion and iteration remain in the existing later implementation stage. A recursive error-data structure, if needed, is a separate type-design problem: an effect fixed-point operator does not automatically simplify a heterogeneous partial-result record.

## Explicit sequencing

Accepted policy: make sequencing explicit with `do`, `let`, and `let-ok`. The callee and arguments of a call are values; intermediate calls, including pure calls, receive bindings. A call may also be the final computation in a block. Conditional tests are values and each branch has its own computation block. This follows the value/computation separation used in [A-normal form](https://www.cs.tufts.edu/comp/150FP/archive/cormac-flanagan/anormal.pdf), expressed in the source language rather than hidden solely in a compiler intermediate representation.

```lisp
(do
  (let source (path "source.txt"))
  (let target (path "copy.txt"))
  (let-ok content (fs.read-text input source))
  (let decorated (text.concat "Copied\n" content))
  (let result (fs.write-text output target decorated))
  result)
```

This example illustrates the implemented sequencing policy. A `do` block elaborates to a chain of lexical bindings, with the final expression providing the block's result. A `let-ok` failure returns `Err` from that block and skips subsequent steps, matching the existing nested `let-ok` behavior. It does not become a nonlocal return from an arbitrary caller.

Including pure calls makes the sequencing structure independent of effect inference and supplies explicit points for updating state facts. The editor may collapse administrative bindings while retaining an expandable order, data dependencies, and source mapping. Its presentation must preserve the checked evaluation order.

Calls require full arity. Partial application uses an explicit `fn`; there is no automatic currying. Merely creating or referencing a function does not execute its body.

## Filesystem-state refinement

Under review: supplement the unconditional effect contract with conditional facts about a particular execution. A fact may describe validated input data, file kind, a snapshot, or a relationship between resolved locations. Such facts can narrow branches and establish operation preconditions. They do not grant capabilities, remove the effect of an operation that still executes, or establish success of every later OS call.

An observation about a live pathname has a lifetime. A later check/use sequence can race with another process; the [Python OS documentation](https://docs.python.org/3/library/os.html#os.access) describes this issue for permission checks. Rechecking immediately before an operation does not remove the intervening race. Evidence must be tied to an appropriate mechanism: immutable data actually used by the computation, a suitable snapshot, an atomic operation that both checks and acts, or a justified exclusion of interfering changes. An open descriptor identifies an opened object but does not by itself freeze its contents.

State facts must be updated or invalidated after writes, removals, and possible changes through aliasing paths or regions. Unrestricted external commands require conservative invalidation unless stronger confinement is established. The viewer should identify the assumptions, their evidence, and their validity scope; a conditional bound must not replace a reusable public contract without its premises.

A Lean development would state preconditions, success and failure postconditions, and the permitted environment interference. Its theorems would be conditional on those modeled premises and on the connection between runtime evidence and the model. This is additional work beyond the current request-trace proofs.

Only needed facts should be inspected. Incomplete supplementary analysis keeps the conservative bound and need not stop execution. Evidence necessary to expose a value at a refined type must be established before that value is used; failure stops the dependent path. No default full-filesystem scan or exhaustive pre-execution simulation is proposed.

Observations used for refinement also have effects and costs. For example, reading a configuration once into immutable data can justify eliminating an unreachable branch for that particular execution, but the read itself remains in the execution's effect accounting. Re-reading the live pathname later would require new evidence. Explicit sequencing provides program points at which knowledge can be introduced, updated, or invalidated, and helps connect pre/postconditions with ordered effect descriptions.

## Concurrency

Accepted policy: concurrent execution uses structured `parallel` blocks and one configurable job limit shared across the entire execution, including nested blocks. The default is one job. Failure handling requests cancellation and joins all children, as described below.

Explicit structured concurrency preserves the accepted sequencing policy. A `parallel` block starts scoped child computations, preserves each child's `do`/`let` order, and joins all children before returning. Cross-child execution order is unspecified except for explicit dependencies. Ordinary blocks remain sequential; the language does not silently schedule independent source steps concurrently. Individual `spawn`/`await` operations and detached tasks are outside the initial scope. The feature's implementation stage, exact syntax, result representation, and detailed scheduler and cleanup rules still require decisions.

For comparison, Python's [task groups](https://docs.python.org/3/library/asyncio-task.html#task-groups) wait for their children at scope exit. That lifetime rule is useful here, but Python's exception and cancellation semantics do not determine esh's `Result` behavior.

This example illustrates the accepted block structure; the complete grammar and result representation are not yet finalized:

```lisp
(do
  (let path-a (path "a.txt"))
  (let path-b (path "b.txt"))
  (parallel
    (do (fs.read-text input path-a))
    (do (fs.read-text input path-b))))
```

For future extensions, `spawn` and `await` separate task creation from waiting: `spawn` makes a child eligible to run and returns a task handle; `await` waits for that child's completion and obtains its result. Awaiting one previously spawned task does not prevent another child from progressing. Scheduling need not mean immediate execution or allocating a dedicated thread. Explicit handles allow other work and individual waits between task creation and the final join. These operations can themselves be scoped: `spawn` is not synonymous with a detached background task. The initial `parallel` construct groups child computations and their final join in one source construct, making the concurrency boundary visible without individual task handles.

A spawned child's effects may occur before its result is awaited. A future task type must not account for those effects only at `await`. Authority capture, lifetime, and failure rules would apply to either interface.

For the job limit, a proposed CLI shape is `esh run script.esh --jobs 4`; this is illustrative and not implemented. Declaring more children than the limit would queue work rather than require all children to execute simultaneously. [GNU make's job slots](https://www.gnu.org/software/make/manual/html_node/Parallel.html) are a useful reference for a numeric concurrency cap, and its [jobserver](https://www.gnu.org/software/make/manual/html_node/Job-Slots.html) illustrates sharing a budget across nested invocations.

The accepted budget scope is the whole esh execution, including nested parallel blocks and imported function calls. Giving each nested block an independent full budget would violate this limit. The default of one avoids automatically increasing concurrency with machine capacity; callers explicitly request a higher limit. As a reference, GNU make also defaults to one job slot, but its other scheduling rules are not automatically adopted here. FIFO controls admission order; a one-job limit does not imply a total source order across nested computations or change their effect contracts.

Accepted slot policy: computation, an owned in-flight operation, and cleanup retain one execution slot. The caller and its owned operation share a slot rather than consuming two. A parent suspended only to join children holds no slot when it has no in-flight work of its own; it reacquires one before ordinary computation resumes. A cancellation request alone does not release a slot while the operation or its cleanup is still running. Waiting for an owned filesystem operation or external process is not a join-only wait.

| State | Slot ownership |
| --- | --- |
| Queued, or ready to resume but not admitted | None |
| Executing computation, including pure computation | One |
| Waiting for the job's owned in-flight operation | One, shared with that operation |
| Only joining children, with no owned in-flight operation | None; each active child accounts for itself |
| Performing cleanup | One |
| Fully settled | None |

With a one-job limit, a parent uses the slot until entering `parallel`, suspends its continuation while joining, and makes the slot available to a child. A child can do the same at another nested block. Once the children settle, the parent becomes ready and acquires capacity before continuing. The scheduler must support suspended continuations or equivalent progress: releasing a numeric permit while exhausting all workers with blocked joins would not implement this rule. This avoids capacity deadlock caused solely by joining parents; it does not prove termination of children or OS operations, or fairness of the ready queue.

Implementation invariants should establish unique slot ownership, no double counting, occupied slots at most the configured limit, and no premature release while owned work remains in flight. Scheduler bookkeeping must be able to record cancellation and observe completion without waiting for a user-job slot. User computation and effectful cleanup still require capacity. Releasing a slot does not release capabilities, discharge resource obligations, or make aliased files independent. Concrete runtime data structures and proofs remain implementation work; additional local caps are outside the agreed initial scheduler scope.

The cap should describe work managed by esh, not claim a bound on all system threads or processes. An external command can create its own workers; bounding those requires cooperation, such as a jobserver protocol, or a separately specified execution constraint. A job limit also does not establish a task order or absence of interference by external actors. Static effect bounds must still cover every child that may execute, including repeated operations, at any supported limit.

The viewer could distinguish declared tasks from queued, running, and waiting work and display the shared budget. Limit enforcement and freedom from scheduler-induced capacity deadlock are additional proof obligations; a numeric limit alone proves neither termination nor resource-conflict freedom.

### Scheduler transitions and admission order

Accepted policy: use a single global FIFO queue for work that needs an execution slot. Newly eligible children and parents resuming after a join enter the tail; the oldest eligible entry receives the next free slot. Children introduced by one `parallel` block are enqueued in source declaration order. Separate readiness events are ordered as the scheduler processes them. There are no user priorities or special priority for resuming parents. A ready task has at most one queue entry.

The accepted initial policy is non-preemptive slot ownership: a running job keeps its slot until it settles or suspends only to join children. Cancellation checkpoints remain active but do not themselves rotate running jobs. Cooperative round-robin scheduling is deferred; there is no ordinary running-to-ready transition solely to give another task a turn in this initial policy.

| From | Event | To | Slot change / queue action |
| --- | --- | --- | --- |
| Ready | Oldest eligible entry admitted | Running | Acquire one slot |
| Running | Host operation admitted | InFlight | Retain the same slot |
| InFlight | Outcome recorded | Running or Cleanup | Retain the same slot |
| Running | Suspend only to join children | Joining | Release slot; newly eligible children enter queue tail |
| Joining | All children, including cleanup, have settled | Ready, if computation or cleanup must resume | Enter queue tail; no slot until admission |
| Running | Required cleanup begins | Cleanup | Retain slot |
| Running or Cleanup | Final outcome published and owned work settled | Settled | Release slot exactly once |

Cancellation is a pending flag alongside these states. A cancelled entry that has not started can settle as not-started without acquiring a slot. Previously started work still joins its children and performs any required cleanup before settling; effectful cleanup needs capacity. Control bookkeeping does not require a user-job slot. A scheduler event that observes a child failure records the failure and marks its scope for cancellation before granting newly freed capacity to affected ordinary work. Work already admitted follows the existing completion and cleanup policy. Terminal outcomes remain immutable.

The fairness guarantee is limited to admission: later arrivals do not overtake a persistently eligible entry already in the FIFO queue. With finitely many entries ahead, continued scheduler service, and continuing slot availability, that entry is eventually admitted unless cancelled. This does not promise equal CPU time, a wall-clock wait bound, completion order, or progress when all slots are held forever by nonterminating computation or blocked operations. Source-order insertion within one block does not make the full concurrent execution deterministic.

FIFO admission and these state transitions define the accepted initial scheduler scope. Queue uniqueness, slot conservation, coordinated cancellation/admission, and joining before settlement are the central proof obligations. Detailed runtime data structures may implement these rules without introducing additional scheduling modes. Broader scheduling features remain later extensions rather than prerequisites for the language's function and type work.

### Cancellation checkpoints

Accepted policy: check the cancellation flag before each executable `do`/`let` step, including pure computations, at function entry and loop iteration boundaries, before child admission, before resuming ordinary work after waiting, and before admitting a host operation. These are cancellation checks, not requests to repeat filesystem analysis. The function and loop boundaries also apply when those features are introduced in their planned stages. The viewer should expose the checkpoints in the checked structure and their source locations.

The initial built-in contract defers cancellation from admission through result handling and required cleanup. It does not inject cancellation at arbitrary instructions within a built-in. For example, a file replacement already admitted may finish writing its temporary file, replace the target, and finish cleanup after a sibling fails. Its actual outcome must be recorded before control stops at a subsequent boundary. Completed effects, operation failures, and cleanup failures must not disappear into a generic cancelled status. There is no new bounded-time cancellation promise for a blocking operation, nor a guarantee that deferring cancellation makes a multi-step operation atomic against other actors.

Adding internal checkpoints later requires a specified interruptible phase, preserved invariants, cleanup behavior, and truthful reporting of any partial completion. Required built-in cleanup is protected from the pending cooperative cancellation request; arbitrary user-defined uninterruptible blocks are not introduced by this policy. Cleanup effects remain within the operation's contract and cleanup errors remain part of reporting. Trio's [checkpoint and cancellation documentation](https://trio.readthedocs.io/en/stable/reference-core.html#checkpoints) is a useful reference for explicit boundaries; its thread API also distinguishes waiting for synchronous work from abandoning a still-running worker. These external semantics are not adopted as esh's scheduler implementation.

Detailed operational rules must coordinate cancellation observation with admitting new steps and operations, and distinguish a request from a published terminal outcome. Already settled results are not retroactively overwritten by cancellation. Preserving an operation's outcome does not turn every locally handled `Err` into a failing child. Exact publication transitions remain part of the scheduler formalization.

A proposed core representation treats cancellation as a task-execution outcome, preserving the ordinary result type of a pure expression instead of wrapping every expression type in `Result`. That representation and its substitution, generalization, and preservation proofs still require design. The accepted cancellation checkpoints do not yield execution capacity in the initial scheduler; admission fairness follows the FIFO rule above.

### Behavioral composition and failure reporting

Concurrency requires its own behavioral composition, provisionally `par(E1, E2)`, distinct from `seq` and branch `join`. A model can preserve per-child order and fork/join dependencies as a partial order, with executions selecting allowed interleavings. A request-only trace does not describe interleavings inside a multi-step filesystem implementation; the model must specify its atomic steps before proving properties of real files.

The permission projection of `par` may still be set union, while count upper bounds for two children add because both may execute. Cancellation can reduce the actual count; it does not justify dropping repeated operations from the upper bound. A branch-choice count instead uses the maximum. Neither a permission projection nor an additive count proves that physical operations commute. Unordered scheduling does not make a duplicated task idempotent.

Shared-resource reasoning needs more than permission narrowing. Existing capabilities can be retained and aliased; narrowing one does not make its owner the sole writer. [Permission-based concurrent separation logic](https://arxiv.org/abs/1411.0851) supplies a reference model for shared reads and exclusive writes. Applying that idea here would require explicit rules for borrowing or partitioning authority, task captures, authority-bearing results, and evidence about actual resource identity. Region names or distinct path strings alone cannot establish separation: roots may overlap, objects may have aliases, and directory changes may interfere with descendants. Unrestricted external commands cannot pass a noninterference check merely because their arguments mention different paths.

A conservative initial candidate admits concurrent access when supported evidence establishes compatible access, and reports unresolved conflicts rather than silently adding locks or serialization. This conflict policy is not yet accepted. Required evidence must be sound within the declared environment assumptions; a one-time path inspection does not exclude later interference. Runtime checks should concern the involved resources, not scan the entire filesystem. Shared reads alone do not establish a stable snapshot against other processes.

Accepted failure policy: when a child returns `Err` to its `parallel` block, that block stops queued children, requests cooperative cancellation of running siblings, and joins every child before returning failure. Locally handling an error and returning a successful child result does not trigger sibling cancellation. In-flight operations and cleanup are awaited. Cancellation need not interrupt a blocking OS operation immediately and does not roll back completed writes. The policy makes no claim of bounded cancellation latency.

For example, with a two-job limit, suppose A and B have started and C is queued. If A returns `Err`, C is not started and B receives a cancellation request. B's already admitted operation may still complete, and required cleanup must finish before the block returns failure. The accepted checkpoints identify the semantic boundaries; the admission rules must formalize their coordination with cancellation requests. Cleanup may itself perform effects that belong in the contract.

The cancellation scope is the failing `parallel` block. A shared job budget does not imply unconditional cancellation of the entire execution. For nested blocks, a failure returned from an inner block can be handled by its caller; if it leaves the enclosing child as `Err`, that outer block applies the same cancellation rule. A type-establishing check must still prevent dependent uses of an invalid value. The default policy does not add automatic retries, which could repeat non-idempotent effects.

Accepted result policy: complete success returns `Ok` containing named, typed values. Failure returns `Err` containing typed per-child outcomes, including successful partial values that a caller can explicitly recover. The outcome distinguishes successful return, failure, cancellation, and not-started work. Child value and error types remain distinct; no common dynamic payload type is implied. A cancelled or unstarted child must not receive a fabricated successful result. The concrete type constructors, task-label syntax, and record encoding still require design.

The accepted error policy retains all failures actually observed through joining, including cleanup failures, with task identity, nesting path, and phase. The default presentation follows source declaration order, with the cancellation trigger and observation order retained separately. A trigger is not necessarily the underlying cause of every reported failure. Scheduling can affect which tasks finish, fail, or never start; source-order presentation does not make those outcomes deterministic. Error collection does not run stopped tasks merely to discover more failures. Python's [task-group failure aggregation](https://docs.python.org/3/library/asyncio-task.html#task-groups) is a reference for collecting observed failures after joining, not an implementation or proof of the typed result design here.

For implementation design, a task's body outcome and its cleanup diagnostics may be separate fields. This permits retaining an original body error alongside cleanup errors, or a valid returned value alongside a later cleanup error, without falsely reporting complete group success. This representation is still a proposal. A value may appear in a typed field only when its type, lifetime, and retained authority remain justified at the return boundary. A returned filesystem-related value does not by itself establish that the live filesystem remains unchanged.

Cancellation requests and final outcomes are distinct: a child may finish successfully or fail before it acknowledges cancellation. Reports must reflect the settled outcome. Failed or cancelled work may already have performed effects; only a not-started body is known not to have executed its own steps. Recovering a partial value does not roll back or automatically retry other operations.

A candidate report representation preserves the nested task structure and derives a source-ordered error view from it. Distinct errors with identical messages must remain distinguishable. Propagating an existing nested failure should retain its origin rather than present its wrapper as a newly independent fault. Exact representation of scope cancellation without a child error, cleanup error types, and origin identities remains open. Alternative modes that finish started work or collect all child outcomes are not part of the accepted initial failure policy.

Refinement facts crossing a fork must remain valid under every permitted sibling action and modeled external interference. Otherwise they must be weakened or invalidated. Lean proof goals would include typed task results, scoped lifetimes, authorization, and conformity of every finite execution prefix to the concurrent behavior contract. Race-freedom claims would additionally require a resource separation and OS interference model; the current sequential request-trace proofs do not establish them. Scheduling fairness, termination, and bounds on blocking require separate assumptions.

The viewer could display task lanes, fork/join points, required ordering, shared resources, and potential conflicts. It should distinguish unspecified scheduling from a recorded execution order, show repeated operations, and expose possible partial completion after failure. Choosing an event loop, worker threads, or processes is an implementation question; the surface model need not promise simultaneous execution or a speedup.

## Conditional guarantees under interference

Accepted policy: retain an unconditional effect bound and allow more precise conditional guarantees, including explicitly unresolved assumptions. The viewer identifies such assumptions, and dependent type guarantees require adequate evidence. The detailed formalization below remains a proposal.

[Rely-guarantee reasoning](https://arxiv.org/abs/1603.01776) distinguishes assumptions about environment steps from commitments about the program's own steps. This is a possible foundation for conditional effect bounds and state guarantees; its existing results do not prove an esh implementation sound.

The proposal retains an unconditional operation bound and adds explicitly conditional refinements. Schematically, for executions allowed by the base runtime model:

```text
always: own_requests(trace) are bounded by E_base
if P(initial_state) and environment_respects(R, trace):
    own_behavior(trace) satisfies E_refined
    successful completion establishes Q(final_state)
```

`P` describes starting facts. `R` constrains interference throughout the relevant interval, rather than just the starting filesystem state. `E_refined` may retain order and multiplicity as well as a narrower operation bound; `Q` is a state postcondition, not merely an effect label. The unconditional contract still bounds the program's requests and authority when the extra premise cannot be established. It does not bound actions of unrelated processes or assert that lexical paths identify stable objects under arbitrary namespace changes. Unrestricted external commands retain their unrestricted effect.

A condition needs a precise resource footprint, conflicting operations, lifetime, and environment scope. For example, a promise about the final contents of a copied file may require that no other actor modifies or replaces the destination between the write and the stated observation point. It need not exclude unrelated filesystem activity. Such a promise must also specify success and permitted partial failure. A final-state guarantee does not claim that the filesystem remains unchanged forever after the function returns.

For concurrent children A and B, a proof can check that A's possible state changes respect B's rely condition and vice versa, alongside the external-environment assumptions and starting conditions. This can permit specified compatible interference rather than requiring complete isolation. Aliasing, operation atomicity, and namespace changes remain part of the proof obligations. Conditional commutativity requires its own proof; it does not imply idempotence or authorize automatic retries.

The viewer would distinguish a condition established for its stated scope, an explicit unresolved environmental assumption, and a detected violation. A monitor that has found no violation supplies only its stated observation coverage; it does not prove that no interfering change occurred. Static compatibility checks can address modeled child tasks. Exclusion mechanisms, snapshots, or validation protocols require their own precise guarantees about other actors. No exhaustive filesystem scan or automatic global monitoring is implied.

If a supplementary condition is unresolved, the program may continue under the unconditional bound while the extra guarantee remains conditional. A detected contract violation follows the existing error policy. A condition required for a refined type must have adequate evidence before any dependent use; otherwise the declared failure path is taken. Live-state evidence must not escape its validity scope, and a conditional empty effect does not justify generalization unless its premise is established for that evaluation. Switching to the base bound after an invalid assumption has already been used does not repair the earlier use or undo filesystem changes.

Public contracts would expose their conditions, with substitution preserving the resource identities and scopes on which they depend. The surface syntax, permitted condition language, call-site propagation rules, and evidence mechanisms remain design questions. They should be confirmed before implementing conditional contracts or concurrency.

## Implementation obligations

- Preserve typing and effect bounds through type, region, and effect substitution.
- Keep regions of captured capabilities fixed during generalization and instantiation.
- Enforce capture declarations for direct and indirect authority; show inference and narrowing in the review report.
- Keep closure creation effects separate from invocation effects.
- Check exported definitions against their declared schemes, rather than inferring and silently publishing a wider interface.
- Extend execution proofs to function calls and retain authorization checks at the host boundary.
- Load and check a module dependency graph without executing imported bodies; reject cycles and project-root escapes.

General recursion and its `div` treatment remain in the subsequent implementation stage described in the [roadmap](roadmap.md). The present proposal does not change the planned language's support for recursion.
