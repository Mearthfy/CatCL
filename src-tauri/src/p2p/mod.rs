pub mod candidate;
pub mod controller;
pub mod crypto;
pub mod error;
pub mod hole_punch;
pub mod invite;
pub mod minecraft;
pub mod nat;
pub mod network;
pub mod port_mapping;
pub mod protocol;
pub mod proxy;
pub mod session;
pub mod transport;

pub use error::{P2pError, Result};

#[cfg(test)]
mod tests {
    use rand::{rngs::OsRng, RngCore};
    use std::{
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::RwLock,
    };

    use super::{
        crypto::EphemeralIdentity,
        invite::Answer,
        protocol::{StreamHandshake, MAGIC, PROTOCOL_VERSION},
        proxy::LocalProxy,
        session::Session,
        transport::{connect, connect_with_timeout, HostTransport},
        P2pError,
    };

    #[tokio::test]
    async fn loopback_tcp_quic_tcp_tunnel_supports_multiple_streams_and_shutdown() {
        let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let echo_address = echo.local_addr().unwrap();
        let echo_task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = echo.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0; 4096];
                    loop {
                        let read = match socket.read(&mut buffer).await {
                            Ok(0) | Err(_) => break,
                            Ok(value) => value,
                        };
                        if socket.write_all(&buffer[..read]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });

        let session = Arc::new(RwLock::new(Session::new(4)));
        let host = HostTransport::start(
            "127.0.0.1:0".parse().unwrap(),
            echo_address,
            session.clone(),
        )
        .await
        .unwrap();
        let mut peer_id = [0; 16];
        let mut peer_nonce = [0; 32];
        OsRng.fill_bytes(&mut peer_id);
        OsRng.fill_bytes(&mut peer_nonce);
        let (session_id, host_nonce) = {
            let value = session.read().await;
            (value.id, value.host_nonce)
        };
        let answer = Answer {
            magic: MAGIC,
            protocol_version: PROTOCOL_VERSION,
            session_id,
            peer_id,
            offer_nonce: host_nonce,
            peer_nonce,
            candidates: vec![],
            expires_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 60,
        };
        session.write().await.authorize(answer).unwrap();
        let (endpoint, connection) = connect(host.address, host.certificate.clone())
            .await
            .unwrap();
        let proxy = LocalProxy::start(
            "127.0.0.1:0".parse().unwrap(),
            connection,
            StreamHandshake::new(session_id, peer_id, peer_nonce),
        )
        .await
        .unwrap();

        let mut jobs = Vec::new();
        for index in 0..3u8 {
            let address = proxy.address;
            jobs.push(tokio::spawn(async move {
                let mut socket = TcpStream::connect(address).await.unwrap();
                let payload = vec![index; 128 * 1024];
                socket.write_all(&payload).await.unwrap();
                socket.shutdown().await.unwrap();
                let mut echoed = Vec::new();
                tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut echoed))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(echoed, payload);
            }));
        }
        for job in jobs {
            job.await.unwrap();
        }
        proxy.shutdown().await;
        endpoint.close(0u32.into(), b"client shutdown");
        host.shutdown().await;
        echo_task.abort();
    }

    #[tokio::test]
    async fn connection_timeout_is_reported() {
        let identity = EphemeralIdentity::generate().unwrap();
        let result = connect_with_timeout(
            "127.0.0.1:9".parse().unwrap(),
            identity.certificate.to_vec(),
            Duration::from_millis(30),
        )
        .await;
        assert!(matches!(
            result,
            Err(P2pError::Timeout) | Err(P2pError::HandshakeFailed(_))
        ));
    }
}
