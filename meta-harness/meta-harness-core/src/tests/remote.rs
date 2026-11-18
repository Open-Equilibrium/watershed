use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::remote::{Host, connection_lost};

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_last_handle_closes_the_connection_to_the_service() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let service = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        while let Some(Ok(frame)) = socket.next().await {
            if matches!(frame, Message::Close(_)) {
                return;
            }
        }
    });
    let host = Host::connect_to(&url, "token", "test").await.unwrap();
    drop(host);
    tokio::time::timeout(Duration::from_secs(10), service)
        .await
        .expect("the service sees the connection end")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_dropped_before_its_answer_stops_waiting_for_it() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        while socket.send(Message::Ping(Vec::new().into())).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let host = Host::connect_to(&url, "token", "test").await.unwrap();
    for _ in 0..5 {
        let _ = tokio::time::timeout(Duration::from_millis(50), host.hosts()).await;
    }
    assert_eq!(host.waiting().await, 0);
}

async fn refusing_peer(reply: &'static [u8]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.is_empty() {
                break;
            }
        }
        writer.write_all(reply).await.unwrap();
    });
    url
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_of_another_api_version_refuses_the_connection_with_the_version_it_speaks() {
    let url = refusing_peer(
        b"HTTP/1.1 426 Upgrade Required\r\nmeta-harness-api: 2\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    )
    .await;
    let error = Host::connect_to(&url, "token", "test")
        .await
        .err()
        .expect("the connection is refused");
    assert_eq!(
        error,
        "Meta-Harness speaks API version 2; this client needs 1. Update Meta-Harness or Liquid."
    );
    assert!(!connection_lost(&error), "a refusal is not retried");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_device_is_refused_with_its_pairing_named_and_not_retried() {
    let url = refusing_peer(
        b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    )
    .await;
    let error = Host::connect_to(&url, "token", "test")
        .await
        .err()
        .expect("the connection is refused");
    assert_eq!(
        error,
        format!(
            "Meta-Harness at {url} no longer accepts this device's pairing; pair this device again."
        )
    );
    assert!(!connection_lost(&error), "a revoked pairing is not retried");
}

#[tokio::test]
async fn pairing_refuses_an_address_that_is_not_ws_before_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let error = crate::remote::pair(&address, "CODE", "test")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        "Enter the address as ws://HOST:PORT, as meta invite prints it."
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err()
    );
}
