use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use mesh_llm_plugin::{
    PluginMetadata, PluginRuntime, PluginStartupPolicy, capability, plugin_server_info,
};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PLUGIN_ID: &str = "slotstream";
const ENDPOINT_ID: &str = "slotstream";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(1);
const INSTALL_HINT: &str = "Install Slotstream separately and pass \
                           `--backend-command <path-to-slotstream> -- <backend args...>`, \
                           or configure `url` for an already-running Slotstream /v1 endpoint.";

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "MESH_LLM_PLUGIN_URL")]
    url: Option<String>,
    #[arg(long)]
    backend_command: Option<String>,
    #[arg(last = true)]
    backend_args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlotstreamSource {
    Managed {
        command: String,
        args: Vec<String>,
        port: u16,
    },
    External {
        base_url: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlotstreamConfig {
    source: SlotstreamSource,
}

impl SlotstreamConfig {
    fn from_args(args: Args) -> Result<Self> {
        let command = args
            .backend_command
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let url = args
            .url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if command.is_some() && url.is_some() {
            bail!("slotstream plugin accepts either --backend-command or --url, not both");
        }
        if url.is_some() && !args.backend_args.is_empty() {
            bail!("backend args cannot be used with --url");
        }
        if let Some(base_url) = url {
            return Ok(Self {
                source: SlotstreamSource::External {
                    base_url: normalize_base_url(&base_url)?,
                },
            });
        }
        let command = command.with_context(|| {
            format!("slotstream plugin requires --backend-command or --url. {INSTALL_HINT}")
        })?;
        managed_command_args(&args.backend_args, 1)?;
        let port = allocate_local_port().context("allocate slotstream endpoint port")?;
        Ok(Self {
            source: SlotstreamSource::Managed {
                command,
                args: args.backend_args,
                port,
            },
        })
    }

    fn endpoint_base_url(&self) -> String {
        match &self.source {
            SlotstreamSource::Managed { port, .. } => format!("http://127.0.0.1:{port}/v1"),
            SlotstreamSource::External { base_url } => base_url.clone(),
        }
    }

    fn managed(&self) -> bool {
        matches!(self.source, SlotstreamSource::Managed { .. })
    }
}

#[derive(Clone)]
struct SlotstreamState {
    config: SlotstreamConfig,
    child: Arc<Mutex<Option<Child>>>,
}

impl SlotstreamState {
    fn new(config: SlotstreamConfig) -> Self {
        Self {
            config,
            child: Arc::new(Mutex::new(None)),
        }
    }

    async fn ensure_started(&self) -> Result<()> {
        let SlotstreamSource::Managed {
            command,
            args,
            port,
        } = &self.config.source
        else {
            return Ok(());
        };

        let mut child = self.child.lock().await;
        if let Some(existing) = child.as_mut() {
            match existing.try_wait().context("poll slotstream process")? {
                Some(status) => bail!("slotstream exited before readiness: {status}"),
                None => return Ok(()),
            }
        }

        let managed_args = managed_command_args(args, *port)?;
        let mut command_builder = Command::new(command);
        command_builder.args(&managed_args);
        command_builder.stdin(std::process::Stdio::null());
        command_builder.stdout(std::process::Stdio::null());
        command_builder.stderr(std::process::Stdio::inherit());
        command_builder.kill_on_drop(true);
        let spawned = command_builder
            .spawn()
            .with_context(|| format!("launch slotstream backend via {command}. {INSTALL_HINT}"))?;
        *child = Some(spawned);
        Ok(())
    }

    async fn health(&self) -> Result<String> {
        match &self.config.source {
            SlotstreamSource::External { base_url } => Ok(format!("external_url={base_url}")),
            SlotstreamSource::Managed { port, .. } => {
                let mut child = self.child.lock().await;
                match child.as_mut() {
                    Some(process) => {
                        if let Some(status) =
                            process.try_wait().context("poll slotstream process")?
                        {
                            *child = None;
                            bail!("slotstream exited with {status}");
                        }
                    }
                    None => return Ok(format!("starting endpoint=http://127.0.0.1:{port}/v1")),
                }
                drop(child);

                let health_url = format!("http://127.0.0.1:{port}/v1/models");
                match reqwest::Client::builder()
                    .timeout(HEALTH_TIMEOUT)
                    .build()
                    .context("build slotstream health client")?
                    .get(&health_url)
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => {
                        Ok(format!("endpoint=http://127.0.0.1:{port}/v1"))
                    }
                    Ok(response) => Ok(format!(
                        "starting endpoint=http://127.0.0.1:{port}/v1 health_status={}",
                        response.status()
                    )),
                    Err(error) => Ok(format!(
                        "starting endpoint=http://127.0.0.1:{port}/v1 detail={error}"
                    )),
                }
            }
        }
    }
}

fn normalize_base_url(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(value).context("parse endpoint URL")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("endpoint must be an HTTP(S) URL without credentials, query or fragment");
    }
    if url.path() != "/v1" {
        bail!("endpoint URL must end in /v1");
    }
    Ok(value.to_string())
}

fn allocate_local_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn managed_command_args(user_args: &[String], port: u16) -> Result<Vec<String>> {
    if user_args
        .iter()
        .any(|arg| arg == "serve" || arg == "--port" || arg.starts_with("--port="))
    {
        bail!("plugin owns the serve subcommand and --port; pass only serve options");
    }
    let mut args = vec!["serve".to_string(), "--port".to_string(), port.to_string()];
    args.extend_from_slice(user_args);
    Ok(args)
}

fn build_plugin_from_config(
    name: String,
    config: SlotstreamConfig,
) -> mesh_llm_plugin::SimplePlugin {
    let endpoint_base = config.endpoint_base_url();
    let state = SlotstreamState::new(config);
    let startup_state = state.clone();
    let health_state = state.clone();
    let inference_endpoint = if state.config.managed() {
        mesh_llm_plugin::inference::provider(ENDPOINT_ID, endpoint_base.clone())
    } else {
        mesh_llm_plugin::inference::openai_http(ENDPOINT_ID, endpoint_base.clone())
    };

    mesh_llm_plugin::plugin! {
        metadata: PluginMetadata::new(
            name,
            VERSION,
            plugin_server_info(
                "mesh-slotstream",
                VERSION,
                "Slotstream SSD Expert Streaming Provider",
                "Registers a slotstream OpenAI-compatible endpoint for single-node SSD expert streaming.",
                Some(
                    "Configure [[plugin]] name = \"slotstream\" with url for an \
                     already-running endpoint, or plugin args for a managed \
                     Slotstream process. Slotstream itself is installed separately.",
                ),
            ),
        ),
        startup_policy: PluginStartupPolicy::Any,
        provides: [
            capability("endpoint:inference"),
            capability("endpoint:inference/openai_compatible"),
            capability("backend:slotstream"),
            capability("backend:ssd-expert-streaming"),
        ],
        inference: [
            inference_endpoint,
        ],
        health: move |_context| {
            let health_state = health_state.clone();
            Box::pin(async move { health_state.health().await })
        },
        on_initialized: move |_context| {
            let startup_state = startup_state.clone();
            Box::pin(async move { startup_state.ensure_started().await })
        },
    }
}

fn build_plugin(name: String, args: Args) -> Result<mesh_llm_plugin::SimplePlugin> {
    let config = SlotstreamConfig::from_args(args)?;
    Ok(build_plugin_from_config(name, config))
}

async fn run_plugin(name: String, args: Args) -> Result<()> {
    PluginRuntime::run(build_plugin(name, args)?).await
}

pub fn run_main() -> i32 {
    let args = Args::parse();
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    let runtime = builder.build().expect("build tokio runtime");
    runtime.block_on(async move {
        match run_plugin(DEFAULT_PLUGIN_ID.to_string(), args).await {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{err:#}");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                1
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use mesh_llm_plugin::Plugin;

    use super::*;

    fn args(url: Option<&str>, backend_command: Option<&str>, backend_args: &[&str]) -> Args {
        Args {
            url: url.map(str::to_string),
            backend_command: backend_command.map(str::to_string),
            backend_args: backend_args.iter().map(|arg| (*arg).to_string()).collect(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owned_child_is_started_once_and_killed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("slotstream-fixture");
        std::fs::write(&script, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let state = SlotstreamState::new(SlotstreamConfig {
            source: SlotstreamSource::Managed {
                command: script.to_string_lossy().into_owned(),
                args: vec![],
                port: 8123,
            },
        });
        state.ensure_started().await.unwrap();
        let pid = state.child.lock().await.as_ref().unwrap().id().unwrap();
        state.ensure_started().await.unwrap();
        assert_eq!(state.child.lock().await.as_ref().unwrap().id(), Some(pid));
        drop(state);
        // Bounded local execution check: only observe the fixture PID, never signal it.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !Command::new("/bin/kill")
                    .args(["-0", &pid.to_string()])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await
                    .unwrap()
                    .success()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("owned child must exit on plugin drop");
    }

    #[tokio::test]
    async fn attached_endpoint_never_spawns_a_process() {
        let state = SlotstreamState::new(SlotstreamConfig {
            source: SlotstreamSource::External {
                base_url: "http://localhost:11434/v1".into(),
            },
        });
        state.ensure_started().await.unwrap();
        assert!(state.child.lock().await.is_none());
    }

    #[test]
    fn managed_command_uses_slotstream_serve_contract() {
        assert_eq!(
            managed_command_args(&["--memory-gb".into(), "22".into()], 8123).unwrap(),
            ["serve", "--port", "8123", "--memory-gb", "22"]
        );
        for arg in ["--port", "--port=9000", "serve"] {
            assert!(managed_command_args(&[arg.into()], 8123).is_err());
        }
    }

    #[test]
    fn invalid_urls_and_unused_backend_args_are_rejected() {
        for url in [
            "file:///v1",
            "http://a/v1?q=x",
            "http://u:p@a/v1",
            "http://a",
            "http://a/v1#x",
        ] {
            assert!(SlotstreamConfig::from_args(args(Some(url), None, &[])).is_err());
        }
        assert!(
            SlotstreamConfig::from_args(args(
                Some("http://localhost:11434/v1"),
                None,
                &["--think"]
            ))
            .is_err()
        );
    }

    #[test]
    fn config_accepts_external_url_mode() {
        let config =
            SlotstreamConfig::from_args(args(Some(" http://127.0.0.1:8000/v1/ "), None, &[]))
                .unwrap();

        assert_eq!(
            config.source,
            SlotstreamSource::External {
                base_url: "http://127.0.0.1:8000/v1".to_string()
            }
        );
        assert_eq!(config.endpoint_base_url(), "http://127.0.0.1:8000/v1");
    }

    #[test]
    fn config_rejects_command_and_url_together() {
        let err = SlotstreamConfig::from_args(args(
            Some("http://127.0.0.1:8000/v1"),
            Some("/opt/slotstream/infer"),
            &[],
        ))
        .expect_err("command and url must be mutually exclusive");
        assert!(err.to_string().contains("not both"));
    }

    #[test]
    fn config_missing_source_explains_external_install_boundary() {
        let err = SlotstreamConfig::from_args(args(None, None, &[]))
            .expect_err("slotstream requires a managed command or attached endpoint");
        let message = err.to_string();

        assert!(message.contains("Install Slotstream separately"));
        assert!(message.contains("--backend-command"));
        assert!(message.contains("--url"));
    }

    #[tokio::test]
    async fn managed_start_failure_explains_external_install_boundary() {
        let state = SlotstreamState::new(SlotstreamConfig {
            source: SlotstreamSource::Managed {
                command: "/definitely/missing/slotstream/infer".to_string(),
                args: Vec::new(),
                port: 8123,
            },
        });

        let message = state
            .ensure_started()
            .await
            .expect_err("missing external slotstream binary must fail")
            .to_string();

        assert!(message.contains("Install Slotstream separately"));
        assert!(message.contains("/definitely/missing/slotstream/infer"));
    }

    #[test]
    fn managed_manifest_declares_plugin_owned_openai_endpoint() {
        let plugin = build_plugin_from_config(
            "slotstream".to_string(),
            SlotstreamConfig {
                source: SlotstreamSource::Managed {
                    command: "/opt/slotstream/infer".to_string(),
                    args: vec!["--model".to_string(), "/models/qwen3.5".to_string()],
                    port: 8123,
                },
            },
        );
        let manifest = plugin.manifest().expect("manifest");
        let endpoint = manifest.endpoints.first().expect("endpoint");

        assert_eq!(endpoint.endpoint_id, ENDPOINT_ID);
        assert_eq!(
            endpoint.address.as_deref(),
            Some("http://127.0.0.1:8123/v1")
        );
        assert_eq!(endpoint.protocol.as_deref(), Some("openai_compatible"));
        assert!(endpoint.supports_streaming);
        assert!(endpoint.managed_by_plugin);
        assert!(
            manifest
                .capabilities
                .iter()
                .any(|capability| capability == "backend:ssd-expert-streaming")
        );
    }

    #[test]
    fn external_manifest_declares_attached_openai_endpoint() {
        let plugin = build_plugin_from_config(
            "slotstream".to_string(),
            SlotstreamConfig {
                source: SlotstreamSource::External {
                    base_url: "http://127.0.0.1:8000/v1".to_string(),
                },
            },
        );
        let manifest = plugin.manifest().expect("manifest");
        let endpoint = manifest.endpoints.first().expect("endpoint");

        assert_eq!(
            endpoint.address.as_deref(),
            Some("http://127.0.0.1:8000/v1")
        );
        assert!(!endpoint.managed_by_plugin);
    }
}
