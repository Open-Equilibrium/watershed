//! Tunnels to other hosts (ADR-0288): bytes between a loopback TCP connection and a WebSocket to
//! another host's remote listener, which hands them to its T3 server. The bytes pass unchanged, so
//! T3 sees the HTTP `Host` header its peer sent, which its link's token is bound to.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::api;

/// The bytes a tunnel message carries at most.
const CHUNK: usize = 16 * 1024;

/// Copies bytes both ways between `tcp` and `socket`, as binary messages, until either side closes.
pub(crate) async fn relay<S: AsyncRead + AsyncWrite + Unpin>(
    tcp: TcpStream,
    socket: WebSocketStream<S>,
) {
    let (mut reader, mut writer) = tcp.into_split();
    let (mut sink, mut frames) = socket.split();
    let up = async {
        let mut buffer = vec![0u8; CHUNK];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if sink
                        .send(Message::Binary(buffer[..n].to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    };
    let down = async {
        while let Some(Ok(frame)) = frames.next().await {
            match frame {
                Message::Binary(bytes) => {
                    if writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    };
    tokio::select! {
        () = up => {}
        () = down => {}
    }
}

/// Accepts connections on `listener` and tunnels each one to the remote listener at `address`,
/// with the host token `token`, until the task is aborted.
pub(crate) async fn listen(listener: TcpListener, address: String, token: String) {
    loop {
        match listener.accept().await {
            Ok((tcp, _)) => {
                tokio::spawn(open(tcp, address.clone(), token.clone()));
            }
            // Such as too many open files: the tunnel keeps listening.
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Opens the tunnel to `address` for `tcp`. A peer that refuses or cannot be reached closes `tcp`.
async fn open(tcp: TcpStream, address: String, token: String) {
    let version = api::API_VERSION.to_string();
    let Ok(request) = crate::remote::handshake(
        &address,
        &[
            ("authorization", format!("Bearer {token}").as_str()),
            (api::API_HEADER, version.as_str()),
            (api::TUNNEL_HEADER, "t3"),
        ],
    ) else {
        return;
    };
    if let Ok((socket, _)) = tokio_tungstenite::connect_async(request).await {
        relay(tcp, socket).await;
    }
}
