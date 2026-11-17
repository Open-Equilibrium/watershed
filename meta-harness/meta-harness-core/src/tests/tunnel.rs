//! The tunnel checks the level of each decision a link sends (ADR-0301): a link asks for no more
//! than the level its host invited.

use std::time::Duration;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, StatusCode};
use serde_json::json;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::hosts::Access;
use crate::tunnel::{checked, relay};

/// The request a link sends with `body`, a decision of the access `access`.
fn decision(access: &str) -> Vec<u8> {
    json!({
        "authorization": {},
        "decision": {"_tag": "pairing-code", "access": access, "code": "CODE"},
    })
    .to_string()
    .into_bytes()
}

/// Whether the tunnel passes the decision `body` at the level `level`.
async fn passes(body: Vec<u8>, level: Access) -> Result<(), StatusCode> {
    let request = Request::post("/oauth/mcp/decision")
        .body(Full::new(Bytes::from(body)))
        .unwrap();
    checked(request, level).await.map(|_| ())
}

#[tokio::test]
async fn a_decision_asks_no_more_than_the_invited_level() {
    let invited = Access::ApprovalRequired;
    assert_eq!(
        passes(decision("auto"), invited).await,
        Err(StatusCode::FORBIDDEN)
    );
    assert_eq!(
        passes(decision("full-access"), invited).await,
        Err(StatusCode::FORBIDDEN)
    );
    assert_eq!(passes(decision("approval-required"), invited).await, Ok(()));
    assert_eq!(passes(decision("read-only"), invited).await, Ok(()));
    let deny = json!({"authorization": {}, "decision": {"_tag": "deny"}});
    assert_eq!(passes(deny.to_string().into_bytes(), invited).await, Ok(()));
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
    // The far end of the WebSocket stays open and never answers, as a half-open peer does.
    let (socket_end, _held) = tokio::io::duplex(1024);
    let socket = WebSocketStream::from_raw_socket(socket_end, Role::Client, None).await;
    let (tcp, _tcp_far) = tokio::io::duplex(1024);
    tokio::time::timeout(Duration::from_secs(600), relay(tcp, socket))
        .await
        .expect("the relay ends once its peer has been silent");
}
