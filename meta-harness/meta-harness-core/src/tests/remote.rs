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

#[tokio::test]
async fn a_stream_error_ends_its_answer_even_while_the_producer_stays_open() {
    let (out, mut sent) = tokio::sync::mpsc::channel(8);
    let items = futures_util::stream::iter([Err::<serde_json::Value, String>("broke".to_owned())])
        .chain(futures_util::stream::pending());
    tokio::time::timeout(
        Duration::from_secs(5),
        crate::service::answer(&out, 7, serde_json::Value::Null, items),
    )
    .await
    .expect("the answer ends after its error");
    let mut texts = Vec::new();
    while let Ok(Message::Text(text)) = sent.try_recv() {
        texts.push(text.to_string());
    }
    assert_eq!(texts.len(), 2, "the ok reply and the error only");
    assert!(texts[1].contains("broke"));
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
async fn a_refused_connection_says_why_and_is_not_retried() {
    type Refusal = (&'static [u8], fn(&str) -> String);
    let cases: [Refusal; 2] = [
        (
            b"HTTP/1.1 426 Upgrade Required\r\nmeta-harness-api: 2\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            |_| "Meta-Harness speaks API version 2; this client needs 1. Update Meta-Harness or Liquid.".to_owned(),
        ),
        (
            b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            |url| format!("Meta-Harness at {url} no longer accepts this device's pairing; pair this device again."),
        ),
    ];
    for (reply, expected) in cases {
        let url = refusing_peer(reply).await;
        let error = Host::connect_to(&url, "token", "test")
            .await
            .err()
            .expect("the connection is refused");
        assert_eq!(error, expected(&url));
        assert!(
            !connection_lost(&error),
            "a refusal is not retried: {error}"
        );
    }
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
