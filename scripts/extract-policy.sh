#!/bin/sh
# Regenerate into scratch space and compare by default. Use --write deliberately.
set -eu
esh_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
esh_charon=${CHARON_BIN:-$esh_root/.local/tools/aeneas-native-bin/bin/charon}
esh_aeneas=${AENEAS_BIN:-$esh_root/.local/tools/aeneas-native-bin/bin/aeneas}
esh_mode=${1:---check}
case "$esh_mode" in --check|--write) ;; *) printf '%s\n' 'Usage: extract-policy.sh [--check|--write]' >&2; exit 2;; esac
mkdir -p "$esh_root/.local"
esh_scratch=$(mktemp -d "$esh_root/.local/extract.XXXXXX")
trap 'rm -rf "$esh_scratch"' EXIT HUP INT TERM
(
  cd "$esh_root/crates/esh-policy"
  "$esh_charon" cargo --preset=aeneas --dest-file "$esh_scratch/policy.llbc" -- --target-dir "$esh_root/.local/charon-target"
)
cd "$esh_root"
"$esh_aeneas" -backend lean "$esh_scratch/policy.llbc" -dest "$esh_scratch" -subdir EshRust/Generated -split-files -namespace EshRust
if [ "$esh_mode" = --write ]; then
  cp "$esh_scratch/EshRust/Generated/Types.lean" verification/EshRust/Generated/Types.lean
  cp "$esh_scratch/EshRust/Generated/Funs.lean" verification/EshRust/Generated/Funs.lean
  shasum -a 256 crates/esh-policy/src/lib.rs crates/esh-policy/Cargo.toml \
    verification/toolchain-pins.env verification/EshRust/Generated/Types.lean \
    verification/EshRust/Generated/Funs.lean > verification/extraction.sha256
else
  diff -ru verification/EshRust/Generated "$esh_scratch/EshRust/Generated"
  shasum -a 256 -c verification/extraction.sha256
fi
