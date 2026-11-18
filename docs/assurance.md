# Assurance boundaries

This prototype verifies a small executable kernel. It does not verify the entire frontend, operating system, host adapter, or review interface.

## What Lean checks

`Esh/Functions.lean` defines intrinsically typed values, closures, environments, and computations. Variables, arguments, results, and computation bounds are indices. A closure contains a typed body and only its selected environment values. The kernel has no untyped fallback value.

`esh_language.py` parses source, loads modules, infers schemes, resolves captures, and produces a shared typed representation. `Esh/FunctionBridge.lean` independently checks that representation before execution, including unused function bodies with symbolic type/region/effect parameters. It checks applications, capability restriction, declared function bounds, and the script bound. Shared type references must refer to earlier entries.

Primitive requests still use `Program` from `Esh/Core.lean` and the real request bridge. Every request requires evidence of authorization. Higher-order evaluation returns an indexed `Execution` carrying the result, emitted trace, and proofs of bound and policy membership. Composition appends traces; normalization applies to permission bounds, not to execution traces. Proofs are checked by Lean and erased during compilation.

The public assurance entry points include:

- `Functions.execution_effect_sound`: completed request traces are within the static bound.
- `Functions.execution_authorized`: completed request traces are within the supplied policy.
- `Functions.pure_trace_empty`: an empty effect bound produces an empty request trace.
- `Functions.append_preserves_trace`: composition retains ordered, repeated trace entries.
- `Functions.subst_included`: effect substitution preserves inclusion.
- `Functions.substitution_keeps_capability`: substituting a value leaves a concrete capability's region and rights unchanged.
- `Functions.mem_normalize`: permission-bound normalization preserves membership.
- `restrict_no_amplification` and `restrict_same_region`: capability restriction cannot add rights or change the region.

Value substitution is a total, checked transformation of stored values and code;
it does not run the original initializer. Unexpected type/context/bound mismatches
are rejected rather than coerced unsafely. Closure application evaluates typed
arguments and a typed body. The higher-order evaluator uses Lean `partial`
definitions because application is not structural recursion. There is no exposed
fuel cutoff, and no termination theorem is claimed for that interpreter. The
frontend currently rejects recursive definition cycles. The statements above
cover completed executions, not a formal correspondence theorem for every
interpreter step or finite-prefix safety under divergence/cancellation. They also
do not cover all observable events inside an OS handler.

The original first-order expression/compiler proofs in `Esh/Core.lean` remain
available as reference lemmas and underpin the primitive request machinery.

`Esh/Model.lean` additionally checks read-after-write, last-write-wins, and frame laws for an abstract file-content store. Locations are resolved independently of region names, so the model does not infer non-aliasing from different names. These elementary laws are groundwork; they are not a refinement proof of the POSIX implementation.

## Trusted and unverified components

- Lean's kernel, compiler, runtime, and standard logical foundations.
- The Python source parser, inference/generalization and capture-declaration rules, module visibility, elaboration's correspondence to source meaning, and diagnostic/source mapping. These have tests, not a complete formal soundness/completeness proof of the source frontend.
- The JSON request bridge and Python syscall adapter.
- The OS, filesystem, and the interpretation of opened directory descriptors.
- The editor's rendering of the checker's report.

An internal request is authorized relative to the script's declared policy. The host independently checks that actual grants cover those declarations before dispatching requests, then rechecks each request against the active grant subset. The executable does not mint access to an undeclared host root.

The bridge validates host reply shapes before exposing typed values. Invalid replies become `Err` values. This validates representation, not the truth of data reported by the host.

## Concrete host scope

The host uses directory descriptors and `O_NOFOLLOW` for script-path traversal. It refuses special-file reads and uses replacement writes. Tests exercise symlink rejection, path rejection, aliasing regions, hard links, permission checks, and failure propagation.

Module loading has a separate designated project root. Canonical targets must
remain inside it, and descriptor-relative reads reject replacement links and
special files. The loaded source bytes are reused for execution. This is not an
atomic snapshot of a concurrently changing project tree.

This is not a complete sandbox against adversarial concurrent changes to the directory namespace. In particular, an already opened descendant directory can be relocated by another process. Descriptor authority, current path containment, and physical inode aliasing are different properties. Stronger confinement needs an OS-specific design and a corresponding model.

External mutation, disk errors, crashes, special filesystems, metadata preservation, and rollback across operations are outside the abstract store laws. A completed early write remains visible if a later operation fails. Temporary-file cleanup can itself fail. The current host is suitable for development experiments with explicit grants, not a claim of production isolation.

## Rechecking

```sh
lake build
python3 -m unittest discover -s tests -v
```

`Esh/Proofs.lean` prints theorem axiom dependencies during the build. The project introduces no custom axioms or admitted proofs. Test success is recorded separately from proof coverage.
