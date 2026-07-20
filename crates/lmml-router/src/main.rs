use clap::Parser;
use lmml_router::{
    apply_upstream_key_specs, parse_upstream_spec, router, RouterAppState, RouterConfig,
};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "lmml-router", about = "Route requests across LMML LAN nodes")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = lmml_router::DEFAULT_ROUTER_PORT)]
    port: u16,
    #[arg(long)]
    router_id: Option<String>,
    #[arg(long)]
    router_name: Option<String>,
    #[arg(long)]
    public_url: Option<String>,
    #[arg(
        long = "upstream",
        env = "LMML_ROUTER_UPSTREAMS",
        value_delimiter = ',',
        required = true
    )]
    upstreams: Vec<String>,
    #[arg(
        long = "upstream-key",
        env = "LMML_ROUTER_UPSTREAM_KEYS",
        value_delimiter = ','
    )]
    upstream_keys: Vec<String>,
    #[arg(
        long = "proxy-timeout-ms",
        env = "LMML_ROUTER_PROXY_TIMEOUT_MS",
        default_value_t = lmml_router::DEFAULT_PROXY_TIMEOUT_MS
    )]
    proxy_timeout_ms: u64,
    #[arg(
        long = "discovery-timeout-ms",
        env = "LMML_ROUTER_DISCOVERY_TIMEOUT_MS",
        default_value_t = lmml_router::DEFAULT_DISCOVERY_TIMEOUT_MS
    )]
    discovery_timeout_ms: u64,
    #[arg(long, env = "LMML_ROUTER_API_KEY")]
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
    let mut upstreams = args
        .upstreams
        .iter()
        .map(|spec| parse_upstream_spec(spec))
        .collect::<Result<Vec<_>, _>>()?;
    apply_upstream_key_specs(&mut upstreams, &args.upstream_keys)?;

    let mut config = RouterConfig {
        host: args.host,
        port: args.port,
        public_url: args.public_url,
        upstreams,
        proxy_timeout_ms: args.proxy_timeout_ms,
        discovery_timeout_ms: args.discovery_timeout_ms,
        api_key: args.api_key,
        allow_unsafe_lan_without_auth: args.unsafe_allow_lan_without_auth,
        ..RouterConfig::default()
    };

    if let Some(router_id) = args.router_id {
        config.router_id = router_id;
    }
    if let Some(router_name) = args.router_name {
        config.router_name = router_name;
    }
    if !args.tags.is_empty() {
        config.tags = args.tags;
    }

    let addr = config.socket_addr()?;
    let state = RouterAppState::new(config)?;
    tracing::info!(addr = %addr, "starting lmml-router API");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(state))
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
