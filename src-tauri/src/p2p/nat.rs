use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use rand::{rngs::OsRng, RngCore};
use serde::Serialize;
use tokio::net::{lookup_host, UdpSocket};

use super::{P2pError, Result};

const STUN_COOKIE: u32 = 0x2112_A442;
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_SUCCESS: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const MAX_STUN_PACKET: usize = 2048;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum NatType {
    OpenInternet,
    ConeOrRestricted,
    Symmetric,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NatReport {
    pub nat_type: NatType,
    pub mapped_addresses: Vec<SocketAddr>,
    pub successful_servers: usize,
}

fn binding_request(transaction: [u8; 12]) -> [u8; 20] {
    let mut packet = [0; 20];
    packet[..2].copy_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    packet[4..8].copy_from_slice(&STUN_COOKIE.to_be_bytes());
    packet[8..].copy_from_slice(&transaction);
    packet
}

fn mapped_address(packet: &[u8], transaction: [u8; 12]) -> Result<SocketAddr> {
    if packet.len() < 20 || packet.len() > MAX_STUN_PACKET {
        return Err(P2pError::NatTraversalFailed("STUN 响应长度无效".into()));
    }
    if u16::from_be_bytes([packet[0], packet[1]]) != STUN_BINDING_SUCCESS
        || u32::from_be_bytes(packet[4..8].try_into().unwrap_or_default()) != STUN_COOKIE
        || packet[8..20] != transaction
    {
        return Err(P2pError::NatTraversalFailed("STUN 响应头无效".into()));
    }
    let declared = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if declared + 20 > packet.len() {
        return Err(P2pError::NatTraversalFailed("STUN 响应被截断".into()));
    }
    let mut offset = 20;
    while offset + 4 <= 20 + declared {
        let kind = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let length = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4;
        if offset + length > packet.len() {
            return Err(P2pError::NatTraversalFailed("STUN 属性被截断".into()));
        }
        if matches!(kind, ATTR_XOR_MAPPED_ADDRESS | ATTR_MAPPED_ADDRESS) && length >= 8 {
            let xor = kind == ATTR_XOR_MAPPED_ADDRESS;
            let family = packet[offset + 1];
            let mut port = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
            if xor {
                port ^= (STUN_COOKIE >> 16) as u16;
            }
            let address = match family {
                0x01 if length >= 8 => {
                    let mut octets: [u8; 4] = packet[offset + 4..offset + 8]
                        .try_into()
                        .unwrap_or_default();
                    if xor {
                        for (index, byte) in STUN_COOKIE.to_be_bytes().iter().enumerate() {
                            octets[index] ^= byte;
                        }
                    }
                    IpAddr::V4(Ipv4Addr::from(octets))
                }
                0x02 if length >= 20 => {
                    let mut octets: [u8; 16] = packet[offset + 4..offset + 20]
                        .try_into()
                        .unwrap_or_default();
                    if xor {
                        let mask: Vec<u8> = STUN_COOKIE
                            .to_be_bytes()
                            .into_iter()
                            .chain(transaction)
                            .collect();
                        for (value, mask) in octets.iter_mut().zip(mask) {
                            *value ^= mask;
                        }
                    }
                    IpAddr::V6(Ipv6Addr::from(octets))
                }
                _ => {
                    offset += (length + 3) & !3;
                    continue;
                }
            };
            return Ok(SocketAddr::new(address, port));
        }
        offset += (length + 3) & !3;
    }
    Err(P2pError::NatTraversalFailed("STUN 响应缺少映射地址".into()))
}

pub async fn discover_public_endpoint(
    socket: &UdpSocket,
    server: &str,
    timeout: Duration,
) -> Result<SocketAddr> {
    let ipv6 = socket.local_addr()?.is_ipv6();
    let server = lookup_host(server)
        .await?
        .find(|address| address.is_ipv6() == ipv6)
        .ok_or_else(|| P2pError::NatTraversalFailed("STUN 地址无法解析".into()))?;
    let mut transaction = [0; 12];
    OsRng.fill_bytes(&mut transaction);
    let request = binding_request(transaction);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut retry = Duration::from_millis(200);
    let mut buffer = [0; MAX_STUN_PACKET];
    loop {
        socket.send_to(&request, server).await?;
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(P2pError::Timeout);
        }
        match tokio::time::timeout(remaining.min(retry), socket.recv_from(&mut buffer)).await {
            Ok(Ok((length, source))) if source == server => {
                return mapped_address(&buffer[..length], transaction)
            }
            Ok(Ok(_)) => continue,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => retry = (retry * 2).min(Duration::from_secs(1)),
        }
    }
}

pub async fn detect_nat(servers: &[String], timeout: Duration) -> Result<NatReport> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local = socket.local_addr()?;
    let mut mapped = Vec::new();
    for server in servers.iter().take(4) {
        if let Ok(address) = discover_public_endpoint(&socket, server, timeout).await {
            mapped.push(address);
        }
    }
    if mapped.is_empty() {
        return Err(P2pError::NatTraversalFailed("所有 STUN 探测均失败".into()));
    }
    let unique: HashSet<_> = mapped.iter().copied().collect();
    let nat_type = if mapped.iter().any(|address| *address == local) {
        NatType::OpenInternet
    } else if unique.len() > 1 {
        NatType::Symmetric
    } else {
        NatType::ConeOrRestricted
    };
    Ok(NatReport {
        nat_type,
        successful_servers: mapped.len(),
        mapped_addresses: mapped,
    })
}

#[tauri::command]
pub async fn detect_p2p_nat(stun_servers: Vec<String>) -> std::result::Result<NatReport, String> {
    if stun_servers.is_empty() {
        return Err("请至少配置一个 STUN 服务器".into());
    }
    detect_nat(&stun_servers, Duration::from_secs(3))
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_xor_mapped_ipv4() {
        let transaction = [7; 12];
        let address = Ipv4Addr::new(203, 0, 113, 8);
        let port = 45678u16;
        let mut packet = vec![0; 32];
        packet[..2].copy_from_slice(&STUN_BINDING_SUCCESS.to_be_bytes());
        packet[2..4].copy_from_slice(&12u16.to_be_bytes());
        packet[4..8].copy_from_slice(&STUN_COOKIE.to_be_bytes());
        packet[8..20].copy_from_slice(&transaction);
        packet[20..22].copy_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        packet[22..24].copy_from_slice(&8u16.to_be_bytes());
        packet[25] = 1;
        packet[26..28].copy_from_slice(&(port ^ (STUN_COOKIE >> 16) as u16).to_be_bytes());
        for (target, (source, mask)) in packet[28..32]
            .iter_mut()
            .zip(address.octets().into_iter().zip(STUN_COOKIE.to_be_bytes()))
        {
            *target = source ^ mask;
        }
        assert_eq!(
            mapped_address(&packet, transaction).unwrap(),
            SocketAddr::new(IpAddr::V4(address), port)
        );
    }
    #[test]
    fn rejects_wrong_transaction() {
        assert!(mapped_address(&[0; 20], [1; 12]).is_err());
    }

    #[tokio::test]
    async fn discovers_endpoint_from_local_stun_server() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server_address = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut request = [0; 64];
            let (length, source) = server.recv_from(&mut request).await.unwrap();
            assert_eq!(length, 20);
            let transaction: [u8; 12] = request[8..20].try_into().unwrap();
            let mut response = vec![0; 32];
            response[..2].copy_from_slice(&STUN_BINDING_SUCCESS.to_be_bytes());
            response[2..4].copy_from_slice(&12u16.to_be_bytes());
            response[4..8].copy_from_slice(&STUN_COOKIE.to_be_bytes());
            response[8..20].copy_from_slice(&transaction);
            response[20..22].copy_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
            response[22..24].copy_from_slice(&8u16.to_be_bytes());
            response[25] = 1;
            response[26..28]
                .copy_from_slice(&(source.port() ^ (STUN_COOKIE >> 16) as u16).to_be_bytes());
            let source_ip = match source.ip() {
                IpAddr::V4(value) => value.octets(),
                _ => unreachable!(),
            };
            for (target, (value, mask)) in response[28..32]
                .iter_mut()
                .zip(source_ip.into_iter().zip(STUN_COOKIE.to_be_bytes()))
            {
                *target = value ^ mask;
            }
            server.send_to(&response, source).await.unwrap();
        });
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let expected = client.local_addr().unwrap();
        let discovered =
            discover_public_endpoint(&client, &server_address.to_string(), Duration::from_secs(1))
                .await
                .unwrap();
        assert_eq!(discovered, expected);
        task.await.unwrap();
    }
}
