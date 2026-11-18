# Rust → Aeneas → Lean

This package proves selected functions extracted from `esh-policy`. It is
separate from the original root Lean model and uses the Lean version required by
the pinned Aeneas backend. See [coverage](../docs/assurance.md).

| Input | Pin |
| --- | --- |
| Aeneas | `1dc8ad31b1b062a77c50e29144a6c7bba955fa09` |
| Charon | `454586e9b770231e66d2062993e632dfc06463e2` |
| Extraction Rust | `nightly-2026-09-17` |
| Proof Lean | `v4.36.0-rc2` (`lean-toolchain`) |

The policy crate has no dependencies. The remaining Rust application is not
currently an Aeneas extraction target. No generated definition is replaced with
a handwritten specification to make a proof pass.

## Install verifier tools

With Git and Nix installed:

```sh
scripts/setup-verifier.sh
```

The script fetches the official pinned sources and changes only Charon's Nix
packaging/toolchain configuration: native target only, minimal toolchain, library
tests plus generated-AST checks. Cross-target integration tests are not run by
this packaging recipe. The verifier implementation source is unchanged. This
avoids installing toolchains for unrelated architectures. Build artifacts stay
under ignored `.local/` and in Nix's store. The build can still need substantial
disk space and an initial network download.

Alternatively build the same commits using the
[official Aeneas instructions](https://github.com/AeneasVerif/aeneas/blob/1dc8ad31b1b062a77c50e29144a6c7bba955fa09/documentation/getting-started.md)
and set `CHARON_BIN` and `AENEAS_BIN` to the resulting binaries.

## Regenerate and check

From the repository root:

```sh
scripts/extract-policy.sh --check-fingerprint
scripts/extract-policy.sh --check
scripts/check-proofs.sh
```

`--check-fingerprint` runs on Linux and macOS in CI. It needs Cargo and
`shasum`. The launcher computes SHA-256 for the Rust policy source, its Cargo
manifest, the tool pins, and both Lean outputs. The
[esh checker](../scripts/extraction/check-fingerprint.esh) reads the freshly
computed and tracked manifests with read-only grants. It compares the complete
text, so removing a valid entry also fails. The standard `shasum -a 256 -c`
command independently verifies the recorded digests. The Python/Lean reference
suite runs the same esh checker on matching and incomplete manifests.

`--check` also runs Charon and Aeneas in a temporary directory and uses
`diff -ru` to compare the whole generated directory, including unexpected files.
Both check modes leave tracked files untouched.

After an intentional policy change, run `scripts/extract-policy.sh --write`.
The [esh installer](../scripts/extraction/install.esh) reads all staged inputs
and checks the set of generated filenames before the first write. Its only write
grant is rooted at `verification/`. It replaces or creates
`EshRust/Generated/Types.lean`, then `EshRust/Generated/Funs.lean`, then
`extraction.sha256`. The launcher checks the installed fingerprint before
reporting success. If a later write fails, the esh report lists completed
changes. Earlier replacements remain on disk. Inspect the report, correct the
failure and rerun the command. Review the diff and replay the Lean proofs.

To inspect the installer's paths and ordering in the browser, use
`esh serve scripts/extraction/install.esh` after building the editor. The
effect view covers the esh script. The shell launcher owns temporary-directory
cleanup, external tool execution, hashing and the independent checks.

Lake resolves dependencies from the committed manifest. On dependency updates,
use `MATHLIB_NO_CACHE_ON_UPDATE=1` to avoid automatically downloading all of
mathlib. For a smaller cache download, the upstream cache tool accepts individual
module names; Aeneas uses only part of mathlib. Do not change toolchain pins merely
to make incompatible proof files compile.

`EshRust/Audit.lean` prints axiom dependencies. Generated files include relative
source locations for review. Neither local machine paths, build logs, credentials,
nor extracted `.llbc` artifacts belong in published commits.
