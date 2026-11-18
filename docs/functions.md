# Functions, modules, and polymorphism

This describes the implemented function stage. The remaining language design is
tracked separately in [design.md](design.md).

## Definitions and calls

```lisp
(def identity
  (export)
  (forall (A Type))
  (params (value A))
  (returns A)
  (effects)
  (body value))
```

The fields may appear in any order, but cannot be repeated. An exported
definition requires parameter types, `returns`, and `effects`. Type variables
are introduced explicitly by `forall`; its kinds are `Type`, `Region`, and
`Effect`. Public bodies are checked with rigid parameters, including unused
definitions. A body that always returns text cannot satisfy `forall A. A -> A`.

Private definitions omit `(export)` and may use `(params value)` and inferred
returns/effects. Named definitions may refer forward to other definitions;
dependency cycles are rejected. Recursive functions belong to a later stage.

Anonymous functions use `fn` with `params`, `body`, optional `returns`/`effects`,
and captures where needed. They receive an inferred rank-1 scheme when bound.
Creating a function does not invoke its body. Ordinary text and filesystem
built-ins are also function values.

All arguments are supplied at once. `(text.concat "prefix")` is an arity error;
write `(fn (params text) (body (text.concat "prefix" text)))` for partial
application. A callback parameter is monomorphic within one invocation, even
when its caller can instantiate the enclosing definition at different types.

## Evaluation order

```lisp
(do
  (let path (path "input.txt"))
  (let-ok text (fs.read-text root path))
  (let decorated (text.concat "Read: " text))
  (ok decorated))
```

Non-final steps are `let` or `let-ok`; the final computation supplies the result.
Callees, arguments, and conditional tests are values. Intermediate pure calls
need bindings too. A function literal is a value. Conditional computations go
in branch-local `do`/binding blocks; literal-value branches need no extra block.
The earlier lexical forms `(let name value body)` and `(let-ok name value body)`
also remain explicit blocks.

An `Err` at `let-ok` skips the rest of that block. The block must return
`Result T FsError`. `let` does not propagate errors implicitly. Each binding's
computation is evaluated once; instantiating a polymorphic value transforms its
type annotations and stored code without evaluating that binding again.

## Authority and captures

Capabilities have type `(Cap R (read write))`. Passing one to a parameter that
requires fewer rights inserts a checked narrowing operation. Inferred private
capability parameters accumulate the rights required by their uses.

Closure bodies may infer ordinary captures. Values that carry or may carry
authority require a declaration:

```lisp
(fn
  (params path)
  (captures root)
  (body (fs.read-text root path)))
```

`(captures _)` requests enumeration of referenced outer authority-bearing
bindings. Omitting `captures` permits no authority capture. A function value or
unconstrained type variable is potentially authority-bearing unless established
otherwise; an empty invocation effect does not establish absence of retained
authority. Known authority-free named functions may be captured ordinarily.

Runtime closures retain only the values selected by the elaborator, including
ordinary dependencies. Direct capability captures are narrowed to the rights
needed by all their uses; returning a capability itself retains the rights of
that returned value. Other closures' hidden authority is not silently erased.
The report shows inferred captures, retained rights, and unknown authority.

Top-level `(cap root (read) infer)` declarations and parameters such as
`(params (root (Cap R (read)) infer))` opt into capability-argument inference.
`_` must identify exactly one eligible lexical candidate; there is no priority
for a name, declaration position, or smaller rights set. Other local capability
values are passed explicitly. Selecting an outer candidate also requires the
applicable capture declaration.

## Polymorphism and effects

Generalization requires a provably empty immediate effect. Only variables not
fixed by the environment or retained authority are generalized. An effectful
`let` or `let-ok` cannot create a polymorphic success value merely because that
value is a function. Captured capability regions remain fixed.

`(effects E1 E2 (fs.read R))` is a union of permission bounds, preserving
independent callback parameters. The kernel normalizes bounds in function
types, so duplicates and permutations do not change those types. Computation
structure and execution traces preserve order and repeated requests.

The solver supports direct instantiation, forward unions, and checked inclusion.
It does not promise complete set-constraint inference. Unsupported equations
or inclusions produce an annotation diagnostic; they never invent a partition
or substitute unrestricted effects. A callback's permission contract alone
does not bound internal multiplicity. An empty bound does establish no
filesystem requests.

## Named types and modules

```lisp
(type Outcome
  (export)
  (forall (A Type))
  (alias (Result A FsError)))
```

`(Outcome Text)` is transparently equal to `(Result Text FsError)`. Alias
parameters may also have Region and Effect kinds; Effect arguments use
`(effects ...)`. Alias cycles are errors. Types are interned in a shared table
for kernel input and review metadata; references cannot form cycles.

Modules use `(module name ...)` with imports, aliases, and function definitions.
They cannot acquire ambient host capabilities or run initializers. Import with
`(import io "./lib/io.esh")` and access exported names through `io.name`.
There is no implicit re-export or package resolution. A script may additionally
declare capabilities and contains one executable body.

Imports are resolved relative to their importer. `--project-root DIR` selects
their boundary; the default is the entry file's parent. Canonical targets must
remain inside it. Descriptor-relative reads reject links substituted during
traversal and reject special files. This does not prove atomic snapshots or
confinement against relocating already-open directories. Source bytes are
retained for the check/run, so execution does not reload a changed module.

The current limits are 256 KiB per source file, 50,000 parsed forms, 128 levels
of source nesting, and 128 files/4 MiB per module graph. These are implementation
limits, not the eventual language's semantic limits.

`serve --file script.esh [--project-root DIR]` supplies import context to the
editor. A buffer without a location supports standalone checking; imports
require a location. Imported sources are reviewable but read-only. The editor
does not write project files or execute scripts.

See [assurance boundaries](assurance.md) for what the Lean proofs establish and
which source-level properties still depend on the frontend.
