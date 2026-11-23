use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) const PING: Duration = Duration::from_secs(30);
pub(crate) const IDLE: Duration = Duration::from_secs(90);

pub(crate) fn pings() -> tokio::time::Interval {
    tokio::time::interval_at(tokio::time::Instant::now() + PING, PING)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn base64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (index, byte)| {
            bits | u32::from(*byte) << (16 - 8 * index)
        });
        for index in 0..=chunk.len() {
            text.push(char::from(
                ALPHABET[(bits >> (18 - 6 * index) & 63) as usize],
            ));
        }
    }
    text
}

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

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) async fn until(limit: Duration, poll: Duration, done: impl Fn() -> bool) -> bool {
    tokio::time::timeout(limit, async {
        while !done() {
            tokio::time::sleep(poll).await;
        }
    })
    .await
    .is_ok()
}

pub(crate) async fn accept_each<F, Fut>(listener: TcpListener, serve: F)
where
    F: Fn(TcpStream) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => {
                    connections.spawn(serve(socket));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}
