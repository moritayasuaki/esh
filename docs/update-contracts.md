# Filesystem update contracts

Implemented in the Rust runtime and review tools. Conditions describe only the
necessary initial state. An omitted fact is unknown, never implicitly absent.
A supplied specification is not evidence that the live filesystem matches it.

## Source and supplied state

```lisp
(requires
  (regular-file app "payload.bin")
  (directory app "data")
  (absent app "maintenance.lock")
  (exact-text app "VERSION" "1\n")
  (content-hash app "payload.bin"
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"))
(preserves (target app "data"))
(ensures (exact-text app "VERSION" "2\n"))
```

These named fields belong to `script`. Targets are a declared capability name
and a literal validated relative path. Content hashes are 64 hexadecimal digits
of SHA-256 over file bytes; hex case is ignored. Text compares exact UTF-8 bytes,
including line endings. A regular file excludes symlinks and special files.

An optional `--state FILE` supplies `(state condition ...)`. It is conjoined with
`requires`; it cannot weaken or replace script requirements. The CLI and browser
use the same parser and contract-combination checks. Conditions require declared
read authority; their observations add `fs.read` to the effect bound. Explicit
script bounds must include those observations. Contradictory conditions cannot
both pass; they are never silently dropped.

## Execution phases

1. Open caller-supplied capability roots and verify declared rights are a subset.
2. Validate all mandatory initial conditions and establish preservation targets.
   Mismatch, unavailable evidence, or an exhausted check budget blocks mutation.
3. Before each mutation, revalidate the current conditions and protected-target
   identity, then reject writes/removals/directory creation that overlap protection.
4. Execute the admitted internal command through descriptor-relative operations.
5. Record successful mutations and evolve conditions on the directory entries
   changed by that operation. Do not replace unrelated requirements with newly
   observed external state.
6. If the body succeeds, validate `ensures`. Only then report overall success.

The implementation conservatively rechecks all current requirements before each
mutation; a dependency-based reduction can be added with its own correctness
argument. A successful own text write evolves matching requirements to the new
exact text; a byte write uses its digest, removal uses absence, and directory
creation uses directory existence. Replacement does not change a different hard
link's contents, so its condition is not refreshed as though it had been written.

Contract violations halt the run even if ordinary operation errors could be
handled in the script. A script returning `Err` does not establish successful
postconditions. Nested `Result` values remain ordinary typed data.

## Preservation and aliases

`preserves` prohibits this program's mutations to the target. If it is a
directory, protection covers its entire subtree. An absent protected target may
not be created. Read operations are permitted. This is not a promise that all
filesystem metadata, such as access timestamps, remains unchanged.

Capability names do not imply physical separation. The runtime compares opened
filesystem identities, ancestry, and unresolved path suffixes. It checks access
through another root too. Protection is conservative for the same inode through
hard links. Path components and symlinks are checked independently of this rule.
Replacing a protected directory's ancestor cannot be used to bypass protection:
this command set only replaces/removes regular files and does not rename trees.

External replacement of a protected target between observations is reported as
drift when detected. The check/operation interval still relies on the conditions
below; it is not a transaction or an adversarial filesystem sandbox.

## Conditional guarantee

Successful postconditions describe observed final state under these explicit
assumptions:

- No conflicting external content or namespace changes affect the specified
  targets throughout the update.
- The OS honors the modeled descriptor and filesystem operation semantics.

Observations are not atomic snapshots. Rechecks do not prove uninterrupted
non-interference, lock a directory, or justify a dependent type indefinitely.
The static viewer may use initial text facts before mutation, then conservatively
invalidates them after a potentially interfering operation. Unresolved targets
and branches remain visible. State predicates are not new source-level refined
types in this release.

## Costs and failure reports

`--max-checks` bounds observations across preflight, rechecks, and postconditions.
`--max-check-bytes` bounds cumulative content bytes checked; `--max-bytes` also
bounds individual reads. Reaching a mandatory-check limit reports incomplete
validation and stops. An already established failure stops additional checking.
This differs from budgeted supplementary expectation checks, which cannot
discharge a mandatory premise and are not implemented for external commands yet.

Run JSON contains check phase, predicate, target, outcome, and error, plus an
ordered `completedChanges` journal. A postcondition failure can occur after
successful mutations; those mutations remain in place. A failed OS call or
cleanup can have effects outside that completed-success journal. Crash recovery,
transactional rollback, and recovery from partially failed system calls need a
stronger OS model and are not promised here.

The [assurance matrix](assurance.md) distinguishes proved policy decisions,
tested runtime enforcement, and environmental assumptions.
