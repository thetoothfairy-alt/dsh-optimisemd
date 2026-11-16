#!/usr/bin/env bash
# Build the native addon and place it under native/<platform>-<arch>.node, the name index.mjs loads.
#
# Installs skip the build when the release's prebuilt addon is already there (v0.4.8): `prepare`
# used to force a cargo build on every install, which failed on hosts without Rust even though the
# package shipped the right binary. `--force` (what `pnpm build` passes) rebuilds unconditionally.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

os=$(uname -s); case "$os" in Linux*) os=linux ;; Darwin*) os=darwin ;; *) os=win32 ;; esac
arch=$(uname -m); case "$arch" in x86_64) arch=x64 ;; aarch64|arm64) arch=arm64 ;; esac
dest="native/dsh-optimisemd.$os-$arch.node"

if [ "${1:-}" != "--force" ] && [ -f "$dest" ]; then
  echo "kept $dest (pass --force to rebuild)"
  exit 0
fi

cargo build --release --locked -p optimisemd-node
case "$os" in
  linux)  src=target/release/liboptimisemd_node.so ;;
  darwin) src=target/release/liboptimisemd_node.dylib ;;
  *)      src=target/release/optimisemd_node.dll ;;
esac
[ -f "$src" ] || { echo "missing build output: $src" >&2; exit 1; }
mkdir -p native
cp "$src" "$dest"
echo "installed $dest"
