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
scripts/extract-policy.sh --check
scripts/check-proofs.sh
```

`--check` generates into a disposable directory and compares both generated Lean
files byte-for-byte. It does not alter the proof inputs. The extraction fingerprint
records the Rust source, policy manifest, tool pins, and generated files. Regular
CI checks that fingerprint and replays Lean proofs; full tool regeneration is
performed by the extraction command. After an intentional
policy change, run `scripts/extract-policy.sh --write`, inspect the generated
diff, and update proofs as needed. Extraction without a successful proof build
is not verification.

Lake resolves dependencies from the committed manifest. On dependency updates,
use `MATHLIB_NO_CACHE_ON_UPDATE=1` to avoid automatically downloading all of
mathlib. For a smaller cache download, the upstream cache tool accepts individual
module names; Aeneas uses only part of mathlib. Do not change toolchain pins merely
to make incompatible proof files compile.

`EshRust/Audit.lean` prints axiom dependencies. Generated files include relative
source locations for review. Neither local machine paths, build logs, credentials,
nor extracted `.llbc` artifacts belong in published commits.
