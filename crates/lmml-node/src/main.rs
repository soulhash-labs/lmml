use std::path::PathBuf;

use clap::Parser;
use lmml_node::{router, NodeAppState, NodeConfig};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "lmml-node", about = "Serve LMML node APIs")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = lmml_node::DEFAULT_NODE_PORT)]
    port: u16,
    #[arg(long)]
    node_id: Option<String>,
    #[arg(long)]
    node_name: Option<String>,
    #[arg(long)]
    public_url: Option<String>,
    #[arg(long = "model-dir")]
    model_dirs: Vec<PathBuf>,
    #[arg(
        long = "llama-url",
        env = "LMML_NODE_LLAMA_URL",
        default_value = lmml_node::DEFAULT_LLAMA_BASE_URL
    )]
    llama_base_url: String,
    #[arg(
        long = "infer-timeout-ms",
        env = "LMML_NODE_INFER_TIMEOUT_MS",
        default_value_t = lmml_node::DEFAULT_INFER_TIMEOUT_MS
    )]
    infer_timeout_ms: u64,
    #[arg(long)]
    enable_server_control: bool,
    #[arg(long, env = "LMML_NODE_API_KEY")]
    api_key: Option<String>,
    #[arg(long)]
    unsafe_allow_lan_without_auth: bool,
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let args = Args::parse();
    let mut config = NodeConfig {
        host: args.host,
        port: args.port,
        public_url: args.public_url,
        llama_base_url: args.llama_base_url,
        infer_timeout_ms: args.infer_timeout_ms,
        enable_server_control: args.enable_server_control,
        api_key: args.api_key,
        allow_unsafe_lan_without_auth: args.unsafe_allow_lan_without_auth,
        ..NodeConfig::default()
    };

    if let Some(node_id) = args.node_id {
        config.node_id = node_id;
    }
    if let Some(node_name) = args.node_name {
        config.node_name = node_name;
    }
    if !args.model_dirs.is_empty() {
        config.model_dirs = args.model_dirs;
    }
    if !args.tags.is_empty() {
        config.tags = args.tags;
    }

    let snapshot = lmml_node::NodeSnapshot::detect(config).await?;
    let addr = snapshot.config.socket_addr()?;
    tracing::info!(addr = %addr, "starting lmml-node API");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(NodeAppState::new(snapshot)))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to install SIGTERM handler");
                let _ignored = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ignored = tokio::signal::ctrl_c().await;
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
