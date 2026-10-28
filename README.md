# Slotstream plugin for MeshLLM

Use [Slotstream](https://github.com/carloslfu/slotstream) as a MeshLLM inference
backend on an **Apple Silicon Mac**. The plugin starts and supervises Slotstream,
or attaches to a server you already run. Mesh handles discovery and routing.

**Why Slotstream?** It streams selected mixture-of-experts weights from SSD into
a bounded RAM cache, so the whole model need not fit in memory. More cache means
fewer disk reads. SSD speed and prompt length affect performance; this is not
RAM-speed inference. The original tested pack needs roughly **110 GB free SSD**.
Slotstream—not this plugin—implements the inference engine.

## Install

Download the Apple Silicon archive and its `.sha256` from
[Releases](https://github.com/Mesh-LLM/slotstream-plugin/releases), then:

```sh
shasum -a 256 -c slotstream-plugin-aarch64-apple-darwin.tar.gz.sha256
tar xzf slotstream-plugin-aarch64-apple-darwin.tar.gz
```

After a release is published, Mesh's GitHub installer can also install the adapter:

```sh
mesh-llm plugins install Mesh-LLM/slotstream-plugin
```

**Install Slotstream and download its model separately**, following its upstream
instructions (macOS 14+). The package includes only our adapter, not Slotstream or
weights. It never downloads models automatically. Binaries are ad-hoc signed,
**not notarized**; macOS may require approval before running downloaded code.
A checksum verifies integrity, not Apple trust.

## Configure

Add to `~/.mesh-llm/config.toml` (use absolute paths):

```toml
[[plugin]]
name = "slotstream-plugin"
command = "/absolute/path/slotstream-plugin/slotstream-plugin"
args = ["--backend-command", "/absolute/path/to/slotstream", "--", "--memory-gb", "22"]
```

If installed with the Mesh CLI, omit `command`. Installation alone does not
configure the backend. The memory option above was tested with Slotstream 0.2.30;
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

Experimental. Real Slotstream 0.2.30 chat and streaming passed through the managed
plugin with the original Qwen3.8-Flash-Next 4-bit pack. Full Mesh host/remote-peer
routing, tool/vision support and throughput are **not independently validated**.
See [details and reproducible checks](docs/DETAILS.md).

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
checks and smokes the extracted binary, then cleans Cargo output. No models,
Keychain credentials or Apple signing secrets are used.
