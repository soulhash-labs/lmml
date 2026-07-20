//! LAN advertisement support for `lmml-node`.
//!
//! Nodes can opt into a lightweight UDP multicast heartbeat that tells
//! `lmml-router` where to probe for authenticated node APIs.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use lmml_api::{LanNodeAdvertisement, LAN_DISCOVERY_MAGIC, LAN_DISCOVERY_VERSION};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::net::UdpSocket;

use crate::{is_local_bind, socket_addr_string, NodeConfig, NodeConfigError, NodeSnapshot};

/// Return the public base URL a LAN router should probe for this node.
pub fn advertised_public_url(config: &NodeConfig) -> Result<String, NodeConfigError> {
    if let Some(public_url) = config
        .public_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        let public_url = public_url.trim_end_matches('/').to_string();
        if !looks_like_http_url(&public_url) {
            return Err(NodeConfigError::InvalidLanAdvertisementPublicUrl { url: public_url });
        }
        return Ok(public_url);
    }
    if is_local_bind(&config.host) || matches!(config.host.as_str(), "0.0.0.0" | "::") {
        return Err(NodeConfigError::PublicUrlRequiredForLanAdvertisement {
            host: config.host.clone(),
        });
    }
    Ok(format!(
        "http://{}",
        socket_addr_string(&config.host, config.port)
    ))
}

/// Run the periodic UDP multicast advertiser for a node snapshot.
pub async fn run_lan_advertiser(snapshot: NodeSnapshot) -> io::Result<()> {
    let public_url = advertised_public_url(&snapshot.config).map_err(io::Error::other)?;
    let socket = UdpSocket::bind(sender_bind_addr(snapshot.config.lan_advertisement_addr)).await?;
    let interval = Duration::from_millis(snapshot.config.lan_advertisement_interval_ms.max(250));
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let advertisement = snapshot.lan_advertisement(public_url.clone());
        let bytes = serde_json::to_vec(&advertisement).map_err(io::Error::other)?;
        socket
            .send_to(&bytes, snapshot.config.lan_advertisement_addr)
            .await?;
    }
}

impl NodeSnapshot {
    /// Build one LAN discovery advertisement from the current node snapshot.
    pub fn lan_advertisement(&self, public_url: String) -> LanNodeAdvertisement {
        let capabilities = self.capabilities();
        LanNodeAdvertisement {
            magic: LAN_DISCOVERY_MAGIC.to_string(),
            version: LAN_DISCOVERY_VERSION,
            api_version: capabilities.api_version,
            node_id: capabilities.node_id,
            node_name: capabilities.node_name,
            public_url,
            backend: capabilities.backend,
            gpus: capabilities.gpus,
            models: capabilities.models,
            auth_required: capabilities.auth_required,
            roles: capabilities.roles,
            tags: capabilities.tags,
            last_seen_utc: utc_now_rfc3339(),
        }
    }
}

fn sender_bind_addr(destination: SocketAddr) -> SocketAddr {
    match destination {
        SocketAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
        SocketAddr::V6(_) => SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 0], 0)),
    }
}

fn utc_now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn looks_like_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}
