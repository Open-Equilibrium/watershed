//! Tunnels to other hosts (ADR-0288, ADR-0301). A host's tunnel carries the bytes of a loopback TCP
//! connection in binary WebSocket messages to another host's remote listener. There the receiving
//! host serves HTTP on the WebSocket: a request for a path a link uses goes to its T3 with the Host
//! header and the body unchanged, and its answer streams back; any other path is refused with 403,
//! without T3 contact.

use std::convert::Infallible;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::client::conn::http1 as client;
use hyper::server::conn::http1 as server;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, duplex, split};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::api;

/// The bytes a tunnel message carries at most.
const CHUNK: usize = 16 * 1024;

/// The bytes that the HTTP server of a tunnel may hold between its WebSocket and its parser.
const BUFFER: usize = 64 * 1024;

/// The paths a link uses, exactly (ADR-0301): the descriptor, the MCP sign-in a link makes, and the
/// MCP endpoint that linked agents call. T3 registers no other path under `/mcp`.
const LINK_PATHS: [&str; 5] = [
    "/.well-known/t3/environment",
    "/oauth/mcp/register",
    "/oauth/mcp/decision",
    "/oauth/mcp/token",
    "/mcp",
];

/// Copies bytes both ways between `stream` and `socket`, as binary messages, until either side
/// closes.
pub(crate) async fn relay<T, S>(stream: T, socket: WebSocketStream<S>)
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut reader, mut writer) = split(stream);
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

/// Serves HTTP on the WebSocket `socket` of a host's tunnel, for T3 at `t3`, until the client
/// closes it: the requests for a link's paths reach T3, and the others get 403 (see the module).
pub(crate) async fn serve<S>(socket: WebSocketStream<S>, t3: String)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (ours, theirs) = duplex(BUFFER);
    let http = server::Builder::new().serve_connection(
        TokioIo::new(theirs),
        service_fn(move |request| forward(request, t3.clone())),
    );
    // The relay ends by itself once the HTTP connection has dropped its side and every answer byte
    // is sent, so an answer that ends the connection is not cut short. The connection ends when
    // the relay does, which drops its side.
    let _ = tokio::join!(relay(ours, socket), http);
}

/// Answers `request`: a path a link uses goes to T3 at `t3`, and any other is refused.
async fn forward(
    request: Request<Incoming>,
    t3: String,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, Infallible> {
    if !LINK_PATHS.contains(&request.uri().path()) {
        return Ok(answer(StatusCode::FORBIDDEN));
    }
    Ok(match send(request, &t3).await {
        Some(response) => response.map(|body| body.boxed()),
        None => answer(StatusCode::BAD_GATEWAY),
    })
}

/// Sends `request` to T3 at `t3` unchanged, over a connection that lasts as long as the body of
/// the answer, which streams back as it comes.
async fn send(request: Request<Incoming>, t3: &str) -> Option<Response<Incoming>> {
    let socket = TcpStream::connect(t3).await.ok()?;
    let (mut sender, connection) = client::handshake(TokioIo::new(socket)).await.ok()?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    sender.ready().await.ok()?;
    sender.send_request(request).await.ok()
}

/// An answer with `status` and no body.
fn answer(status: StatusCode) -> Response<BoxBody<Bytes, hyper::Error>> {
    let mut response = Response::new(
        Empty::<Bytes>::new()
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    response
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
