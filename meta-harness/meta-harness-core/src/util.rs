//! Helpers the core shares: lower-case hex, locks that a panicked holder does not poison, and the
//! accept loop of every listener.

use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

/// How long a call waits for another process or host before it gives up: a T3 command, a connect
/// or a pairing (ADR-0288, ADR-0299).
pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);

/// How often a connection pings its peer, and how long a peer may stay silent before the connection
/// ends: a half-open peer answers no ping (ADR-0299).
pub(crate) const PING: Duration = Duration::from_secs(30);
pub(crate) const IDLE: Duration = Duration::from_secs(90);

/// The timer of a connection's pings: the first comes a [`PING`] after the connection starts. A ping
/// at the start races the teardown of short tunnel connections, which then lost their answers.
pub(crate) fn pings() -> tokio::time::Interval {
    tokio::time::interval_at(tokio::time::Instant::now() + PING, PING)
}

/// `bytes` in lower-case hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes of `text` in unpadded base64url, or `None` when it holds another character.
pub(crate) fn base64url_decode(text: &str) -> Option<Vec<u8>> {
    let (mut bits, mut count, mut bytes) = (0u32, 0u32, Vec::new());
    for character in text.bytes() {
        let value = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a' + 26,
            b'0'..=b'9' => character - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Some(bytes)
}

/// Locks `mutex`. A holder that panicked leaves the state it guards in place, so the lock is taken
/// anyway instead of failing every later caller.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `serve` on each connection that `listener` accepts, as a task of its own, until the task
/// ends.
pub(crate) async fn accept_each<F, Fut>(listener: TcpListener, serve: F)
where
    F: Fn(TcpStream) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    // The connections are tasks of this loop, so aborting the loop ends them too.
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => {
                    connections.spawn(serve(socket));
                }
                // Such as too many open files: the listener keeps listening.
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}
