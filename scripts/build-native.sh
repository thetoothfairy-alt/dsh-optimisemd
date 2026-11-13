#!/usr/bin/env bash
# Build the native addon and place it under native/<platform>-<arch>.node, the name index.mjs loads.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release -p optimisemd-node
case "$(uname -s)" in
  Linux*)   src=target/release/liboptimisemd_node.so;  ext=.node ;;
  Darwin*)  src=target/release/liboptimisemd_node.dylib; ext=.node ;;
  *)        src=target/release/optimisemd_node.dll; ext=.node ;;
esac
platform=$(uname -s | tr '[:upper:]' '[:lower:]'); [ "$platform" = "linux" ] || true
os=$(uname -s); case "$os" in Linux*) os=linux ;; Darwin*) os=darwin ;; *) os=win32 ;; esac
arch=$(uname -m); case "$arch" in x86_64) arch=x64 ;; aarch64|arm64) arch=arm64 ;; esac
mkdir -p native
cp "$src" "native/dsh-optimisemd.$os-$arch$ext"
echo "installed native/dsh-optimisemd.$os-$arch$ext"
