//! Tunnels to other hosts (ADR-0288, ADR-0301). A host's tunnel carries the bytes of a loopback TCP
//! connection in binary WebSocket messages to another host's remote listener. There the receiving
//! host serves HTTP on the WebSocket: a request for a path a link uses goes to its T3 with the Host
//! header and the body unchanged, and its answer streams back; any other path is refused with 403,
//! without T3 contact. A decision that approves a link at a level above the one its host was invited
//! at is refused the same way (ADR-0300, ADR-0301).

use std::convert::Infallible;
use std::sync::Arc;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, Limited};
use hyper::body::{Body, Bytes, Incoming};
use hyper::client::conn::http1 as client;
use hyper::header::{CONTENT_LENGTH, HeaderValue, TRANSFER_ENCODING};
use hyper::http::request::Parts;
use hyper::server::conn::http1 as server;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, duplex, split};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::api;
use crate::hosts::Access;
use crate::util::{IDLE, TIMEOUT, base64url_decode};

/// The bytes a tunnel message carries at most.
const CHUNK: usize = 16 * 1024;

/// The bytes that the HTTP server of a tunnel may hold between its WebSocket and its parser.
const BUFFER: usize = 64 * 1024;

/// Whether a link uses `method` on `path`, exactly (ADR-0301): the descriptor, the MCP sign-in a
/// link makes, and the MCP endpoint that linked agents call, each with the methods T3 serves it
/// with. T3 registers no other path under `/mcp`.
fn passes(method: &Method, path: &str) -> bool {
    match path {
        "/.well-known/t3/environment" => *method == Method::GET,
        REGISTER_PATH | DECISION_PATH | TOKEN_PATH => *method == Method::POST,
        // The MCP transport answers POST with requests, GET with its stream and DELETE with the end
        // of a session.
        "/mcp" => matches!(*method, Method::GET | Method::POST | Method::DELETE),
        _ => false,
    }
}

/// The path that registers a link's client, whose name the tunnel sets to its host's (ADR-0300).
const REGISTER_PATH: &str = "/oauth/mcp/register";

/// The path that exchanges a link's approved code for its token, which makes the session.
const TOKEN_PATH: &str = "/oauth/mcp/token";

/// The path of the decision that approves a link's sign-in, whose level the tunnel checks.
const DECISION_PATH: &str = "/oauth/mcp/decision";

/// The bytes of a link's request body that the tunnel reads at most, which no request needs more of.
const BODY: usize = 64 * 1024;

/// The name that a T3 client_id is signed for: the `n` of its payload. The payload is the part
/// before the dot, base64url of JSON; T3 verifies the signature, which this does not repeat. A
/// client_id of any other shape names nothing.
pub(crate) fn client_name_of(client_id: &str) -> Option<String> {
    let mut parts = client_id.split('.');
    let (body, _signature, extra) = (parts.next()?, parts.next()?, parts.next());
    if extra.is_some() {
        return None;
    }
    let payload: Value = serde_json::from_slice(&base64url_decode(body)?).ok()?;
    payload["n"].as_str().map(str::to_owned)
}

/// An error that a body or its reader reports.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Copies bytes both ways between `stream` and `socket`, as binary messages, until either side
/// closes or the peer stays silent for [`IDLE`]: this side pings every [`PING`].
pub(crate) async fn relay<T, S>(stream: T, socket: WebSocketStream<S>)
where
    T: AsyncRead + AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut reader, mut writer) = split(stream);
    let (mut sink, mut frames) = socket.split();
    let up = async {
        let mut buffer = vec![0u8; CHUNK];
        let mut ping = crate::util::pings();
        loop {
            tokio::select! {
                read = reader.read(&mut buffer) => match read {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if !sent(&mut sink, Message::Binary(buffer[..n].to_vec().into())).await {
                            break;
                        }
                    }
                },
                _ = ping.tick() => {
                    if !sent(&mut sink, Message::Ping(Vec::new().into())).await {
                        break;
                    }
                }
            }
        }
    };
    let down = async {
        // Any frame, a pong included, comes at least every PING from a peer that is there.
        while let Ok(Some(Ok(frame))) = tokio::time::timeout(IDLE, frames.next()).await {
            match frame {
                Message::Binary(bytes) => {
                    // A local end that stops reading ends the relay as a silent peer does.
                    if !matches!(
                        tokio::time::timeout(IDLE, writer.write_all(&bytes)).await,
                        Ok(Ok(()))
                    ) {
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

/// Sends `message` on `sink` unless the peer does not take it within [`IDLE`]; answers whether it was
/// sent. A peer that stops reading holds the relay no longer than a silent one.
async fn sent<S>(sink: &mut SplitSink<WebSocketStream<S>, Message>, message: Message) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    matches!(
        tokio::time::timeout(IDLE, sink.send(message)).await,
        Ok(Ok(()))
    )
}

/// Serves HTTP on the WebSocket `socket` of a host's tunnel, for T3 at `t3`, until the client
/// closes it: the requests for a link's paths reach T3, and the others get 403 (see the module).
/// `owner` is the host's name for the link and `level` the access it was invited at.
pub(crate) async fn serve<S>(
    socket: WebSocketStream<S>,
    t3: String,
    owner: String,
    level: Access,
    sign_ins: Arc<RwLock<()>>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (ours, theirs) = duplex(BUFFER);
    let http = server::Builder::new().serve_connection(
        TokioIo::new(theirs),
        service_fn(move |request| {
            forward(request, t3.clone(), owner.clone(), level, sign_ins.clone())
        }),
    );
    // The relay ends by itself once the HTTP connection has dropped its side and every answer byte
    // is sent, so an answer that ends the connection is not cut short. The connection ends when
    // the relay does, which drops its side.
    let _ = tokio::join!(relay(ours, socket), http);
}

/// Answers `request`: a path a link uses goes to T3 at `t3`, and any other is refused. A link's
/// registration takes the name of its host, and its decision goes on only when it asks for no
/// more than `level`.
pub(crate) async fn forward<B>(
    request: Request<B>,
    t3: String,
    owner: String,
    level: Access,
    sign_ins: Arc<RwLock<()>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, Infallible>
where
    B: Body<Data = Bytes> + Send + Sync + 'static,
    B::Error: Into<BoxError>,
{
    let path = request.uri().path();
    if !passes(request.method(), path) {
        return Ok(answer(StatusCode::FORBIDDEN));
    }
    let gated = path == TOKEN_PATH;
    let request = match path {
        REGISTER_PATH => named(request, &owner).await,
        DECISION_PATH => checked(request, &owner, level).await,
        // Read before the gate is held, so a body that never arrives holds no sign-in.
        TOKEN_PATH => exchanged(request, &owner).await,
        _ => Ok(request.map(|body| body.map_err(Into::<BoxError>::into).boxed())),
    };
    let request = match request {
        Ok(request) => request,
        Err(status) => return Ok(answer(status)),
    };
    let answered = if gated {
        // A token exchange makes the session, so it is held until T3 has answered: a revoke that
        // waits on the gate then ends that session too. T3 gets a bounded time to answer.
        let _in_flight = sign_ins.read().await;
        tokio::time::timeout(TIMEOUT, send(request, &t3))
            .await
            .ok()
            .flatten()
    } else {
        send(request, &t3).await
    };
    Ok(match answered {
        Some(response) => response.map(|body| body.boxed()),
        None => answer(StatusCode::BAD_GATEWAY),
    })
}

/// Reads a link's request body whole. A body over [`BODY`] bytes is too large, and one that does
/// not arrive within [`TIMEOUT`] is given up on.
async fn read<B>(body: B) -> Result<Bytes, StatusCode>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
    let collected = tokio::time::timeout(TIMEOUT, Limited::new(body, BODY).collect())
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    Ok(collected.to_bytes())
}

/// Reads a token exchange whole and passes it on when its client_id names the tunnel's owner. T3
/// makes the session under the name in the client_id, so an exchange of another pairing's code
/// would label a session with that pairing (ADR-0301). A body without exactly one client_id is
/// refused.
async fn exchanged<B>(
    request: Request<B>,
    owner: &str,
) -> Result<Request<BoxBody<Bytes, BoxError>>, StatusCode>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
    let (parts, body) = request.into_parts();
    let bytes = read(body).await?;
    let client = form_field(&bytes, "client_id").and_then(|id| client_name_of(&id));
    if client.as_deref() != Some(owner) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(resend(parts, bytes))
}

/// The value of the one field `key` of an application/x-www-form-urlencoded `body`; `None` when the
/// field is absent or repeated, or when the body is not a form.
pub(crate) fn form_field(body: &[u8], key: &str) -> Option<String> {
    let mut found = None;
    for pair in std::str::from_utf8(body).ok()?.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if form_text(name)? == key && found.replace(form_text(value)?).is_some() {
            return None;
        }
    }
    found
}

/// The text of a form component: `+` is a space, and `%XX` is the byte XX.
fn form_text(component: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(component.len());
    let mut rest = component.bytes();
    while let Some(byte) = rest.next() {
        bytes.push(match byte {
            b'+' => b' ',
            b'%' => {
                let high = (rest.next()? as char).to_digit(16)?;
                let low = (rest.next()? as char).to_digit(16)?;
                (high * 16 + low) as u8
            }
            other => other,
        });
    }
    String::from_utf8(bytes).ok()
}

/// `parts` with `bytes` as its body, whose length its headers then state.
fn resend(mut parts: Parts, bytes: Bytes) -> Request<BoxBody<Bytes, BoxError>> {
    parts.headers.remove(TRANSFER_ENCODING);
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    let body = Full::new(bytes).map_err(|never| match never {}).boxed();
    Request::from_parts(parts, body)
}

/// Reads the body of a decision and passes `request` on with it, when the decision is for the client
/// `owner` (its client_id names it) and asks for no more than `level`. A body that is no JSON,
/// names another client, or asks for an access that names no level is refused: the tunnel cannot
/// tell what it would grant, and T3 would label the session by the name in the client_id (ADR-0301).
pub(crate) async fn checked<B>(
    request: Request<B>,
    owner: &str,
    level: Access,
) -> Result<Request<BoxBody<Bytes, BoxError>>, StatusCode>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
    let (parts, body) = request.into_parts();
    let bytes = read(body).await?;
    let decision: Value = serde_json::from_slice(&bytes).map_err(|_| StatusCode::FORBIDDEN)?;
    let client = decision["authorization"]["client_id"]
        .as_str()
        .and_then(client_name_of);
    if client.as_deref() != Some(owner) {
        return Err(StatusCode::FORBIDDEN);
    }
    match &decision["decision"]["access"] {
        // T3's deny names no level and grants nothing; every approval must name one within `level`.
        Value::Null if decision["decision"]["_tag"] == "deny" => {}
        Value::String(asked) if Access::parse(asked).is_some_and(|asked| asked <= level) => {}
        _ => return Err(StatusCode::FORBIDDEN),
    }
    Ok(resend(parts, bytes))
}

/// Passes `request` on with its client named `owner`, the pairing name of the host it came from.
/// T3 names the session that the sign-in makes after the client, so `meta host revoke` finds that
/// session by its name (ADR-0300). A body that is no JSON object is refused.
pub(crate) async fn named<B>(
    request: Request<B>,
    owner: &str,
) -> Result<Request<BoxBody<Bytes, BoxError>>, StatusCode>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
    let (parts, body) = request.into_parts();
    let bytes = read(body).await?;
    let mut registration: Value =
        serde_json::from_slice(&bytes).map_err(|_| StatusCode::FORBIDDEN)?;
    let fields = registration.as_object_mut().ok_or(StatusCode::FORBIDDEN)?;
    fields.insert("client_name".to_owned(), Value::String(owner.to_owned()));
    let bytes = serde_json::to_vec(&registration).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(resend(parts, Bytes::from(bytes)))
}

/// Sends `request` to T3 at `t3` unchanged, over a connection that lasts as long as the body of
/// the answer, which streams back as it comes.
async fn send<B>(request: Request<B>, t3: &str) -> Option<Response<Incoming>>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
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
    crate::util::accept_each(listener, move |tcp| {
        open(tcp, address.clone(), token.clone())
    })
    .await;
}

/// Opens the tunnel to `address` for `tcp`. A peer that refuses or cannot be reached closes `tcp`.
async fn open(tcp: TcpStream, address: String, token: String) {
    let Ok(request) = crate::remote::handshake(
        &address,
        &[
            ("authorization", format!("Bearer {token}").as_str()),
            (api::TUNNEL_HEADER, "t3"),
        ],
    ) else {
        return;
    };
    let connected = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async(request)).await;
    if let Ok(Ok((socket, _))) = connected {
        relay(tcp, socket).await;
    }
}
