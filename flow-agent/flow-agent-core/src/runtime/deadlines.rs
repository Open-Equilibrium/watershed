use crate::runtime::types::RuntimeError;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) const HTTP_CONNECT_DEADLINE: Duration = Duration::from_secs(10);
pub(crate) const HTTP_HEADER_DEADLINE: Duration = Duration::from_secs(30);
pub(crate) const AUTH_BODY_DEADLINE: Duration = Duration::from_secs(30);
pub(crate) const AUTH_OVERALL_DEADLINE: Duration = Duration::from_secs(60);
pub(crate) const DEVICE_POLL_OVERALL_DEADLINE: Duration = Duration::from_secs(15 * 60);
pub(crate) const RESPONSES_IDLE_DEADLINE: Duration = Duration::from_secs(120);
pub(crate) const RESPONSES_OVERALL_DEADLINE: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HttpDeadlines {
    pub(crate) connect: Duration,
    pub(crate) header: Duration,
    pub(crate) read: Duration,
    pub(crate) overall: Duration,
}

pub(crate) const AUTH_HTTP_DEADLINES: HttpDeadlines = HttpDeadlines {
    connect: HTTP_CONNECT_DEADLINE,
    header: HTTP_HEADER_DEADLINE,
    read: AUTH_BODY_DEADLINE,
    overall: AUTH_OVERALL_DEADLINE,
};

pub(crate) const RESPONSES_HTTP_DEADLINES: HttpDeadlines = HttpDeadlines {
    connect: HTTP_CONNECT_DEADLINE,
    header: HTTP_HEADER_DEADLINE,
    read: RESPONSES_IDLE_DEADLINE,
    overall: RESPONSES_OVERALL_DEADLINE,
};

pub(crate) fn build_http_client(
    deadlines: HttpDeadlines,
) -> Result<reqwest::Client, reqwest::Error> {
    configure_http_client(reqwest::Client::builder(), deadlines)
}

fn configure_http_client(
    builder: reqwest::ClientBuilder,
    deadlines: HttpDeadlines,
) -> Result<reqwest::Client, reqwest::Error> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    builder
        .connect_timeout(deadlines.connect)
        .read_timeout(deadlines.read)
        .timeout(deadlines.overall)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

#[cfg(test)]
pub(crate) fn build_http_client_from_builder(
    builder: reqwest::ClientBuilder,
    deadlines: HttpDeadlines,
) -> Result<reqwest::Client, reqwest::Error> {
    configure_http_client(builder, deadlines)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeadlineElapsed;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AwaitInterruption {
    Cancelled,
    DeadlineElapsed,
}

pub(crate) async fn await_deadline<F>(
    deadline: Duration,
    future: F,
) -> Result<F::Output, DeadlineElapsed>
where
    F: Future,
{
    tokio::time::timeout(deadline, future)
        .await
        .map_err(|_| DeadlineElapsed)
}

pub(crate) async fn await_deadline_or_cancellation<F>(
    deadline: Duration,
    cancelled: &AtomicBool,
    future: F,
) -> Result<F::Output, AwaitInterruption>
where
    F: Future,
{
    const CANCELLATION_POLL: Duration = Duration::from_millis(10);
    let started = tokio::time::Instant::now();
    let deadline_at = started
        .checked_add(deadline)
        .ok_or(AwaitInterruption::DeadlineElapsed)?;
    let mut future = Box::pin(future);
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AwaitInterruption::Cancelled);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline_at {
            return Err(AwaitInterruption::DeadlineElapsed);
        }
        let quantum = CANCELLATION_POLL.min(deadline_at.saturating_duration_since(now));
        match tokio::time::timeout(quantum, future.as_mut()).await {
            Ok(output) => return Ok(output),
            Err(_) => continue,
        }
    }
}

pub(crate) fn block_on_network<F>(future: F) -> Result<F::Output, RuntimeError>
where
    F: Future,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|_| RuntimeError::Protocol("network runtime construction failed".to_owned()))?;
    Ok(runtime.block_on(future))
}
