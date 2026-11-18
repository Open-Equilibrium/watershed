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

const CHUNK: usize = 16 * 1024;

const BUFFER: usize = 64 * 1024;

fn passes(method: &Method, path: &str) -> bool {
    match path {
        "/.well-known/t3/environment" => *method == Method::GET,
        REGISTER_PATH | DECISION_PATH | TOKEN_PATH => *method == Method::POST,
        "/mcp" => matches!(*method, Method::GET | Method::POST | Method::DELETE),
        _ => false,
    }
}

const REGISTER_PATH: &str = "/oauth/mcp/register";

const TOKEN_PATH: &str = "/oauth/mcp/token";

const DECISION_PATH: &str = "/oauth/mcp/decision";

const BODY: usize = 64 * 1024;

pub(crate) fn client_name_of(client_id: &str) -> Option<String> {
    let mut parts = client_id.split('.');
    let (body, _signature, extra) = (parts.next()?, parts.next()?, parts.next());
    if extra.is_some() {
        return None;
    }
    let payload: Value = serde_json::from_slice(&base64url_decode(body)?).ok()?;
    payload["n"].as_str().map(str::to_owned)
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

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
        while let Ok(Some(Ok(frame))) = tokio::time::timeout(IDLE, frames.next()).await {
            match frame {
                Message::Binary(bytes) => {
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

async fn sent<S>(sink: &mut SplitSink<WebSocketStream<S>, Message>, message: Message) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    matches!(
        tokio::time::timeout(IDLE, sink.send(message)).await,
        Ok(Ok(()))
    )
}

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
    let _ = tokio::join!(relay(ours, socket), http);
}

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
        TOKEN_PATH => exchanged(request, &owner).await,
        _ => Ok(request.map(|body| body.map_err(Into::<BoxError>::into).boxed())),
    };
    let request = match request {
        Ok(request) => request,
        Err(status) => return Ok(answer(status)),
    };
    let answered = if gated {
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

fn resend(mut parts: Parts, bytes: Bytes) -> Request<BoxBody<Bytes, BoxError>> {
    parts.headers.remove(TRANSFER_ENCODING);
    parts
        .headers
        .insert(CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    let body = Full::new(bytes).map_err(|never| match never {}).boxed();
    Request::from_parts(parts, body)
}

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
        Value::Null if decision["decision"]["_tag"] == "deny" => {}
        Value::String(asked) if Access::parse(asked).is_some_and(|asked| asked <= level) => {}
        _ => return Err(StatusCode::FORBIDDEN),
    }
    Ok(resend(parts, bytes))
}

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

fn answer(status: StatusCode) -> Response<BoxBody<Bytes, hyper::Error>> {
    let mut response = Response::new(
        Empty::<Bytes>::new()
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    response
}

pub(crate) async fn listen(listener: TcpListener, address: String, token: String) {
    crate::util::accept_each(listener, move |tcp| {
        open(tcp, address.clone(), token.clone())
    })
    .await;
}

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
