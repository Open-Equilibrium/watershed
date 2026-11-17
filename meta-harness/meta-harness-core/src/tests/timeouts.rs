//! Calls that wait on another process or host give up after a timeout, so a silent T3 or peer
//! cannot stall a link (ADR-0288, ADR-0299).

use std::time::Duration;

use tokio::net::TcpListener;

use crate::remote::Host;

/// Long enough that a call still waiting when it runs out has certainly given up.
const GUARD: Duration = Duration::from_secs(600);

#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn a_t3_command_that_never_finishes_gives_up() {
    use std::os::unix::fs::PermissionsExt;
    // A stand-in for the T3 executable that never exits.
    // A fresh directory under a random name, created exclusively, so no path can be planted for it.
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let dir = std::env::temp_dir().join(format!("mh-timeout-{name}"));
    std::fs::create_dir(&dir).unwrap();
    let script = dir.join("t3");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&script)
        .unwrap();
    std::io::Write::write_all(&mut file, b"#!/bin/sh\nsleep 600\n").unwrap();
    drop(file);
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let answer = tokio::time::timeout(GUARD, crate::t3::links(&script, &dir))
        .await
        .expect("the command gives up");
    let _ = std::fs::remove_dir_all(&dir);
    let error = answer.err().expect("it fails");
    assert!(error.contains("longer than"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_answers_no_connection_is_given_up_on() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    // Accepts every connection and answers none of them.
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let answer = tokio::time::timeout(GUARD, Host::connect_to(&url, "token", "test"))
        .await
        .expect("the connection gives up");
    assert!(answer.is_err(), "a silent peer is no connection");
}
