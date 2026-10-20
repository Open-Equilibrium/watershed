use crate::runtime::deadlines::{
    AUTH_HTTP_DEADLINES, DeadlineElapsed, HttpDeadlines, RESPONSES_HTTP_DEADLINES, await_deadline,
    block_on_network, build_http_client_from_builder,
};
use reqwest::dns::{Name, Resolve, Resolving};
use std::{
    collections::HashSet,
    future::Future,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Condvar, Mutex,
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

type SystemLookup = Arc<dyn Fn(&str) -> std::io::Result<reqwest::dns::Addrs> + Send + Sync>;

#[derive(Default)]
struct LookupGate {
    released: HashSet<String>,
    release_all: bool,
}

struct HeldSystemLookups {
    gate: Arc<(Mutex<LookupGate>, Condvar)>,
    lookup: SystemLookup,
    started: mpsc::Receiver<String>,
    completed: mpsc::Receiver<String>,
    commands: Vec<thread::JoinHandle<()>>,
}

impl HeldSystemLookups {
    fn new() -> Self {
        let gate = Arc::new((Mutex::new(LookupGate::default()), Condvar::new()));
        let held = Arc::clone(&gate);
        let (started, started_rx) = mpsc::channel();
        let (completed, completed_rx) = mpsc::channel();
        let lookup = Arc::new(move |name: &str| {
            started.send(name.to_owned()).unwrap();
            let (lock, wake) = &*held;
            let state = lock.lock().unwrap();
            let (state, guard) = wake
                .wait_timeout_while(state, Duration::from_secs(20), |state| {
                    !state.release_all && !state.released.contains(name)
                })
                .unwrap();
            assert!(
                !guard.timed_out(),
                "held lookup cleanup did not release {name}"
            );
            drop(state);
            let _ = completed.send(name.to_owned());
            Err(std::io::Error::other("controlled system lookup completed"))
        });
        Self {
            gate,
            lookup,
            started: started_rx,
            completed: completed_rx,
            commands: Vec::new(),
        }
    }

    fn command(
        &mut self,
        name: &str,
        authentication: bool,
        cancelled: Arc<AtomicBool>,
        deadlines: HttpDeadlines,
    ) -> mpsc::Receiver<Result<(), crate::runtime::types::RuntimeError>> {
        let endpoint = format!("http://{name}/lookup");
        let lookup = Arc::clone(&self.lookup);
        let (returned, result) = mpsc::channel();
        self.commands.push(thread::spawn(move || {
            let result = crate::runtime::deadlines::with_system_lookup(lookup, || {
                if authentication {
                    crate::runtime::auth::post_json_with_deadlines(
                        &endpoint,
                        &serde_json::json!({}),
                        16,
                        deadlines,
                    )
                    .map(|_| ())
                } else {
                    let credential = crate::runtime::oauth_credential::CredentialRecord {
                        credential_type: "oauth".to_owned(),
                        access: "access-fixture".to_owned(),
                        refresh: "refresh-fixture".to_owned(),
                        expires: 1,
                        account_id: "account-fixture".to_owned(),
                        is_fedramp: false,
                    };
                    crate::runtime::openai_codex::request_responses_at_with_deadlines_and_cancellation(
                        &endpoint,
                        &credential,
                        &serde_json::json!({}),
                        deadlines,
                        &cancelled,
                    )
                    .map(|_| ())
                }
            });
            let _ = returned.send(result);
        }));
        result
    }

    fn release(&self, name: &str) {
        let (lock, wake) = &*self.gate;
        lock.lock().unwrap().released.insert(name.to_owned());
        wake.notify_all();
        assert_eq!(
            self.completed.recv_timeout(Duration::from_secs(2)).unwrap(),
            name
        );
    }
}

impl Drop for HeldSystemLookups {
    fn drop(&mut self) {
        let (lock, wake) = &*self.gate;
        lock.lock().unwrap().release_all = true;
        wake.notify_all();
        for command in self.commands.drain(..) {
            let _ = command.join();
        }
    }
}

fn lookup_deadlines() -> HttpDeadlines {
    HttpDeadlines {
        connect: Duration::from_secs(5),
        header: Duration::from_millis(100),
        read: Duration::from_secs(5),
        overall: Duration::from_secs(10),
    }
}

fn assert_lookup_command_error(
    result: mpsc::Receiver<Result<(), crate::runtime::types::RuntimeError>>,
    authentication: bool,
    cancellation: bool,
) {
    use crate::runtime::types::RuntimeError;
    let error = result
        .recv_timeout(Duration::from_secs(2))
        .expect("synchronous command must return before held system lookup completes")
        .expect_err("lookup is interrupted");
    if authentication {
        assert_eq!(error.to_string(), "authentication protocol failure");
    } else if cancellation {
        assert!(matches!(error, RuntimeError::Cancelled));
    } else {
        assert!(
            error
                .to_string()
                .contains("Responses header deadline elapsed")
        );
        assert!(!error.provider_failure().unwrap().is_definitive());
    }
}

#[test]
fn system_dns_timeout_and_cancellation_return_before_lookup_completion() {
    let mut held = HeldSystemLookups::new();
    for (name, authentication, cancellation) in [
        ("auth-timeout.invalid", true, false),
        ("provider-timeout.invalid", false, false),
        ("provider-cancellation.invalid", false, true),
    ] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let result = held.command(
            name,
            authentication,
            Arc::clone(&cancelled),
            lookup_deadlines(),
        );
        assert_eq!(
            held.started.recv_timeout(Duration::from_secs(2)).unwrap(),
            name
        );
        if cancellation {
            cancelled.store(true, Ordering::Release);
        }
        assert_lookup_command_error(result, authentication, cancellation);
        assert!(matches!(
            held.completed.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        held.release(name);
    }
}

#[test]
fn system_dns_capacity_is_shared_and_reclaimed_only_after_lookup_completion() {
    let mut held = HeldSystemLookups::new();
    let mut results = Vec::new();
    for index in 0..32 {
        let authentication = index % 2 == 0;
        let name = format!("mixed-{index}.invalid");
        let cancelled = Arc::new(AtomicBool::new(false));
        results.push((
            held.command(
                &name,
                authentication,
                Arc::clone(&cancelled),
                lookup_deadlines(),
            ),
            authentication,
        ));
        assert_eq!(
            held.started.recv_timeout(Duration::from_secs(2)).unwrap(),
            name
        );
        if !authentication {
            cancelled.store(true, Ordering::Release);
        }
    }
    let excess = held.command(
        "excess.invalid",
        true,
        Arc::new(AtomicBool::new(false)),
        lookup_deadlines(),
    );
    assert!(
        held.started
            .recv_timeout(Duration::from_millis(250))
            .is_err(),
        "the 33rd system lookup started while all 32 admitted lookups remained held"
    );
    assert_lookup_command_error(excess, true, false);
    for (result, authentication) in results {
        assert_lookup_command_error(result, authentication, !authentication);
    }
    assert!(matches!(
        held.completed.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));

    let cancelled = Arc::new(AtomicBool::new(false));
    let waiting = held.command(
        "cancelled-admission.invalid",
        false,
        Arc::clone(&cancelled),
        HttpDeadlines {
            header: Duration::from_secs(5),
            ..lookup_deadlines()
        },
    );
    cancelled.store(true, Ordering::Release);
    assert_lookup_command_error(waiting, false, true);
    assert!(matches!(
        held.started.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));

    held.release("mixed-0.invalid");
    let replacement = held.command(
        "replacement.invalid",
        true,
        Arc::new(AtomicBool::new(false)),
        lookup_deadlines(),
    );
    assert_eq!(
        held.started.recv_timeout(Duration::from_secs(2)).unwrap(),
        "replacement.invalid"
    );
    assert_lookup_command_error(replacement, true, false);
    assert!(matches!(
        held.completed.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
}
