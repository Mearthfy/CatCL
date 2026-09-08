use std::{net::SocketAddr, sync::Arc, time::Duration};

use quinn::{ClientConfig, Endpoint, ServerConfig};
use rustls::RootCertStore;
use tokio::{io::AsyncReadExt, sync::RwLock, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::{
    crypto::EphemeralIdentity,
    protocol::{StreamHandshake, MAX_CONTROL_FRAME},
    proxy::forward_stream,
    session::Session,
    P2pError, Result,
};

pub struct HostTransport {
    pub address: SocketAddr,
    pub certificate: Vec<u8>,
    endpoint: Endpoint,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl HostTransport {
    pub async fn start(
        bind: SocketAddr,
        target: SocketAddr,
        session: Arc<RwLock<Session>>,
    ) -> Result<Self> {
        let identity = EphemeralIdentity::generate()?;
        let certificate = identity.certificate.to_vec();
        let server =
            ServerConfig::with_single_cert(vec![identity.certificate], identity.private_key.into())
                .map_err(|error| P2pError::CryptoError(error.to_string()))?;
        let endpoint = Endpoint::server(server, bind)
            .map_err(|error| P2pError::NetworkUnavailable(error.to_string()))?;
        let address = endpoint.local_addr()?;
        let worker_endpoint = endpoint.clone();
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            loop {
                let incoming = tokio::select! {
                    _ = worker_cancel.cancelled() => break,
                    incoming = worker_endpoint.accept() => match incoming { Some(value) => value, None => break },
                };
                let session = session.clone();
                tokio::spawn(async move {
                    let Ok(connection) = incoming.await else {
                        return;
                    };
                    while let Ok((send, mut recv)) = connection.accept_bi().await {
                        let length =
                            match tokio::time::timeout(Duration::from_secs(8), recv.read_u32())
                                .await
                            {
                                Ok(Ok(value)) if value as usize <= MAX_CONTROL_FRAME => {
                                    value as usize
                                }
                                _ => break,
                            };
                        let mut bytes = vec![0; length];
                        if recv.read_exact(&mut bytes).await.is_err() {
                            break;
                        }
                        let Ok(handshake) = bincode::deserialize::<StreamHandshake>(&bytes) else {
                            break;
                        };
                        let allowed = {
                            let session = session.read().await;
                            handshake.magic == super::protocol::MAGIC
                                && handshake.protocol_version == super::protocol::PROTOCOL_VERSION
                                && handshake.session_id == session.id
                                && session.verify(&handshake.peer_id, &handshake.nonce)
                        };
                        if !allowed {
                            break;
                        }
                        tokio::spawn(async move {
                            let _ = forward_stream(send, recv, target).await;
                        });
                    }
                });
            }
        });
        Ok(Self {
            address,
            certificate,
            endpoint,
            cancel,
            task,
        })
    }

    pub async fn shutdown(self) {
        self.cancel.cancel();
        self.endpoint.close(0u32.into(), b"host shutdown");
        let _ = self.task.await;
    }
}

pub async fn connect(
    host: SocketAddr,
    certificate: Vec<u8>,
) -> Result<(Endpoint, quinn::Connection)> {
    connect_with_timeout(host, certificate, Duration::from_secs(10)).await
}

pub async fn connect_with_timeout(
    host: SocketAddr,
    certificate: Vec<u8>,
    timeout: Duration,
) -> Result<(Endpoint, quinn::Connection)> {
    let mut roots = RootCertStore::empty();
    roots
        .add(certificate.into())
        .map_err(|error| P2pError::CryptoError(error.to_string()))?;
    let client = ClientConfig::with_root_certificates(Arc::new(roots))
        .map_err(|error| P2pError::CryptoError(error.to_string()))?;
    let bind = if host.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let mut endpoint = Endpoint::client(bind.parse().expect("static socket address"))
        .map_err(|error| P2pError::NetworkUnavailable(error.to_string()))?;
    endpoint.set_default_client_config(client);
    let connection = tokio::time::timeout(
        timeout,
        endpoint
            .connect(host, "catcl.local")
            .map_err(|error| P2pError::NetworkUnavailable(error.to_string()))?,
    )
    .await
    .map_err(|_| P2pError::Timeout)?
    .map_err(|error| P2pError::HandshakeFailed(error.to_string()))?;
    Ok((endpoint, connection))
}
