#!/bin/sh
set -eu
esh_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$esh_root/verification"
# Avoid downloading the full upstream mathlib cache on dependency updates.
export MATHLIB_NO_CACHE_ON_UPDATE=1
lake build
