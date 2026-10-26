//! Real plugin executable, socket transport and endpoint registration.
//! Set SLOTSTREAM_E2E_BINARY and SLOTSTREAM_E2E_MODEL_DIR for real inference.
use anyhow::{Context, Result, bail};
use mesh_llm_plugin::{LocalStream, PROTOCOL_VERSION, proto, read_envelope, write_envelope};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{net::UnixListener, process::Command, time::timeout};

async fn exchange(
    stream: &mut LocalStream,
    id: u64,
    payload: proto::envelope::Payload,
) -> Result<proto::envelope::Payload> {
    write_envelope(
        stream,
        &proto::Envelope {
            protocol_version: PROTOCOL_VERSION,
            plugin_id: "slotstream".into(),
            request_id: id,
            payload: Some(payload),
        },
    )
    .await?;
    let response = timeout(Duration::from_secs(10), read_envelope(stream)).await??;
    assert_eq!(response.request_id, id);
    response.payload.context("response payload")
}

async fn initialize(stream: &mut LocalStream) -> Result<String> {
    let response = exchange(
        stream,
        1,
        proto::envelope::Payload::InitializeRequest(proto::InitializeRequest {
            host_protocol_version: PROTOCOL_VERSION,
            host_version: "slotstream-protocol-test".into(),
            ..Default::default()
        }),
    )
    .await?;
    let proto::envelope::Payload::InitializeResponse(response) = response else {
        bail!("unexpected init response: {response:?}");
    };
    assert_eq!(response.plugin_protocol_version, PROTOCOL_VERSION);
    let manifest = response.manifest.context("manifest")?;
    assert_eq!(manifest.endpoints.len(), 1);
    manifest.endpoints[0]
        .address
        .clone()
        .context("endpoint address")
}

fn plugin_command(socket: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mesh-slotstream"));
    command
        .env("MESH_LLM_PLUGIN_ENDPOINT", socket)
        .env("MESH_LLM_PLUGIN_TRANSPORT", "unix")
        .env_remove("MESH_LLM_PLUGIN_URL")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    command
}

#[tokio::test]
async fn executable_handshake_and_clean_shutdown() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let socket = dir.path().join("host.sock");
    let listener = UnixListener::bind(&socket)?;
    let mut child = plugin_command(&socket)
        .args(["--url", "http://127.0.0.1:11434/v1"])
        .spawn()?;
    let (stream, _) = timeout(Duration::from_secs(10), listener.accept()).await??;
    let mut stream = LocalStream::Unix(stream);
    assert_eq!(initialize(&mut stream).await?, "http://127.0.0.1:11434/v1");
    let health = exchange(
        &mut stream,
        2,
        proto::envelope::Payload::HealthRequest(proto::HealthRequest {}),
    )
    .await?;
    assert!(matches!(
        health,
        proto::envelope::Payload::HealthResponse(_)
    ));
    let response = exchange(
        &mut stream,
        3,
        proto::envelope::Payload::ShutdownRequest(proto::ShutdownRequest {
            reason: "test complete".into(),
        }),
    )
    .await?;
    assert!(matches!(
        response,
        proto::envelope::Payload::ShutdownResponse(_)
    ));
    drop(stream);
    assert!(
        timeout(Duration::from_secs(10), child.wait())
            .await??
            .success()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires separately installed Slotstream and downloaded model; see README"]
async fn real_slotstream_managed_inference() -> Result<()> {
    let binary = std::env::var("SLOTSTREAM_E2E_BINARY").context("SLOTSTREAM_E2E_BINARY")?;
    let model = std::env::var("SLOTSTREAM_E2E_MODEL_DIR").context("SLOTSTREAM_E2E_MODEL_DIR")?;
    let dir = tempfile::tempdir()?;
    let socket = dir.path().join("host.sock");
    let listener = UnixListener::bind(&socket)?;
    let mut child = plugin_command(&socket)
        .args([
            "--backend-command",
            &binary,
            "--",
            "--model",
            &model,
            "--memory-gb",
            "22",
            "--max-context",
            "32768",
        ])
        .spawn()?;
    let (stream, _) = timeout(Duration::from_secs(10), listener.accept()).await??;
    let mut stream = LocalStream::Unix(stream);
    let base = initialize(&mut stream).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()?;
    let model_id = timeout(Duration::from_secs(240), async {
        loop {
            if let Ok(response) = client.get(format!("{base}/models")).send().await
                && response.status().is_success()
                && let Ok(value) = response.json::<serde_json::Value>().await
                && let Some(id) = value.pointer("/data/0/id").and_then(|id| id.as_str())
            {
                break id.to_string();
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
    .await
    .context("model readiness")?;
    let request = serde_json::json!({"model":model_id, "messages":[{"role":"user","content":"Reply with only the word hello."}], "max_tokens":64, "temperature":0, "stream":false});
    let response = client
        .post(format!("{base}/chat/completions"))
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;
    let content = response
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(!content.trim().is_empty(), "empty answer: {response}");
    eprintln!("REAL_SLOTSTREAM model={model_id} response={response}");
    let mut streaming = request.clone();
    streaming["stream"] = serde_json::json!(true);
    let body = client
        .post(format!("{base}/chat/completions"))
        .json(&streaming)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    assert!(
        body.contains("data: [DONE]"),
        "SSE missing terminal marker: {body}"
    );
    let mut answer = String::new();
    for line in body.lines().filter_map(|line| line.strip_prefix("data: ")) {
        if line == "[DONE]" {
            continue;
        }
        let chunk: serde_json::Value = serde_json::from_str(line)?;
        if let Some(content) = chunk
            .pointer("/choices/0/delta/content")
            .and_then(|v| v.as_str())
        {
            answer.push_str(content);
        }
    }
    assert!(!answer.trim().is_empty(), "streaming answer empty: {body}");
    eprintln!("REAL_SLOTSTREAM streaming_answer={answer}");

    let response = exchange(
        &mut stream,
        2,
        proto::envelope::Payload::ShutdownRequest(proto::ShutdownRequest {
            reason: "real inference complete".into(),
        }),
    )
    .await?;
    assert!(matches!(
        response,
        proto::envelope::Payload::ShutdownResponse(_)
    ));
    drop(stream);
    assert!(
        timeout(Duration::from_secs(10), child.wait())
            .await??
            .success()
    );
    // The exact owned endpoint must become unreachable after plugin shutdown.
    timeout(Duration::from_secs(10), async {
        loop {
            if client.get(format!("{base}/models")).send().await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("managed endpoint teardown")?;
    Ok(())
}
