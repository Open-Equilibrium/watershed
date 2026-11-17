//! The tunnel checks the level of each decision a link sends (ADR-0301): a link asks for no more
//! than the level its host invited.

use std::sync::Arc;
use std::time::Duration;

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

/// The name of the host these tests stand for, and the client_id that T3 signs for it.
const OWNER: &str = "host-1a2b";
const CLIENT_ID: &str = "eyJ2IjoxLCJuIjoiaG9zdC0xYTJiIiwiciI6W119.signature";

/// A decision of the access `access` for the client that `CLIENT_ID` names.
fn decision(access: &str) -> Vec<u8> {
    json!({
        "authorization": {"client_id": CLIENT_ID},
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
    checked(request, OWNER, level).await.map(|_| ())
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
    let deny = json!({"authorization": {"client_id": CLIENT_ID}, "decision": {"_tag": "deny"}});
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

#[test]
fn a_client_id_names_the_pairing_it_was_registered_for() {
    // The payload of T3's signed client_id: base64url of {"v":1,"n":"host-1a2b","r":[]}.
    let client_id = "eyJ2IjoxLCJuIjoiaG9zdC0xYTJiIiwiciI6W119.signature";
    assert_eq!(client_name_of(client_id).as_deref(), Some("host-1a2b"));
    assert_eq!(client_name_of("no-signature"), None);
    assert_eq!(
        client_name_of("eyJ2IjoxLCJuIjoiaG9zdC0xYTJiIiwiciI6W119.a.b"),
        None
    );
}

#[tokio::test]
async fn a_decision_for_another_client_is_refused() {
    let request = |body: serde_json::Value| {
        Request::post("/oauth/mcp/decision")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap()
    };
    // A client_id T3 signed for another name, such as the one an older pairing registered with.
    let other = json!({"authorization": {"client_id": "eyJ2IjoxLCJuIjoib3RoZXIifQ.signature"},
        "decision": {"_tag": "deny"}});
    assert_eq!(
        checked(request(other), OWNER, Access::Auto)
            .await
            .map(|_| ()),
        Err(StatusCode::FORBIDDEN)
    );
    let anonymous = json!({"authorization": {}, "decision": {"_tag": "deny"}});
    assert_eq!(
        checked(request(anonymous), OWNER, Access::Auto)
            .await
            .map(|_| ()),
        Err(StatusCode::FORBIDDEN)
    );
}

#[tokio::test(start_paused = true)]
async fn a_token_exchange_whose_body_never_arrives_holds_no_sign_in_gate() {
    // T3 accepts the connection and answers nothing, so an exchange that reaches it waits there.
    let t3 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let t3_address = t3.local_addr().unwrap().to_string();
    let gate = Arc::new(RwLock::new(()));
    // A body that is announced but never arrives.
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

#[tokio::test]
async fn a_decision_without_a_level_passes_only_as_a_refusal() {
    // T3's approvals carry a level; only its deny names none, and grants nothing.
    let approval = json!({
        "authorization": {"client_id": CLIENT_ID},
        "decision": {"_tag": "pairing-code", "code": "CODE"},
    });
    assert_eq!(
        passes(approval.to_string().into_bytes(), Access::Auto).await,
        Err(StatusCode::FORBIDDEN)
    );
    let session = json!({
        "authorization": {"client_id": CLIENT_ID},
        "decision": {"_tag": "browser-session", "csrfToken": "token"},
    });
    assert_eq!(
        passes(session.to_string().into_bytes(), Access::Auto).await,
        Err(StatusCode::FORBIDDEN)
    );
    let deny = json!({"authorization": {"client_id": CLIENT_ID}, "decision": {"_tag": "deny"}});
    assert_eq!(
        passes(deny.to_string().into_bytes(), Access::Auto).await,
        Ok(())
    );
}
