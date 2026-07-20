//! LAN discovery state for `lmml-router`.
//!
//! Discovery only supplies candidate upstream URLs. The main router still
//! verifies each candidate with authenticated health, capabilities, and load
//! probes before routing traffic to it.

use std::collections::BTreeMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lmml_api::{LanNodeAdvertisement, LAN_DISCOVERY_MULTICAST_ADDR, MAX_LAN_ADVERTISEMENT_BYTES};
use thiserror::Error;
use tokio::net::UdpSocket;

use crate::UpstreamNodeConfig;

/// Error returned when a UDP payload is not an acceptable LMML advertisement.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum LanAdvertisementParseError {
    /// Payload is not valid JSON for the advertisement DTO.
    #[error("invalid LAN advertisement JSON")]
    InvalidJson,
    /// Payload does not carry the current LMML advertisement marker/version.
    #[error("unsupported LAN advertisement marker or version")]
    UnsupportedAdvertisement,
    /// Discovered nodes must require authenticated protected routes.
    #[error("LAN advertisement rejected because node does not require auth")]
    UnauthenticatedNode,
    /// Payload does not include a routable HTTP(S) base URL.
    #[error("LAN advertisement missing HTTP public URL")]
    MissingPublicUrl,
}

/// In-memory table of currently discovered LAN nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct DiscoveredNodeTable {
    nodes: Arc<Mutex<BTreeMap<String, DiscoveredNode>>>,
}

#[derive(Debug, Clone)]
struct DiscoveredNode {
    advertisement: LanNodeAdvertisement,
    last_seen: Instant,
}

impl DiscoveredNodeTable {
    /// Record a validated advertisement and return whether it became routable.
    pub(crate) fn record(&self, advertisement: LanNodeAdvertisement, now: Instant) -> bool {
        if validate_lan_advertisement(&advertisement).is_err() {
            return false;
        }
        let Ok(mut nodes) = self.nodes.lock() else {
            return false;
        };
        nodes.insert(
            advertisement.node_id.clone(),
            DiscoveredNode {
                advertisement,
                last_seen: now,
            },
        );
        true
    }

    /// Return active discovered nodes as ordinary upstream configurations.
    pub(crate) fn upstreams(
        &self,
        now: Instant,
        ttl: Duration,
        api_key: &str,
    ) -> Vec<UpstreamNodeConfig> {
        let Ok(mut nodes) = self.nodes.lock() else {
            return Vec::new();
        };
        prune_expired(&mut nodes, now, ttl);
        nodes
            .values()
            .map(|node| {
                let mut upstream = UpstreamNodeConfig::new(
                    node.advertisement.node_id.clone(),
                    node.advertisement.public_url.clone(),
                );
                upstream.api_key = Some(api_key.to_string());
                upstream.discovered = true;
                upstream
            })
            .collect()
    }

    /// Return the number of active discovered nodes.
    pub(crate) fn active_len(&self, now: Instant, ttl: Duration) -> usize {
        let Ok(mut nodes) = self.nodes.lock() else {
            return 0;
        };
        prune_expired(&mut nodes, now, ttl);
        nodes.len()
    }
}

/// Parse and validate one UDP LAN advertisement payload.
pub fn parse_lan_advertisement(
    bytes: &[u8],
) -> Result<LanNodeAdvertisement, LanAdvertisementParseError> {
    let advertisement = serde_json::from_slice::<LanNodeAdvertisement>(bytes)
        .map_err(|_| LanAdvertisementParseError::InvalidJson)?;
    validate_lan_advertisement(&advertisement)?;
    Ok(advertisement)
}

pub(crate) async fn bind_lan_discovery_socket(addr: SocketAddr) -> io::Result<UdpSocket> {
    let SocketAddr::V4(addr) = addr else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LMML LAN discovery currently supports IPv4 multicast only",
        ));
    };
    if !addr.ip().is_multicast() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LMML LAN discovery address must be IPv4 multicast",
        ));
    }
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, addr.port())).await?;
    socket.join_multicast_v4(*addr.ip(), Ipv4Addr::UNSPECIFIED)?;
    socket.set_multicast_loop_v4(true)?;
    Ok(socket)
}

pub(crate) fn default_lan_discovery_addr() -> SocketAddr {
    LAN_DISCOVERY_MULTICAST_ADDR
        .parse()
        .expect("valid default LMML LAN discovery address")
}

pub(crate) fn max_advertisement_bytes() -> usize {
    MAX_LAN_ADVERTISEMENT_BYTES
}

fn validate_lan_advertisement(
    advertisement: &LanNodeAdvertisement,
) -> Result<(), LanAdvertisementParseError> {
    if !advertisement.is_current_lmml_advertisement() {
        return Err(LanAdvertisementParseError::UnsupportedAdvertisement);
    }
    if !advertisement.auth_required {
        return Err(LanAdvertisementParseError::UnauthenticatedNode);
    }
    if !looks_like_http_url(&advertisement.public_url) {
        return Err(LanAdvertisementParseError::MissingPublicUrl);
    }
    Ok(())
}

fn prune_expired(nodes: &mut BTreeMap<String, DiscoveredNode>, now: Instant, ttl: Duration) {
    nodes.retain(|_, node| now.duration_since(node.last_seen) <= ttl);
}

fn looks_like_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lmml_api::{
        BackendKind, NodeRole, API_VERSION, LAN_DISCOVERY_MAGIC, LAN_DISCOVERY_VERSION,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn parses_current_authenticated_advertisement() {
        let payload = json!({
            "magic": LAN_DISCOVERY_MAGIC,
            "version": LAN_DISCOVERY_VERSION,
            "api_version": API_VERSION,
            "node_id": "node-a",
            "node_name": "Node A",
            "public_url": "http://192.168.1.12:8101",
            "backend": "cuda",
            "gpus": [],
            "models": [],
            "auth_required": true,
            "roles": ["lan_worker"],
            "tags": ["lmml"],
            "last_seen_utc": "2026-07-20T00:00:00Z"
        });

        let advertisement =
            parse_lan_advertisement(payload.to_string().as_bytes()).expect("advertisement");

        assert_eq!(advertisement.node_id, "node-a");
        assert_eq!(advertisement.backend, BackendKind::Cuda);
        assert_eq!(advertisement.roles, vec![NodeRole::LanWorker]);
    }

    #[test]
    fn rejects_unauthenticated_advertisement() {
        let payload = json!({
            "magic": LAN_DISCOVERY_MAGIC,
            "version": LAN_DISCOVERY_VERSION,
            "api_version": API_VERSION,
            "node_id": "node-a",
            "node_name": "Node A",
            "public_url": "http://192.168.1.12:8101",
            "backend": "cuda",
            "gpus": [],
            "models": [],
            "auth_required": false,
            "roles": ["lan_worker"],
            "tags": ["lmml"],
            "last_seen_utc": "2026-07-20T00:00:00Z"
        });

        assert_eq!(
            parse_lan_advertisement(payload.to_string().as_bytes()),
            Err(LanAdvertisementParseError::UnauthenticatedNode)
        );
    }

    #[test]
    fn discovered_nodes_expire() {
        let table = DiscoveredNodeTable::default();
        let now = Instant::now();
        assert!(table.record(test_advertisement("node-a", "http://127.0.0.1:8101"), now));
        assert_eq!(table.active_len(now, Duration::from_secs(30)), 1);

        assert_eq!(
            table.active_len(now + Duration::from_secs(31), Duration::from_secs(30)),
            0
        );
    }

    fn test_advertisement(node_id: &str, public_url: &str) -> LanNodeAdvertisement {
        LanNodeAdvertisement {
            magic: LAN_DISCOVERY_MAGIC.to_string(),
            version: LAN_DISCOVERY_VERSION,
            api_version: API_VERSION.to_string(),
            node_id: node_id.to_string(),
            node_name: node_id.to_string(),
            public_url: public_url.to_string(),
            backend: BackendKind::Cuda,
            gpus: Vec::new(),
            models: Vec::new(),
            auth_required: true,
            roles: vec![NodeRole::LanWorker],
            tags: vec!["lmml".to_string()],
            last_seen_utc: "2026-07-20T00:00:00Z".to_string(),
        }
    }
}
