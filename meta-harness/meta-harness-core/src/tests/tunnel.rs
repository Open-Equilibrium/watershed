use std::sync::Arc;
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::stream::{Pending, pending};
use http_body_util::{Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::{Request, StatusCode};
use serde_json::json;
use tokio::sync::RwLock;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::hosts::Access;
use crate::tunnel::{checked, client_name_of, form_field, forward, relay};

const OWNER: &str = "host-1a2b";
const CLIENT_ID: &str = "eyJ2IjoxLCJuIjoiaG9zdC0xYTJiIiwiciI6W119.signature";

fn decision(access: &str) -> Vec<u8> {
    json!({
        "authorization": {"client_id": CLIENT_ID},
        "decision": {"_tag": "pairing-code", "access": access, "code": "CODE"},
    })
    .to_string()
    .into_bytes()
}

async fn passes(body: Vec<u8>, level: Access) -> Result<(), StatusCode> {
    let request = Request::post("/oauth/mcp/decision")
        .body(Full::new(Bytes::from(body)))
        .unwrap();
    checked(request, OWNER, level).await.map(|_| ())
}

fn body(value: serde_json::Value) -> Vec<u8> {
    value.to_string().into_bytes()
}

#[tokio::test]
async fn a_decision_passes_only_within_the_level_its_host_was_invited_at() {
    let deny = json!({"authorization": {"client_id": CLIENT_ID}, "decision": {"_tag": "deny"}});
    let no_level = json!({"authorization": {"client_id": CLIENT_ID},
        "decision": {"_tag": "pairing-code", "code": "CODE"}});
    let session = json!({"authorization": {"client_id": CLIENT_ID},
        "decision": {"_tag": "browser-session", "csrfToken": "token"}});
    let cases = [
        (
            decision("auto"),
            Access::ApprovalRequired,
            Err(StatusCode::FORBIDDEN),
        ),
        (
            decision("full-access"),
            Access::ApprovalRequired,
            Err(StatusCode::FORBIDDEN),
        ),
        (
            decision("approval-required"),
            Access::ApprovalRequired,
            Ok(()),
        ),
        (decision("read-only"), Access::ApprovalRequired, Ok(())),
        (body(deny.clone()), Access::ApprovalRequired, Ok(())),
        (
            body(
                json!({"decision": {"_tag": "pairing-code", "access": "read-only", "code": "CODE"}}),
            ),
            Access::ApprovalRequired,
            Err(StatusCode::FORBIDDEN),
        ),
        (body(no_level), Access::Auto, Err(StatusCode::FORBIDDEN)),
        (body(session), Access::Auto, Err(StatusCode::FORBIDDEN)),
        (body(deny), Access::Auto, Ok(())),
    ];
    for (request, level, expected) in cases {
        assert_eq!(passes(request, level).await, expected);
    }
}

#[tokio::test]
async fn a_decision_the_tunnel_cannot_read_is_refused() {
    let invited = Access::Auto;
    assert_eq!(
        passes(b"not json".to_vec(), invited).await,
        Err(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        passes(decision("root"), invited).await,
        Err(StatusCode::FORBIDDEN)
    );
    let large = vec![b' '; 64 * 1024 + 1];
    assert_eq!(
        passes(large, invited).await,
        Err(StatusCode::PAYLOAD_TOO_LARGE)
    );
}

#[tokio::test(start_paused = true)]
async fn a_relay_ends_when_its_peer_goes_silent() {
    let (socket_end, _held) = tokio::io::duplex(1024);
    let socket = WebSocketStream::from_raw_socket(socket_end, Role::Client, None).await;
    let (tcp, _tcp_far) = tokio::io::duplex(1024);
    tokio::time::timeout(Duration::from_secs(600), relay(tcp, socket))
        .await
        .expect("the relay ends once its peer has been silent");
}

#[test]
fn a_client_id_names_the_pairing_it_was_registered_for() {
    let client_id = "eyJ2IjoxLCJuIjoiaG9zdC0xYTJiIiwiciI6W119.signature";
    assert_eq!(client_name_of(client_id).as_deref(), Some("host-1a2b"));
    assert_eq!(client_name_of("no-signature"), None);
    assert_eq!(
        client_name_of("eyJ2IjoxLCJuIjoiaG9zdC0xYTJiIiwiciI6W119.a.b"),
        None
    );
}

#[tokio::test(start_paused = true)]
async fn a_token_exchange_whose_body_never_arrives_holds_no_sign_in_gate() {
    let t3 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let t3_address = t3.local_addr().unwrap().to_string();
    let gate = Arc::new(RwLock::new(()));
    let withheld: StreamBody<Pending<Result<Frame<Bytes>, std::io::Error>>> =
        StreamBody::new(pending());
    let request = Request::post("/oauth/mcp/token").body(withheld).unwrap();
    let (answer, revoked) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(60),
            forward(
                request,
                t3_address,
                OWNER.to_owned(),
                Access::Auto,
                gate.clone()
            ),
        ),
        tokio::time::timeout(Duration::from_secs(5), gate.write()),
    );
    assert!(
        revoked.is_ok(),
        "a revoke gets the gate while the body is pending"
    );
    let answered = answer
        .expect("the exchange ends")
        .expect("the tunnel answers");
    assert_eq!(answered.status(), StatusCode::REQUEST_TIMEOUT);
    drop(t3);
}

#[test]
fn a_token_form_names_one_client_id_however_it_is_encoded() {
    let encoded = "grant_type=authorization_code&client_id=eyJ2IjoxLCJuIjoiaG9zdC0xYTJiIiwiciI6W119%2Esignature";
    assert_eq!(
        form_field(encoded.as_bytes(), "client_id").as_deref(),
        Some(CLIENT_ID)
    );
    let plus = "client_id=a+b";
    assert_eq!(
        form_field(plus.as_bytes(), "client_id").as_deref(),
        Some("a b")
    );
    let repeated = "client_id=a&client_id=b";
    assert_eq!(form_field(repeated.as_bytes(), "client_id"), None);
    assert_eq!(form_field(b"code=x", "client_id"), None);
}

#[tokio::test(start_paused = true)]
async fn a_relay_ends_when_its_local_end_stops_reading() {
    let (socket_end, far_end) = tokio::io::duplex(1024);
    let socket = WebSocketStream::from_raw_socket(socket_end, Role::Client, None).await;
    let mut peer = WebSocketStream::from_raw_socket(far_end, Role::Server, None).await;
    peer.send(tokio_tungstenite::tungstenite::Message::Binary(
        vec![7u8; 100].into(),
    ))
    .await
    .unwrap();
    let (tcp, _tcp_far) = tokio::io::duplex(16);
    tokio::time::timeout(Duration::from_secs(600), relay(tcp, socket))
        .await
        .expect("the relay ends once its local end has stopped reading");
}
