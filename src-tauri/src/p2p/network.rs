use std::{collections::HashSet, net::IpAddr};

use serde::Serialize;

use super::{candidate::direct_candidates, invite::Candidate, P2pError, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSnapshot {
    pub ipv6_direct: bool,
    pub udp_available: bool,
    pub candidates: Vec<Candidate>,
}

pub fn local_addresses() -> Result<Vec<IpAddr>> {
    let interfaces = if_addrs::get_if_addrs()
        .map_err(|error| P2pError::NetworkUnavailable(error.to_string()))?;
    let mut seen = HashSet::new();
    Ok(interfaces
        .into_iter()
        .map(|interface| interface.ip())
        .filter(|address| seen.insert(*address))
        .collect())
}

pub async fn inspect(port: u16) -> Result<NetworkSnapshot> {
    let addresses = tokio::task::spawn_blocking(local_addresses)
        .await
        .map_err(|error| P2pError::NetworkUnavailable(error.to_string()))??;
    let candidates = direct_candidates(addresses, port);
    let ipv6_direct = candidates
        .iter()
        .any(|candidate| matches!(candidate.kind, super::invite::CandidateKind::Ipv6Direct));
    let udp_available = tokio::net::UdpSocket::bind("0.0.0.0:0").await.is_ok();
    Ok(NetworkSnapshot {
        ipv6_direct,
        udp_available,
        candidates,
    })
}

#[tauri::command]
pub async fn inspect_p2p_network(
    port: Option<u16>,
) -> std::result::Result<NetworkSnapshot, String> {
    inspect(port.unwrap_or(0))
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn network_inspection_returns_parseable_candidates() {
        let snapshot = inspect(45678).await.unwrap();
        assert!(snapshot
            .candidates
            .iter()
            .all(|candidate| candidate.socket_addr().is_ok()));
    }
}
