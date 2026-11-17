# Assurance boundaries

This prototype verifies a small executable kernel. It does not verify the entire frontend, operating system, host adapter, or review interface.

## What Lean checks

`Esh/Core.lean` defines an intrinsically typed expression language. Variables, bindings, branches, and primitive arguments are indexed by their value types. Elaboration produces values of this expression type; evaluation therefore does not need an untyped fallback value.

Expressions compile to `Program policy effects result`. Each primitive request requires a proof of membership in `policy`. Composition concatenates effect bounds; widening requires a subset proof. User effect annotations are converted into finite subset certificates before constructing the executable program.

The same `Program.run` interpreter is used with the real request bridge. It returns an `Execution` containing both the emitted request trace and proofs that every entry belongs to the effect bound and the granted policy. Proofs are checked by Lean and erased during compilation.

The public assurance entry points include:

- `execution_effect_sound`: completed request traces are within the static bound.
- `execution_authorized`: completed request traces are within the supplied policy.
- `pure_trace_empty`: an empty effect bound produces an empty request trace.
- `restrict_no_amplification` and `restrict_same_region`: capability restriction cannot add rights or change the region.

These statements concern the first-order, terminating kernel currently implemented. They do not yet establish prefix safety for future divergent programs, nor correspondence with all observable events inside an OS handler.

`Esh/Model.lean` additionally checks read-after-write, last-write-wins, and frame laws for an abstract file-content store. Locations are resolved independently of region names, so the model does not infer non-aliasing from different names. These elementary laws are groundwork; they are not a refinement proof of the POSIX implementation.

## Trusted and unverified components

- Lean's kernel, compiler, runtime, and standard logical foundations.
- The source parser, elaboration's correspondence to source meaning, and diagnostic/source mapping.
- The JSON request bridge and Python syscall adapter.
- The OS, filesystem, and the interpretation of opened directory descriptors.
- The editor's rendering of the checker's report.

An internal request is authorized relative to the script's declared policy. The host independently checks that actual grants cover those declarations before dispatching requests, then rechecks each request against the active grant subset. The executable does not mint access to an undeclared host root.

The bridge validates host reply shapes before exposing typed values. Invalid replies become `Err` values. This validates representation, not the truth of data reported by the host.

## Concrete host scope

The host uses directory descriptors and `O_NOFOLLOW` for script-path traversal. It refuses special-file reads and uses replacement writes. Tests exercise symlink rejection, path rejection, aliasing regions, hard links, permission checks, and failure propagation.

This is not a complete sandbox against adversarial concurrent changes to the directory namespace. In particular, an already opened descendant directory can be relocated by another process. Descriptor authority, current path containment, and physical inode aliasing are different properties. Stronger confinement needs an OS-specific design and a corresponding model.

External mutation, disk errors, crashes, special filesystems, metadata preservation, and rollback across operations are outside the abstract store laws. A completed early write remains visible if a later operation fails. Temporary-file cleanup can itself fail. The current host is suitable for development experiments with explicit grants, not a claim of production isolation.

## Rechecking

```sh
lake build
python3 -m unittest discover -s tests -v
```

`Esh/Proofs.lean` prints theorem axiom dependencies during the build. The project introduces no custom axioms or admitted proofs. Test success is recorded separately from proof coverage.
