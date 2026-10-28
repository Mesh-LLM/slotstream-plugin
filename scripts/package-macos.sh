#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo 'Requires Apple Silicon macOS' >&2; exit 1; }
stage=$(mktemp -d)
trap 'rm -rf "$stage"; cargo clean' EXIT
# Avoid toolchain rust-objcopy's optional LLVM dependency; sign the full binary.
CARGO_PROFILE_RELEASE_STRIP=none cargo build --locked --release
mkdir -p "$stage/slotstream-plugin" dist
cp target/release/mesh-slotstream "$stage/slotstream-plugin/slotstream-plugin"
cp README.md LICENSE "$stage/slotstream-plugin/"
# Mesh GitHub installer resolves the executable from the repository name.
sed -e 's/name = "slotstream"/name = "slotstream-plugin"/' \
    -e 's/command = "mesh-slotstream"/command = "slotstream-plugin"/' \
    plugin.toml > "$stage/slotstream-plugin/plugin.toml"
codesign --force --sign - "$stage/slotstream-plugin/slotstream-plugin"
codesign --verify --strict "$stage/slotstream-plugin/slotstream-plugin"
archive=slotstream-plugin-aarch64-apple-darwin.tar.gz
tar -czf "dist/$archive" -C "$stage" slotstream-plugin
(cd dist && shasum -a 256 "$archive" > "$archive.sha256" && shasum -a 256 -c "$archive.sha256")
mkdir "$stage/extracted"
tar -xzf "dist/$archive" -C "$stage/extracted"
codesign --verify --strict "$stage/extracted/slotstream-plugin/slotstream-plugin"
"$stage/extracted/slotstream-plugin/slotstream-plugin" --help > /dev/null
# Safe smoke: attach mode must not spawn a backend. Full protocol test runs in CI.
grep -qx 'command = "slotstream-plugin"' "$stage/extracted/slotstream-plugin/plugin.toml"
