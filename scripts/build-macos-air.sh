#!/usr/bin/env bash
# build-macos-air.sh — build the darwin-arm64 addon on the owner's MacBook Air, the only Mac this workspace can reach.
#
#   bash scripts/build-macos-air.sh            # build the tag at HEAD
#   bash scripts/build-macos-air.sh v0.1.0     # build a specific tag
#
# The release workflow cannot use a macOS runner on this account's plan, so the macOS artefact is built here and attached
# to the GitHub release by hand (or by scripts/attach-macos-asset.sh, which needs a token). The Air has Xcode and rustup
# installed; no Homebrew and no Node are needed (the addon build is cargo-only).
set -euo pipefail
cd "$(dirname "$0")/.."
TAG="${1:-$(git describe --tags --abbrev=0 2>/dev/null || echo main)}"
HOST="${AIR_HOST:-macbook-air}"
DEST="${2:-native/dsh-optimisemd.darwin-arm64.node}"
# Build paths end up in panic/debug strings. Remap the Air's home (and the cargo registry under it) to /build so the
# published addon does not carry the owner's macOS username.
ssh "$HOST" "rm -rf ~/dsh-optimisemd-build && git clone -q --depth 1 --branch '$TAG' https://github.com/thetoothfairy-alt/dsh-optimisemd.git ~/dsh-optimisemd-build && cd ~/dsh-optimisemd-build && RUSTFLAGS=\"--remap-path-prefix=\$HOME=/build\" bash scripts/build-native.sh"
mkdir -p "$(dirname "$DEST")"
scp -q "$HOST:~/dsh-optimisemd-build/native/dsh-optimisemd.darwin-arm64.node" "$DEST"
file "$DEST"
