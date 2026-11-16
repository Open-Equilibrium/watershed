//! The client's connection to the Meta-Harness service: a client that lets go of its connection
//! closes it, so the service ends the connection too.

use std::time::Duration;

use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::remote::Host;

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
