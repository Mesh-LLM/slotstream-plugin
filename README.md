# Slotstream plugin for MeshLLM

Use [Slotstream](https://github.com/carloslfu/slotstream) as a MeshLLM inference
backend on an **Apple Silicon Mac**.

Slotstream runs large mixture-of-experts language models by keeping their weights
on SSD and loading the experts needed for each token into a limited RAM cache.
That lets you run a model whose weights would not all fit in RAM, at the cost of
waiting for disk reads.

This plugin connects that local Slotstream server to MeshLLM. It can start the
server for you or connect to one you already run. **The release includes the connector, Slotstream 0.2.30 and its Metal runtime. Model weights download separately.**

## Install

Download the Apple Silicon archive and its `.sha256` from
[Releases](https://github.com/Mesh-LLM/slotstream-plugin/releases), then:

```sh
shasum -a 256 -c slotstream-plugin-aarch64-apple-darwin.tar.gz.sha256
tar xzf slotstream-plugin-aarch64-apple-darwin.tar.gz
```

Install the adapter with Mesh's GitHub installer:

```sh
mesh-llm plugins install Mesh-LLM/slotstream-plugin
```

The next bundled release needs no separate Slotstream installation. **v0.1.0 is
adapter-only**; until the bundled release is published, use a source/package build
or install Slotstream separately.

Model weights are not bundled. An explicit `--model` choice below authorizes the
upstream resumable, hash-verified download on startup (about 100 GB for the tested
4-bit pack). Cached weights are reused and retained in `~/.slotstream/models`
when Mesh stops or the plugin is removed. An existing model directory skips the
download. Starting without `--model` never initiates a download.

Requires Apple Silicon and macOS 14+. Binaries are signed, **not notarized**;
macOS may require approval. A checksum verifies integrity, not Apple trust.

## Configure

Add to `~/.mesh-llm/config.toml` (use absolute paths):

```toml
[[plugin]]
name = "slotstream-plugin"
command = "/absolute/path/slotstream-plugin/slotstream-plugin"
args = ["--model", "qwen3.8-flash-next:4bit", "--", "--memory-gb", "22"]
```

If installed with the Mesh CLI, omit `command`. Keep the runtime and `mlx.metallib` beside the connector. To use another installation, pass `--backend-command /path/to/slotstream` before `--`. The memory option above was tested with Slotstream 0.2.30;
choose options supported by your installed version and hardware.

Run `mesh-llm serve`, then check `http://127.0.0.1:9337/v1/models` and use an
exact returned model ID with Mesh's OpenAI-compatible chat API.

Alternatively, attach to an existing Slotstream server:

```toml
[[plugin]]
name = "slotstream-plugin"
command = "/absolute/path/slotstream-plugin/slotstream-plugin"
url = "http://127.0.0.1:11434/v1"
```

Attach mode never stops your server. Managed mode stops only its owned child on
normal shutdown; forced termination may leave that child running.

## Status

Experimental. See [tested setup and verification notes](docs/DETAILS.md#verification-and-limits).

## Build and release

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release  # target/release/mesh-slotstream
cargo clean
```

Source builds keep the executable name `mesh-slotstream`; use that path in
`command`. Release packages name it `slotstream-plugin` for Mesh installer
compatibility. The SDK is pinned in Cargo.toml/Cargo.lock. Apache-2.0.

The GitHub workflow tests and packages on PRs, main and manual dispatch. Matching
`vX.Y.Z` tags publish the same tested archive plus SHA-256 checksum to Releases;
manual dispatch produces artifacts only. Maintainers can reproduce packaging on
Apple Silicon with Rust 1.99.0 using `bash scripts/package-macos.sh`. It signs,
checks and smokes the extracted binary, then cleans Cargo output. No model weights, Keychain credentials or Apple signing secrets are used.

Bundled runtime provenance: upstream v0.2.30, source `924442a2100a02caf5e2047dd67234a119e228dc`, archive SHA-256 `e7a3e6532ae0931ea3538abe1974dead0fc741d9d8a529e1f02517b7ac78335c`. Upstream MIT and third-party notices are in `licenses/slotstream/`; original build identity and source archive are retained.

For an explicit download without starting Mesh, run the extracted `slotstream-plugin --pull-model qwen3.8-flash-next:4bit`.
