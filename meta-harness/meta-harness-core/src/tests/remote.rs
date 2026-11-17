//! The client's connection to the Meta-Harness service: a client that lets go of its connection
//! closes it, so the service ends the connection too.

use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::remote::{Host, connection_lost};

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_last_handle_closes_the_connection_to_the_service() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    // Reads until the client closes the connection, which a service sees as a close frame.
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
async fn a_service_of_another_api_version_refuses_the_connection_with_the_version_it_speaks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    // Answers the handshake as a service of API version 2 does.
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.is_empty() {
                break;
            }
        }
        writer
            .write_all(
                b"HTTP/1.1 426 Upgrade Required\r\nmeta-harness-api: 2\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    });
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    // Refuses the handshake as a service does a token it no longer knows.
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.is_empty() {
                break;
            }
        }
        writer
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    });
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
