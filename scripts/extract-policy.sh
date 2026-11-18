#!/bin/sh
# Keep external tools in the launcher and grant esh only its staging and output roots.
set -eu
esh_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
esh_charon=${CHARON_BIN:-$esh_root/.local/tools/aeneas-native-bin/bin/charon}
esh_aeneas=${AENEAS_BIN:-$esh_root/.local/tools/aeneas-native-bin/bin/aeneas}
esh_mode=${1:---check}
usage() {
  printf '%s\n' 'Usage: extract-policy.sh [--check|--write|--check-fingerprint]' >&2
  exit 2
}
[ "$#" -le 1 ] || usage
case "$esh_mode" in --check|--write|--check-fingerprint) ;; *) usage;; esac
mkdir -p "$esh_root/.local"
esh_scratch=$(mktemp -d "$esh_root/.local/extract.XXXXXX")
trap 'rm -rf "$esh_scratch"' EXIT HUP INT TERM
esh_staged="$esh_scratch/verification"
mkdir "$esh_staged"
cd "$esh_root"

# Mirror the repository-relative paths in scratch so shasum records the same
# filenames whether hashing regenerated files or the current checkout.
esh_generated_root=$esh_root
if [ "$esh_mode" != --check-fingerprint ]; then
  (
    cd crates/esh-policy
    "$esh_charon" cargo --preset=aeneas --dest-file "$esh_scratch/policy.llbc" -- --target-dir "$esh_root/.local/charon-target"
  )
  "$esh_aeneas" -backend lean "$esh_scratch/policy.llbc" -dest "$esh_staged" -subdir EshRust/Generated -split-files -namespace EshRust
  esh_generated_root=$esh_scratch
fi

if [ "$esh_mode" = --check ]; then
  diff -ru verification/EshRust/Generated "$esh_staged/EshRust/Generated"
fi
(
  shasum -a 256 crates/esh-policy/src/lib.rs crates/esh-policy/Cargo.toml verification/toolchain-pins.env
  cd "$esh_generated_root"
  shasum -a 256 verification/EshRust/Generated/Types.lean verification/EshRust/Generated/Funs.lean
) > "$esh_staged/extraction.sha256"

run_esh() {
  cargo run --quiet --locked -p esh-cli -- run "scripts/extraction/$1.esh" \
    --grant staged read "$esh_staged" --grant tracked "$2" verification
}
if [ "$esh_mode" = --write ]; then
  run_esh install write
fi
run_esh check-fingerprint read
# Retain an independent check of the standard checksum manifest after every mode.
shasum -a 256 -c verification/extraction.sha256
