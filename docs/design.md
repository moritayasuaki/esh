# Language design

esh targets the work commonly performed by shell and Python scripts, especially filesystem automation. Programs may be generated automatically; a human should be able to review their structure and possible effects. Reviewability takes priority over minimizing source text.

## Types and effects

Internal expressions are inferred where possible. Exported functions require type and effect annotations that act as checked public contracts. The planned language supports polymorphism over value types, resource regions, and effects.

Effects describe a conservative upper bound on operations. Branches contribute their possible effects even when only one branch will execute. Execution order, predicates, and path dependencies belong to the typed program structure rather than an unordered effect set.

Ordinary failures use `Result`. Success-path composition must expose where an error short-circuits later operations. General loops and recursion are allowed in the design. Potential computational nontermination is tracked with `div`; external blocking and waiting need separate treatment.

## Resources

Resource values carry both a region identity and permissions. Region identities do not imply disjoint physical locations. Two roots may alias or overlap.

Resource arguments are explicit in the executable core. Source may contain a marker requesting inference, but only explicitly registered candidates participate. All applicable candidates count: zero or multiple candidates produce a type error. Names, declaration order, and smaller permission sets do not resolve ambiguity.

Permissions may be narrowed automatically to the callee's requirement. The review interface exposes both argument resolution and permission restriction. Restriction does not revoke rights held by another value or by the original capability.

## State and control flow

Values are immutable by default. Repetition carries an explicit input state and produces a next state or a final result. General reassignment and mutable references are deferred. The review interface may present the resulting program as familiar procedural steps while retaining links to the checked structure.

## Built-ins and external commands

Basic filesystem operations should have verified internal implementations with explicit models and assumptions for the OS boundary. Proofs must state their coverage, including failure behavior, instead of equating a verified request protocol with a verified operating system.

An external command introduces an explicit unrestricted effect. Command declarations may also generate runtime checks. Passing such checks does not remove the unrestricted effect.

Type-establishing validation remains necessary before exposing a value at its expected type. Supplementary expectation checks run within an aggregate additional-cost budget. A detected violation is an error; an incomplete check alone does not stop execution. Checked, violated, and incomplete states remain distinguishable. Expensive global properties are explicit requirements rather than properties silently added to ordinary types.

## Review tools

The viewer and executor share elaboration results. Review tools expose inferred types/effects, resolved capabilities, narrowed permissions, generated checks, branch conditions, state transitions, and links to source. Initial work focuses on static bounds. Concrete change planning and pre-execution diffs are future extensions.

## Open details

The full surface grammar, polymorphic inference algorithm, module system, recursion analysis, external-command contract language, and concrete filesystem refinement proofs remain to be designed and implemented. Prototype syntax is provisional and does not establish those future rules.
