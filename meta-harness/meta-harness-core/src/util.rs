//! Helpers the core shares: lower-case hex, locks that a panicked holder does not poison, and the
//! accept loop of every listener.

use std::future::Future;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

/// How long a call waits for another process or host before it gives up: a T3 command, a connect
/// or a pairing (ADR-0288, ADR-0299).
pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);

/// `bytes` in lower-case hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
    loop {
        match listener.accept().await {
            Ok((socket, _)) => drop(tokio::spawn(serve(socket))),
            // Such as too many open files: the listener keeps listening.
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}
