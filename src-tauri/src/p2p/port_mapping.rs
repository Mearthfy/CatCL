use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use igd_next::{aio::tokio::search_gateway, PortMappingProtocol, SearchOptions};
use serde::Serialize;
use tokio::{net::UdpSocket, sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::{network::local_addresses, P2pError, Result};

const MAPPING_PORT: u16 = 5351;
const NAT_PMP_VERSION: u8 = 0;
const PCP_VERSION: u8 = 2;
const UDP_PROTOCOL: u8 = 17;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum MappingMethod {
    Upnp,
    NatPmp,
    Pcp,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    pub method: MappingMethod,
    pub internal_port: u16,
    pub external_port: u16,
    pub external_address: Option<IpAddr>,
    pub lifetime_secs: u32,
}

struct ActiveMapping {
    mapping: PortMapping,
    local_address: Ipv4Addr,
    cancel: CancellationToken,
    renewal: JoinHandle<()>,
}

#[derive(Default)]
pub struct MappingState(Mutex<Option<ActiveMapping>>);

fn gateway_ipv4() -> Result<Ipv4Addr> {
    match default_net::get_default_gateway()
        .map_err(|error| P2pError::PortMappingFailed(error.to_string()))?
        .ip_addr
    {
        IpAddr::V4(address) => Ok(address),
        _ => Err(P2pError::PortMappingFailed("默认网关不是 IPv4".into())),
    }
}

async fn mapping_socket(gateway: Ipv4Addr) -> Result<UdpSocket> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket
        .connect(SocketAddrV4::new(gateway, MAPPING_PORT))
        .await?;
    Ok(socket)
}

async fn exchange(socket: &UdpSocket, request: &[u8], minimum: usize) -> Result<Vec<u8>> {
    let mut response = [0; 128];
    let mut delay = Duration::from_millis(250);
    for _ in 0..4 {
        socket.send(request).await?;
        match tokio::time::timeout(delay, socket.recv(&mut response)).await {
            Ok(Ok(length)) if length >= minimum => return Ok(response[..length].to_vec()),
            Ok(Ok(_)) => return Err(P2pError::PortMappingFailed("路由器响应过短".into())),
            Ok(Err(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset
                ) => {}
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => {}
        }
        delay *= 2;
    }
    Err(P2pError::Timeout)
}

pub async fn map_nat_pmp(
    internal_port: u16,
    external_port: u16,
    lifetime: u32,
) -> Result<PortMapping> {
    let socket = mapping_socket(gateway_ipv4()?).await?;
    let mut request = [0; 12];
    request[0] = NAT_PMP_VERSION;
    request[1] = 1;
    request[4..6].copy_from_slice(&internal_port.to_be_bytes());
    request[6..8].copy_from_slice(&external_port.to_be_bytes());
    request[8..12].copy_from_slice(&lifetime.to_be_bytes());
    let response = exchange(&socket, &request, 16).await?;
    parse_nat_pmp(&response)
}

fn parse_nat_pmp(response: &[u8]) -> Result<PortMapping> {
    if response.len() < 16 || response[0] != 0 || response[1] != 129 {
        return Err(P2pError::PortMappingFailed("NAT-PMP 响应无效".into()));
    }
    let result = u16::from_be_bytes([response[2], response[3]]);
    if result != 0 {
        return Err(P2pError::PortMappingFailed(format!(
            "NAT-PMP 错误码 {result}"
        )));
    }
    Ok(PortMapping {
        method: MappingMethod::NatPmp,
        internal_port: u16::from_be_bytes([response[8], response[9]]),
        external_port: u16::from_be_bytes([response[10], response[11]]),
        external_address: None,
        lifetime_secs: u32::from_be_bytes(response[12..16].try_into().unwrap_or_default()),
    })
}

fn ipv4_mapped(address: Ipv4Addr) -> [u8; 16] {
    let mut value = [0; 16];
    value[10] = 0xff;
    value[11] = 0xff;
    value[12..].copy_from_slice(&address.octets());
    value
}

pub async fn map_pcp(internal_port: u16, external_port: u16, lifetime: u32) -> Result<PortMapping> {
    let gateway = gateway_ipv4()?;
    let socket = mapping_socket(gateway).await?;
    let local = match socket.local_addr()?.ip() {
        IpAddr::V4(value) => value,
        _ => return Err(P2pError::PortMappingFailed("本机地址不是 IPv4".into())),
    };
    let mut nonce = [0; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
    let mut request = [0; 60];
    request[0] = PCP_VERSION;
    request[1] = 1;
    request[4..8].copy_from_slice(&lifetime.to_be_bytes());
    request[8..24].copy_from_slice(&ipv4_mapped(local));
    request[24..36].copy_from_slice(&nonce);
    request[36] = UDP_PROTOCOL;
    request[40..42].copy_from_slice(&internal_port.to_be_bytes());
    request[42..44].copy_from_slice(&external_port.to_be_bytes());
    let response = exchange(&socket, &request, 60).await?;
    parse_pcp(&response, nonce)
}

fn parse_pcp(response: &[u8], nonce: [u8; 12]) -> Result<PortMapping> {
    if response.len() < 60
        || response[0] != PCP_VERSION
        || response[1] != 0x81
        || response[3] != 0
        || response[24..36] != nonce
    {
        return Err(P2pError::PortMappingFailed(format!(
            "PCP 响应无效或错误码 {}",
            response.get(3).copied().unwrap_or(255)
        )));
    }
    let address: [u8; 16] = response[44..60].try_into().unwrap_or_default();
    Ok(PortMapping {
        method: MappingMethod::Pcp,
        internal_port: u16::from_be_bytes([response[40], response[41]]),
        external_port: u16::from_be_bytes([response[42], response[43]]),
        external_address: Some(Ipv6AddrExt::to_ip(address)),
        lifetime_secs: u32::from_be_bytes(response[4..8].try_into().unwrap_or_default()),
    })
}

struct Ipv6AddrExt;
impl Ipv6AddrExt {
    fn to_ip(bytes: [u8; 16]) -> IpAddr {
        let value = std::net::Ipv6Addr::from(bytes);
        value
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(value))
    }
}

pub async fn map_upnp(
    local_address: Ipv4Addr,
    internal_port: u16,
    external_port: u16,
    lifetime: u32,
) -> Result<PortMapping> {
    let gateway = tokio::time::timeout(
        Duration::from_secs(3),
        search_gateway(SearchOptions::default()),
    )
    .await
    .map_err(|_| P2pError::Timeout)?
    .map_err(|error| P2pError::PortMappingFailed(error.to_string()))?;
    gateway
        .add_port(
            PortMappingProtocol::UDP,
            external_port,
            SocketAddr::V4(SocketAddrV4::new(local_address, internal_port)),
            lifetime,
            "CatCL P2P",
        )
        .await
        .map_err(|error| P2pError::PortMappingFailed(error.to_string()))?;
    let external_address = gateway.get_external_ip().await.ok();
    Ok(PortMapping {
        method: MappingMethod::Upnp,
        internal_port,
        external_port,
        external_address,
        lifetime_secs: lifetime,
    })
}

pub async fn release(mapping: &PortMapping, local_address: Ipv4Addr) -> Result<()> {
    match mapping.method {
        MappingMethod::Upnp => {
            let gateway = search_gateway(SearchOptions::default())
                .await
                .map_err(|error| P2pError::PortMappingFailed(error.to_string()))?;
            gateway
                .remove_port(PortMappingProtocol::UDP, mapping.external_port)
                .await
                .map_err(|error| P2pError::PortMappingFailed(error.to_string()))
        }
        MappingMethod::NatPmp => map_nat_pmp(mapping.internal_port, mapping.external_port, 0)
            .await
            .map(|_| ()),
        MappingMethod::Pcp => {
            let _ = local_address;
            map_pcp(mapping.internal_port, mapping.external_port, 0)
                .await
                .map(|_| ())
        }
    }
}

pub async fn map_best(
    local_address: Ipv4Addr,
    internal_port: u16,
    preferred_external: u16,
    lifetime: u32,
) -> Result<PortMapping> {
    let lifetime = lifetime.clamp(60, 7200);
    if let Ok(mapping) = map_upnp(local_address, internal_port, preferred_external, lifetime).await
    {
        return Ok(mapping);
    }
    if let Ok(mapping) = map_nat_pmp(internal_port, preferred_external, lifetime).await {
        return Ok(mapping);
    }
    map_pcp(internal_port, preferred_external, lifetime).await
}

async fn renew(mapping: &PortMapping, local_address: Ipv4Addr) -> Result<PortMapping> {
    match mapping.method {
        MappingMethod::Upnp => {
            map_upnp(
                local_address,
                mapping.internal_port,
                mapping.external_port,
                mapping.lifetime_secs,
            )
            .await
        }
        MappingMethod::NatPmp => {
            map_nat_pmp(
                mapping.internal_port,
                mapping.external_port,
                mapping.lifetime_secs,
            )
            .await
        }
        MappingMethod::Pcp => {
            map_pcp(
                mapping.internal_port,
                mapping.external_port,
                mapping.lifetime_secs,
            )
            .await
        }
    }
}

#[tauri::command]
pub async fn create_port_mapping(
    state: tauri::State<'_, MappingState>,
    local_address: String,
    internal_port: u16,
    preferred_external_port: Option<u16>,
    lifetime_secs: Option<u32>,
) -> std::result::Result<PortMapping, String> {
    if internal_port == 0 {
        return Err("内部端口不能为 0".into());
    }
    let local_address: Ipv4Addr = local_address.parse().map_err(|_| "本机 IPv4 地址无效")?;
    if !local_addresses()
        .map_err(|error| error.to_string())?
        .contains(&IpAddr::V4(local_address))
    {
        return Err("该 IPv4 地址不属于本机网络接口".into());
    }
    let mut active = state.0.lock().await;
    if active.is_some() {
        return Err("当前已有活动的端口映射，请先关闭联机房间".into());
    }
    let lifetime = lifetime_secs.unwrap_or(1800).clamp(60, 7200);
    let mapping = map_best(
        local_address,
        internal_port,
        preferred_external_port.unwrap_or(internal_port),
        lifetime,
    )
    .await
    .map_err(|error| error.to_string())?;
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let worker_mapping = mapping.clone();
    let renewal = tokio::spawn(async move {
        let delay = Duration::from_secs((worker_mapping.lifetime_secs / 2).max(30) as u64);
        loop {
            tokio::select! {
                _ = worker_cancel.cancelled() => break,
                _ = tokio::time::sleep(delay) => { let _ = renew(&worker_mapping, local_address).await; }
            }
        }
    });
    *active = Some(ActiveMapping {
        mapping: mapping.clone(),
        local_address,
        cancel,
        renewal,
    });
    Ok(mapping)
}

#[tauri::command]
pub async fn remove_port_mapping(
    state: tauri::State<'_, MappingState>,
) -> std::result::Result<(), String> {
    let active = state.0.lock().await.take();
    if let Some(active) = active {
        active.cancel.cancel();
        let _ = active.renewal.await;
        release(&active.mapping, active.local_address)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_ipv4_mapped_address() {
        assert_eq!(
            Ipv6AddrExt::to_ip(ipv4_mapped(Ipv4Addr::new(1, 2, 3, 4))),
            IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))
        );
    }
    #[test]
    fn parses_nat_pmp_success_and_error() {
        let mut response = [0; 16];
        response[1] = 129;
        response[8..10].copy_from_slice(&25565u16.to_be_bytes());
        response[10..12].copy_from_slice(&45678u16.to_be_bytes());
        response[12..16].copy_from_slice(&1800u32.to_be_bytes());
        let mapping = parse_nat_pmp(&response).unwrap();
        assert_eq!(mapping.external_port, 45678);
        assert_eq!(mapping.lifetime_secs, 1800);
        response[3] = 2;
        assert!(parse_nat_pmp(&response).is_err());
    }
    #[test]
    fn pcp_rejects_wrong_nonce() {
        let mut response = [0; 60];
        response[0] = PCP_VERSION;
        response[1] = 0x81;
        response[24..36].copy_from_slice(&[1; 12]);
        assert!(parse_pcp(&response, [2; 12]).is_err());
    }
}
