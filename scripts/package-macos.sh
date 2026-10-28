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
# Pin the previously inference-tested upstream release, including Metal resources.
curl --fail --location --silent --show-error --connect-timeout 20 --max-time 180 \
    https://github.com/carloslfu/slotstream/releases/download/v0.2.30/slotstream-arm64.tar.gz \
    -o "$stage/upstream.tar.gz"
echo "e7a3e6532ae0931ea3538abe1974dead0fc741d9d8a529e1f02517b7ac78335c  $stage/upstream.tar.gz" | shasum -a 256 -c -
mkdir "$stage/upstream"
tar -xzf "$stage/upstream.tar.gz" -C "$stage/upstream"
for resource in slotstream mlx.metallib build-identity.json build-source.tar.gz; do
    cp "$stage/upstream/$resource" "$stage/slotstream-plugin/"
done
cp -R licenses "$stage/slotstream-plugin/"
codesign --verify --strict "$stage/slotstream-plugin/slotstream"
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
"$stage/extracted/slotstream-plugin/slotstream" --version
"$stage/extracted/slotstream-plugin/slotstream" pull --help > /dev/null
"$stage/extracted/slotstream-plugin/slotstream" doctor --memory-gb 22 > /dev/null
# Safe smoke: attach mode must not spawn a backend. Full protocol test runs in CI.
grep -qx 'command = "slotstream-plugin"' "$stage/extracted/slotstream-plugin/plugin.toml"
