use crate::runtime::deadlines::{
    AUTH_HTTP_DEADLINES, DeadlineElapsed, HttpDeadlines, RESPONSES_HTTP_DEADLINES, await_deadline,
    block_on_network, build_http_client_from_builder,
};
use reqwest::dns::{Name, Resolve, Resolving};
use std::{
    future::Future,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub(super) type ScriptedHttpServer = (
    String,
    mpsc::Receiver<()>,
    mpsc::Sender<Vec<u8>>,
    mpsc::Receiver<()>,
    thread::JoinHandle<()>,
);

pub(super) fn spawn_scripted_http_server(path: &str) -> ScriptedHttpServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("scripted HTTP server binds");
    let endpoint = format!("http://{}{path}", listener.local_addr().unwrap());
    let (connected, connected_rx) = mpsc::sync_channel(0);
    let (writes, writes_rx) = mpsc::channel::<Vec<u8>>();
    let (written, written_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one scripted HTTP request");
        connected.send(()).expect("request reports");
        drain_http_request(&mut stream);
        for bytes in writes_rx {
            stream.write_all(&bytes).expect("scripted response writes");
            stream.flush().expect("scripted response flushes");
            written.send(()).expect("scripted write reports");
        }
    });
    (endpoint, connected_rx, writes, written_rx, server)
}

pub(super) fn send_scripted_http_bytes(
    writes: &mpsc::Sender<Vec<u8>>,
    written: &mpsc::Receiver<()>,
    bytes: impl Into<Vec<u8>>,
) {
    writes.send(bytes.into()).expect("scripted bytes send");
    written
        .recv_timeout(Duration::from_secs(2))
        .expect("scripted bytes are written");
}

fn drain_http_request(stream: &mut TcpStream) {
    const MAX_REQUEST_BYTES: usize = 64 * 1024;
    let mut request = vec![0_u8; MAX_REQUEST_BYTES];
    assert!(
        stream.read(&mut request).expect("scripted request reads") > 0,
        "scripted request is empty"
    );
}

#[derive(Debug)]
struct PendingResolver;

impl Resolve for PendingResolver {
    fn resolve(&self, _name: Name) -> Resolving {
        Box::pin(std::future::pending())
    }
}

fn assert_configured_connect_deadline(selected: HttpDeadlines, expected: Duration) {
    assert_eq!(selected.connect, expected);
    block_on_paused_network(async move {
        let client = build_http_client_from_builder(
            reqwest::Client::builder().dns_resolver(PendingResolver),
            HttpDeadlines {
                connect: selected.connect,
                header: selected.connect + Duration::from_secs(1),
                read: selected.connect + Duration::from_secs(1),
                overall: selected.connect + Duration::from_secs(1),
            },
        )
        .expect("client builds");
        let request =
            tokio::spawn(async move { client.get("http://pending.invalid").send().await });
        assert_pending(&request).await;
        tokio::time::advance(selected.connect - Duration::from_nanos(1)).await;
        assert_pending(&request).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        let result = expect_ready(request).await;
        assert!(result.is_err());
    });
}

pub(super) fn block_on_paused_network<F>(future: F) -> F::Output
where
    F: Future,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("network runtime builds");
    runtime.block_on(future)
}

pub(super) async fn assert_pending<T>(future: &tokio::task::JoinHandle<T>) {
    tokio::task::yield_now().await;
    assert!(!future.is_finished());
}

pub(super) async fn settle_pending<T>(future: &tokio::task::JoinHandle<T>) {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert!(!future.is_finished());
}

pub(super) async fn expect_ready<T>(future: tokio::task::JoinHandle<T>) -> T {
    for _ in 0..16 {
        if future.is_finished() {
            return future.await.expect("deadline task completes");
        }
        tokio::task::yield_now().await;
    }
    panic!("future remains pending at its exact deadline")
}

#[test]
fn auth_connect_deadline() {
    assert_configured_connect_deadline(AUTH_HTTP_DEADLINES, Duration::from_secs(10));
}

#[test]
fn responses_connect_deadline() {
    assert_configured_connect_deadline(RESPONSES_HTTP_DEADLINES, Duration::from_secs(10));
}

#[test]
fn elapsed_deadline_cancels_the_in_flight_future_once() {
    struct PendingGuard(Arc<AtomicBool>);

    impl Drop for PendingGuard {
        fn drop(&mut self) {
            assert!(!self.0.swap(true, Ordering::SeqCst));
        }
    }

    let cancelled = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&cancelled);
    let result = block_on_network(async move {
        let guard = PendingGuard(observed);
        let _ = &guard;
        await_deadline(Duration::from_millis(1), std::future::pending::<()>()).await
    });

    assert!(matches!(result, Ok(Err(DeadlineElapsed))));
    assert!(cancelled.load(Ordering::SeqCst));
}
