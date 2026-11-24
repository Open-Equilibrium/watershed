use std::time::Duration;

use tokio::net::TcpListener;

use crate::remote::{Host, connection_lost};

const GUARD: Duration = Duration::from_secs(600);

#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn a_t3_command_that_never_finishes_gives_up() {
    let dir = super::Dir::new();
    let script = dir.join("t3");
    super::executable(&script, "#!/bin/sh\nsleep 600\n");
    let answer = tokio::time::timeout(GUARD, crate::t3::links(&script, &dir))
        .await
        .expect("the command gives up");
    let error = answer.err().expect("it fails");
    assert!(error.contains("longer than"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_answers_no_connection_is_given_up_on() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let answer = tokio::time::timeout(GUARD, Host::connect_to(&url, "token", "test"))
        .await
        .expect("the connection gives up");
    let error = answer.err().expect("a silent peer is no connection");
    assert!(connection_lost(&error), "{error}");
}

#[tokio::test]
async fn a_peer_that_goes_silent_after_connecting_fails_its_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            if let Ok(connection) = tokio_tungstenite::accept_async(socket).await {
                held.push(connection);
            }
        }
    });
    let host = Host::connect_to(&url, "token", "test").await.unwrap();
    tokio::time::pause();
    let answer = tokio::time::timeout(GUARD, host.status())
        .await
        .expect("the call gives up");
    let error = answer.expect_err("a silent peer fails the call");
    assert!(connection_lost(&error), "{error}");
}
