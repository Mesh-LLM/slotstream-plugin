# Slotstream for MeshLLM

Publish a [Slotstream](https://github.com/carloslfu/slotstream) server's models to
MeshLLM, without copying inference traffic through plugin IPC or modifying mesh core.
Slotstream runs on Apple Silicon/macOS 14+; install it and download its model
**separately** before using managed mode. The plugin does not run installers,
`pull`, `launch`, or `stop`, and starts the backend with closed stdin.

## Features

- **Managed or attached runtime:** launch and supervise one owned `slotstream serve`
  child, or attach to an existing OpenAI-compatible Slotstream endpoint.
- **Mesh integration:** register the backend with the Mesh plugin protocol; Mesh
  handles model discovery, endpoint health and inference routing. Requests go to
  the backend HTTP API, not through plugin IPC.
- **Explicit resource control:** pass supported memory/context options to the
  runtime. No automatic installers, model downloads, or changes to mesh core.
- **Safe ownership boundaries:** attach mode never stops your server; managed
  shutdown only stops the child this plugin created.
- **Chat and streaming smoke-tested:** real Slotstream 0.2.30 with the original
  Qwen3.8-Flash-Next 4-bit pack. Tool and vision support remain upstream features,
  not independently validated adapter guarantees.

## How SSD-backed inference works

**Slotstream does the inference and SSD streaming; this plugin is the control-plane
adapter, not a new model engine.** Slotstream is a native Swift runtime using
Apple MLX and Metal. Its Qwen3.8-Flash-Next mixture-of-experts model selects only
some experts for each token, so the entire roughly 105 GB model need not reside
in unified memory at once.

As described in [upstream engineering documentation](https://github.com/carloslfu/slotstream/blob/main/docs/ENGINEERING.md#how-it-works),
the original model contains about 68 GB of routed experts, a 32 GB n-gram lookup
table and a 3.8 GB shared component. Slotstream keeps the shared component in RAM
and reads experts from SSD into a bounded cache-slot pool shared across 48 layers.
More RAM can retain more experts and reduce SSD reads; expert lookahead can
prefetch upcoming work. This trades SSD bandwidth and latency for a smaller
resident weight footprint—it does not make disk as fast as RAM or eliminate
KV-cache/context memory requirements.

Upstream currently lists Apple Silicon, macOS 14+, at least 16 GB RAM and about
110 GB free SSD space for the original pack. Check its
[hardware guide](https://github.com/carloslfu/slotstream/blob/main/docs/HARDWARE.md)
for current requirements. Model packs, memory budgets, prompt length, filesystem
cache and SSD speed affect performance. This repository does **not** claim a
throughput benchmark; the validated 22 GB budget below is a smoke configuration,
not a recommendation for every Mac. No weights or upstream binaries are bundled.

## Build and configure

```sh
cargo build --release
# Put target/release/mesh-slotstream on PATH, or set command to its absolute path.
```

In your MeshLLM config:

```toml
[[plugin]]
name = "slotstream"
command = "/absolute/path/to/mesh-slotstream"
args = ["--backend-command", "/absolute/path/to/slotstream", "--", "--memory-gb", "22"]
```

The plugin starts `slotstream serve --port <allocated-loopback-port>` followed
by your serve options. Do not include `serve` or `--port` yourself. Pass options
such as `--model smart`, `--max-context 32768`, or `--memory-limit-gb 22` only
when supported by your installed Slotstream version. No shell interpolation.
The plugin supervises only its own child and kills that child on normal plugin
exit. Forced termination of the plugin process cannot guarantee child cleanup.
Ephemeral port selection has a small bind race; a collision is a startup failure,
not permission to stop another process.

Alternatively, attach a server you already started:

```toml
[[plugin]]
name = "slotstream"
command = "/absolute/path/to/mesh-slotstream"
url = "http://127.0.0.1:11434/v1"
```

`url` is passed by MeshLLM through `MESH_LLM_PLUGIN_URL`; `--url` also works.
Attach mode never starts or stops the server. URL and managed command are mutually
exclusive. URLs must end in `/v1`, without credentials, query, or fragment.

Mesh owns endpoint health checks, model discovery, and the OpenAI data plane.
The managed plugin also observes `/v1/models` and child liveness; a loading
endpoint is not inference-ready merely because plugin initialization succeeded.
Use an exact model from mesh's `/v1/models`; do not assume the upstream alias.
Slotstream currently serializes generation. Large prompts can have long prefill.
Streaming/tool/vision capability and API limits are those of your Slotstream
version, not new promises made by this adapter.

## Verification and limits

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo clean
```

Tests cover manifest ownership, launch arguments, URL validation, attachment,
missing binaries, one-child startup and owned-child teardown using a harmless
local process fixture. They do **not** prove real model inference, mesh peer
routing, or Slotstream performance. No upstream binary or weights are bundled.

Protocol SDK is pinned to MeshLLM commit
`d9c6e5a2ff3143c2c9161236bf24d415c403b9ca`.

## Sources

- [Upstream CLI](https://github.com/carloslfu/slotstream/blob/main/docs/CLI.md): `serve --port N`, memory options.
- [Upstream API](https://github.com/carloslfu/slotstream/blob/main/docs/API.md): `/v1/models`, chat streaming and tools.
- [Mesh plugin contract](https://github.com/Mesh-LLM/mesh-llm/blob/d9c6e5a2ff3143c2c9161236bf24d415c403b9ca/mesh/docs/plugins/README.md): inference endpoint registration.
- Initial adapter structure derives from Mesh-LLM/flash-moe (Apache-2.0).

### Opt-in real-runtime check

With Slotstream and its model already installed:

```sh
SLOTSTREAM_E2E_BINARY=/absolute/path/to/slotstream \
SLOTSTREAM_E2E_MODEL_DIR=/absolute/path/to/downloaded/model \
cargo test --test protocol -- --ignored --nocapture
```

This launches the real plugin executable and speaks the Mesh plugin protocol over
an isolated Unix socket, initializes its managed endpoint, waits for model discovery,
checks nonempty chat and streaming answers, then verifies shutdown removes the owned
endpoint. It uses a 22 GB budget and a 32K context window. It does not launch a full
MeshLLM host or prove cross-peer routing.

Validated with upstream release **0.2.30**, release archive SHA-256 verified,
and its hash-verified original `qwen3.8-flash-next:4bit` pack. Both normal and
streaming requests to say hello returned `hello`; the managed runtime loaded in
2.1 seconds in that run. These are smoke results, **not throughput benchmarks**.
The earlier statement that tests do not prove real inference refers to the default,
weights-free suite; the opt-in test above was run separately against real weights.
