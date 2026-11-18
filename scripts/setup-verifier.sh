#!/bin/sh
# Build the pinned official tools with native-only packaging. Verifier source is unchanged.
set -eu
esh_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$esh_root/verification/toolchain-pins.env"
mkdir -p "$esh_root/.local/tools"
esh_tools="$esh_root/.local/tools"
fetch() {
  esh_repo=$1 esh_rev=$2 esh_dest=$3
  if [ ! -d "$esh_dest" ]; then
    git clone --filter=blob:none --no-checkout "$esh_repo" "$esh_dest"
    git -C "$esh_dest" checkout --detach "$esh_rev"
  fi
  test "$(git -C "$esh_dest" rev-parse HEAD)" = "$esh_rev"
  test -z "$(git -C "$esh_dest" status --porcelain --untracked-files=no)"
}
fetch https://github.com/AeneasVerif/aeneas.git "$ESH_AENEAS_REV" "$esh_tools/aeneas"
fetch https://github.com/AeneasVerif/charon.git "$ESH_CHARON_REV" "$esh_tools/charon-source"
esh_native=$(mktemp -d "$esh_tools/charon-packaging.XXXXXX")
trap 'rm -rf "$esh_native"' EXIT HUP INT TERM
git -C "$esh_tools/charon-source" archive HEAD | tar -x -C "$esh_native"
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) esh_target=aarch64-apple-darwin;;
  Darwin:x86_64) esh_target=x86_64-apple-darwin;;
  Linux:x86_64) esh_target=x86_64-unknown-linux-gnu;;
  Linux:aarch64) esh_target=aarch64-unknown-linux-gnu;;
  *) printf '%s\n' 'No native packaging recipe for this platform.' >&2; exit 2;;
esac
cat > "$esh_native/charon/rust-toolchain" <<EOF
[toolchain]
channel = "$ESH_EXTRACTION_RUST"
profile = "minimal"
components = ["rustc-dev", "llvm-tools", "rust-src", "miri"]
targets = ["$esh_target"]
EOF
# Cross-target integration tests require the discarded toolchains. Retain library
# tests and generated-AST checks; record this reduced upstream test scope explicitly.
sed 's/cargo test --profile release --locked/cargo test --lib --profile release --locked/' \
  "$esh_native/nix/charon.nix" > "$esh_native/nix/charon.nix.new"
mv "$esh_native/nix/charon.nix.new" "$esh_native/nix/charon.nix"
cd "$esh_tools/aeneas"
nix --extra-experimental-features 'nix-command flakes' build .#aeneas \
  --override-input charon "path:$esh_native" --no-write-lock-file \
  --out-link "$esh_tools/aeneas-native-bin" --max-jobs 2
