use std::{net::SocketAddr, sync::Arc, time::Duration};

use subtle::ConstantTimeEq;
use tokio::{net::UdpSocket, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::{
    protocol::{MAGIC, PROTOCOL_VERSION},
    P2pError, Result,
};

const PUNCH: u8 = 1;
const ACK: u8 = 2;
const KEEPALIVE: u8 = 3;
const PACKET_LEN: usize = 54;
const MAX_CANDIDATES: usize = 32;

fn packet(kind: u8, session_id: [u8; 16], token: [u8; 32]) -> [u8; PACKET_LEN] {
    let mut bytes = [0; PACKET_LEN];
    bytes[..3].copy_from_slice(&MAGIC);
    bytes[3..5].copy_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    bytes[5] = kind;
    bytes[6..22].copy_from_slice(&session_id);
    bytes[22..].copy_from_slice(&token);
    bytes
}

fn validate(bytes: &[u8], session_id: &[u8; 16], expected_token: &[u8; 32]) -> Option<u8> {
    if bytes.len() != PACKET_LEN
        || bytes[..3] != MAGIC
        || bytes[3..5] != PROTOCOL_VERSION.to_be_bytes()
        || !bool::from(bytes[6..22].ct_eq(session_id))
        || !bool::from(bytes[22..].ct_eq(expected_token))
    {
        return None;
    }
    matches!(bytes[5], PUNCH | ACK | KEEPALIVE).then_some(bytes[5])
}

pub struct PunchedSocket {
    pub socket: Arc<UdpSocket>,
    pub remote: SocketAddr,
    cancel: CancellationToken,
    keepalive: JoinHandle<()>,
}

impl PunchedSocket {
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.keepalive.await;
    }

    pub async fn into_std(self) -> Result<std::net::UdpSocket> {
        self.cancel.cancel();
        let _ = self.keepalive.await;
        let socket = Arc::try_unwrap(self.socket)
            .map_err(|_| P2pError::NatTraversalFailed("UDP socket 仍被后台任务占用".into()))?;
        socket.into_std().map_err(Into::into)
    }
}

pub async fn punch(
    socket: UdpSocket,
    candidates: &[SocketAddr],
    session_id: [u8; 16],
    local_token: [u8; 32],
    expected_remote_token: [u8; 32],
    timeout: Duration,
) -> Result<PunchedSocket> {
    if candidates.is_empty() || candidates.len() > MAX_CANDIDATES {
        return Err(P2pError::NatTraversalFailed("远端候选数量无效".into()));
    }
    let socket = Arc::new(socket);
    let outbound = packet(PUNCH, session_id, local_token);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut interval = tokio::time::interval(Duration::from_millis(120));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut buffer = [0; 256];
    let remote = loop {
        tokio::select! {
            _ = interval.tick() => {
                for candidate in candidates { let _ = socket.send_to(&outbound, candidate).await; }
            }
            received = socket.recv_from(&mut buffer) => {
                let (length, source) = match received {
                    Ok(value) => value,
                    Err(error) if matches!(error.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) => continue,
                    Err(error) => return Err(error.into()),
                };
                if !candidates.contains(&source) { continue; }
                if let Some(kind) = validate(&buffer[..length], &session_id, &expected_remote_token) {
                    if kind == PUNCH { socket.send_to(&packet(ACK, session_id, local_token), source).await?; }
                    break source;
                }
            }
            _ = tokio::time::sleep_until(deadline) => return Err(P2pError::Timeout),
        }
    };
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let worker_socket = socket.clone();
    let keepalive_packet = packet(KEEPALIVE, session_id, local_token);
    let keepalive = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.tick().await;
        loop {
            tokio::select! {
                _ = worker_cancel.cancelled() => break,
                _ = interval.tick() => { if worker_socket.send_to(&keepalive_packet, remote).await.is_err() { break; } }
            }
        }
    });
    Ok(PunchedSocket {
        socket,
        remote,
        cancel,
        keepalive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_and_wrong_session_packets_are_rejected() {
        assert!(validate(&[0; 10], &[1; 16], &[2; 32]).is_none());
        assert!(validate(&packet(PUNCH, [3; 16], [2; 32]), &[1; 16], &[2; 32]).is_none());
    }

    #[tokio::test]
    async fn two_peers_punch_simultaneously_and_shutdown() {
        let left = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let left_address = left.local_addr().unwrap();
        let right_address = right.local_addr().unwrap();
        let session = [8; 16];
        let left_job = tokio::spawn(async move {
            punch(
                left,
                &[right_address],
                session,
                [1; 32],
                [2; 32],
                Duration::from_secs(2),
            )
            .await
        });
        let right_job = tokio::spawn(async move {
            punch(
                right,
                &[left_address],
                session,
                [2; 32],
                [1; 32],
                Duration::from_secs(2),
            )
            .await
        });
        let left = left_job.await.unwrap().unwrap();
        let right = right_job.await.unwrap().unwrap();
        assert_eq!(left.remote, right_address);
        assert_eq!(right.remote, left_address);
        left.shutdown().await;
        right.shutdown().await;
    }

    #[tokio::test]
    async fn punching_times_out() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let result = punch(
            socket,
            &["127.0.0.1:9".parse().unwrap()],
            [1; 16],
            [2; 32],
            [3; 32],
            Duration::from_millis(60),
        )
        .await;
        assert!(matches!(result, Err(P2pError::Timeout)));
    }
}
