use std::{net::SocketAddr, time::Duration};

use quinn::{Connection, RecvStream, SendStream};
use tokio::{
    io::{copy, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use super::{protocol::StreamHandshake, P2pError, Result};

pub async fn forward_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    target: SocketAddr,
) -> Result<()> {
    let tcp = tokio::time::timeout(Duration::from_secs(8), TcpStream::connect(target))
        .await
        .map_err(|_| P2pError::Timeout)??;
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    let upstream = async {
        copy(&mut tcp_read, &mut send).await?;
        send.finish()
            .map_err(|e| std::io::Error::other(e.to_string()))
    };
    let downstream = async {
        copy(&mut recv, &mut tcp_write).await?;
        tcp_write.shutdown().await
    };
    tokio::try_join!(upstream, downstream)
        .map(|_| ())
        .map_err(Into::into)
}

pub struct LocalProxy {
    pub address: SocketAddr,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl LocalProxy {
    pub async fn start(
        bind: SocketAddr,
        connection: Connection,
        handshake: StreamHandshake,
    ) -> Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let address = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = tokio::select! {
                    _ = worker_cancel.cancelled() => break,
                    accepted = listener.accept() => match accepted { Ok(value) => value, Err(_) => break },
                };
                let connection = connection.clone();
                let handshake = handshake.clone();
                tokio::spawn(async move {
                    let _ = proxy_socket(socket, connection, handshake).await;
                });
            }
        });
        Ok(Self {
            address,
            cancel,
            task,
        })
    }

    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

async fn proxy_socket(
    socket: TcpStream,
    connection: Connection,
    handshake: StreamHandshake,
) -> Result<()> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| P2pError::Proxy(error.to_string()))?;
    let bytes = bincode::serialize(&handshake)?;
    send.write_u32(bytes.len() as u32).await?;
    send.write_all(&bytes)
        .await
        .map_err(|error| P2pError::Proxy(error.to_string()))?;
    let (mut tcp_read, mut tcp_write) = socket.into_split();
    let upstream = async {
        copy(&mut tcp_read, &mut send).await?;
        send.finish()
            .map_err(|e| std::io::Error::other(e.to_string()))
    };
    let downstream = async {
        copy(&mut recv, &mut tcp_write).await?;
        tcp_write.shutdown().await
    };
    tokio::try_join!(upstream, downstream)
        .map(|_| ())
        .map_err(Into::into)
}
