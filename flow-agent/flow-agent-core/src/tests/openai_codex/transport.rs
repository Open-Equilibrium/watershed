use super::super::deadlines::{
    assert_pending, block_on_paused_network, expect_ready, send_scripted_http_bytes,
    settle_pending, spawn_scripted_http_server,
};
use crate::runtime::deadlines::{
    HttpDeadlines, RESPONSES_HTTP_DEADLINES, block_on_network, build_http_client,
};
use crate::runtime::endpoint_credential::EndpointCredential;
use crate::runtime::error::ProviderFailureKind;
use crate::runtime::openai_codex::{
    ResponsesAuth, request_responses_at_with_deadlines_and_cancellation,
    request_responses_with_client_async,
};
use crate::runtime::productive::{OpenAiCodexProvider, ProductiveProvider, ProviderCredential};
use crate::runtime::responses::{MAX_RESPONSES_DECODED_STREAM_BYTES, MAX_RESPONSES_LINE_BYTES};
use crate::runtime::types::RuntimeError;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

fn write_sse_response(stream: &mut impl Write, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .expect("response writes");
}

#[test]
fn productive_http_dispatch_uses_the_pinned_headers_and_no_retry() {
    let credential = fixture_credential_with_routing(None);
    let access = credential.access.clone();
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!(
        "http://localhost:{}/responses",
        listener.local_addr().unwrap().port()
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = vec![0_u8; 16 * 1024];
        let read = stream.read(&mut request).expect("request reads");
        let request = String::from_utf8(request[..read].to_vec()).expect("request UTF-8");
        assert!(request.starts_with("POST /responses HTTP/1.1\r\n"));
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains(&format!(
            "authorization: bearer {}\r\n",
            access.to_ascii_lowercase()
        )));
        assert!(lower.contains("chatgpt-account-id: account-fixture\r\n"));
        assert!(lower.contains("originator: flow-agent\r\n"));
        assert!(lower.contains(&format!(
            "user-agent: flow-agent/{}\r\n",
            env!("CARGO_PKG_VERSION")
        )));
        assert!(lower.contains("accept: text/event-stream\r\n"));
        assert!(!lower.contains("x-openai-fedramp:"));
        let body = concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:[DONE]\n\n"
        );
        write_sse_response(&mut stream, body);
    });
    let turn = OpenAiCodexProvider::default()
        .turn_at(
            &endpoint,
            &credential,
            &serde_json::json!({"model":"fixture"}),
        )
        .expect("one request completes");
    assert_eq!(turn.output_text, "ok");
    server.join().expect("fake provider completes");
}

#[test]
fn productive_http_dispatch_routes_fedramp_accounts() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = vec![0_u8; 16 * 1024];
        let read = stream.read(&mut request).expect("request reads");
        let request = String::from_utf8(request[..read].to_vec()).expect("request UTF-8");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-openai-fedramp: true\r\n")
        );
        let body = concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:[DONE]\n\n"
        );
        write_sse_response(&mut stream, body);
    });

    OpenAiCodexProvider::default()
        .turn_at(
            &endpoint,
            &fixture_credential_with_routing(Some(true)),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect("FedRAMP request completes");
    server.join().expect("fake provider completes");
}

#[test]
fn productive_provider_reuses_completed_connections_with_request_local_headers() {
    fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte)?;
            request.push(byte[0]);
            assert!(request.len() <= 16 * 1024, "fixture headers are bounded");
        }
        let headers = String::from_utf8(request).expect("fixture request headers are UTF-8");
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(|length| length.parse::<usize>().expect("fixture content length"))
            })
            .expect("fixture request has a content length");
        assert!(content_length <= 4096, "fixture body is bounded");
        let mut body = vec![0; content_length];
        stream.read_exact(&mut body)?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).expect("fixture JSON body"),
            serde_json::json!({"model":"fixture"})
        );
        Ok(headers)
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let (release_sender, release_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("first provider request connects");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("fixture deadlock guard is set");
        let mut connections = 1;
        let mut requests = Vec::new();
        for turn in 1..=2 {
            let request = match read_request(&mut stream) {
                Ok(request) => request,
                Err(error) if turn == 2 && error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    (stream, _) = listener.accept().expect("second provider request connects");
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .expect("fixture deadlock guard is set");
                    connections += 1;
                    read_request(&mut stream).expect("second request reads")
                }
                Err(error) => panic!("fixture request failed: {error}"),
            };
            requests.push(request);
            let body = format!(
                "data:{{\"type\":\"response.created\",\"response\":{{\"id\":\"resp_{turn}\"}}}}\n\n\
                 data:{{\"type\":\"response.output_text.delta\",\"delta\":\"turn {turn}\"}}\n\n\
                 data:{{\"type\":\"response.completed\",\"response\":{{\"id\":\"resp_{turn}\"}}}}\n\n\
                 data:[DONE]\n\n"
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("complete keep-alive response writes");
            stream.flush().expect("complete response flushes");
        }
        release_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("both provider turns complete while the connection stays open");
        (connections, requests)
    });

    let mut provider = OpenAiCodexProvider::default();
    for turn in 1..=2 {
        let mut credential = fixture_credential_with_routing(Some(turn == 1));
        credential.access = format!("access-fixture-{turn}");
        credential.account_id = format!("account-fixture-{turn}");
        let response = provider
            .turn_at(
                &endpoint,
                &credential,
                &serde_json::json!({"model":"fixture"}),
            )
            .expect("fully completed provider turn succeeds");
        assert_eq!(response.output_text, format!("turn {turn}"));
    }
    release_sender
        .send(())
        .expect("fixture connection can close");
    let (connections, requests) = server.join().expect("fake provider completes");
    assert_eq!(requests.len(), 2, "each turn sends exactly one request");
    for (index, request) in requests.iter().enumerate() {
        let turn = index + 1;
        assert!(request.starts_with("POST /responses HTTP/1.1\r\n"));
        let headers = request.to_ascii_lowercase();
        assert!(headers.contains(&format!("authorization: bearer access-fixture-{turn}\r\n")));
        assert!(headers.contains(&format!("chatgpt-account-id: account-fixture-{turn}\r\n")));
        assert!(headers.contains("originator: flow-agent\r\n"));
        assert!(headers.contains("accept: text/event-stream\r\n"));
        assert!(headers.contains(&format!(
            "user-agent: flow-agent/{}\r\n",
            env!("CARGO_PKG_VERSION")
        )));
        assert_eq!(headers.contains("x-openai-fedramp: true\r\n"), turn == 1);
    }
    assert_eq!(
        connections, 1,
        "completed turns reuse the provider's HTTP connection"
    );
}

#[test]
fn productive_http_dispatch_enforces_the_decoded_stream_budget() {
    const EVENT_BYTES: usize = 260_000;
    const JSON_OVERHEAD: usize = r#"{"padding":""}"#.len();
    let event = format!(
        "data:{{\"padding\":\"{}\"}}\n\n",
        "x".repeat(EVENT_BYTES - JSON_OVERHEAD)
    );
    assert!(event.len() - 2 <= MAX_RESPONSES_LINE_BYTES);
    let event_count = MAX_RESPONSES_DECODED_STREAM_BYTES / EVENT_BYTES + 1;
    let content_length = event.len() * event_count;

    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = [0_u8; 4096];
        assert!(stream.read(&mut request).expect("request reads") > 0);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
        )
        .expect("response headers write");
        for _ in 0..event_count {
            stream.write_all(event.as_bytes()).expect("event writes");
        }
    });

    let error = OpenAiCodexProvider::default()
        .turn_at(&endpoint, &fixture_credential(), &serde_json::json!({}))
        .expect_err("aggregate decoded stream is bounded");
    assert!(
        error.to_string().contains("decoded response stream"),
        "{error}"
    );
    server.join().expect("fake provider completes");
}

#[test]
fn definitive_http_failure_reports_status_and_bounded_provider_message() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = [0_u8; 4096];
        assert!(stream.read(&mut request).expect("request reads") > 0);
        let body = serde_json::json!({"error":{"message":"x".repeat(4_100)}}).to_string();
        write!(
            stream,
            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("provider error writes");
    });

    let error = OpenAiCodexProvider::default()
        .turn_at(&endpoint, &fixture_credential(), &serde_json::json!({}))
        .expect_err("provider rejection is definitive");
    let message = error.to_string();
    let provider_message = message
        .strip_prefix("provider_error (HTTP 429): ")
        .expect("stable provider error and HTTP status");
    assert_eq!(provider_message.chars().count(), 4_000);
    assert!(provider_message.chars().all(|character| character == 'x'));
    server.join().expect("fake provider completes");
}

#[test]
fn reply_that_is_not_an_event_stream_is_reply_invalid() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = [0_u8; 4096];
        assert!(stream.read(&mut request).expect("request reads") > 0);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .expect("provider reply writes");
    });

    let error = OpenAiCodexProvider::default()
        .turn_at(&endpoint, &fixture_credential(), &serde_json::json!({}))
        .expect_err("a JSON reply is not an event stream");
    let failure = error
        .provider_failure()
        .expect("the reply is a provider reply failure");
    assert!(failure.is_reply_invalid());
    assert_eq!(failure.http_status(), None);
    assert_eq!(
        failure.message(),
        "Flow Agent rejected the provider reply: Responses reply is not text/event-stream"
    );
    server.join().expect("fake provider completes");
}

#[test]
fn productive_http_dispatch_rejects_a_missing_terminal_sentinel() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = [0_u8; 16 * 1024];
        assert!(stream.read(&mut request).expect("request reads") > 0);
        let body = concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n"
        );
        write_sse_response(&mut stream, body);
    });

    let error = OpenAiCodexProvider::default()
        .turn_at(
            &endpoint,
            &fixture_credential(),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("missing terminal sentinel fails");
    assert!(error.to_string().contains("terminal sentinel"));
    server.join().expect("fake provider completes");
}

#[test]
fn productive_http_dispatch_leaves_a_stream_without_a_terminal_event_uncertain() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = [0_u8; 16 * 1024];
        assert!(stream.read(&mut request).expect("request reads") > 0);
        let body = concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:[DONE]\n\n"
        );
        write_sse_response(&mut stream, body);
    });

    let error = OpenAiCodexProvider::default()
        .turn_at(
            &endpoint,
            &fixture_credential(),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("a stream without a terminal event fails");
    assert!(
        !error
            .provider_failure()
            .expect("provider failure")
            .is_definitive()
    );
    server.join().expect("fake provider completes");
}

#[test]
fn productive_http_dispatch_returns_after_the_terminal_sentinel() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let (release_sender, release_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = [0_u8; 4096];
        assert!(stream.read(&mut request).expect("request reads") > 0);
        let body = concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:[DONE]\n\n"
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:X}\r\n{}\r\n",
            body.len(),
            body
        )
        .expect("streaming response writes");
        stream.flush().expect("terminal sentinel flushes");
        release_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("client returns before the response body closes");
    });
    let cancelled = AtomicBool::new(false);
    let result = request_responses_at_with_deadlines_and_cancellation(
        &endpoint,
        &fixture_credential(),
        &serde_json::json!({}),
        HttpDeadlines {
            connect: Duration::from_secs(1),
            header: Duration::from_secs(1),
            read: Duration::from_secs(3),
            overall: Duration::from_secs(4),
        },
        &cancelled,
    );
    release_sender
        .send(())
        .expect("provider body can close after the client returns");
    result.expect("terminal sentinel completes the response");
    server.join().expect("fake provider completes");
}

fn fixture_credential() -> crate::runtime::oauth_credential::CredentialRecord {
    fixture_credential_with_routing(None)
}

fn fixture_credential_with_routing(
    is_fedramp: Option<bool>,
) -> crate::runtime::oauth_credential::CredentialRecord {
    crate::runtime::oauth_credential::CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: "access-fixture".to_owned(),
        refresh: "refresh-fixture".to_owned(),
        expires: 1,
        account_id: "account-fixture".to_owned(),
        is_fedramp: is_fedramp.unwrap_or(false),
    }
}

#[test]
fn responses_header_deadline() {
    assert_eq!(RESPONSES_HTTP_DEADLINES.header, Duration::from_secs(30));
    let (endpoint, connected, writes, _written, server) = spawn_scripted_http_server("/responses");
    let credential = fixture_credential();
    let body = serde_json::json!({});
    let cancelled = AtomicBool::new(false);
    block_on_paused_network(async {
        let client = build_http_client(HttpDeadlines {
            read: RESPONSES_HTTP_DEADLINES.read + Duration::from_secs(1),
            overall: RESPONSES_HTTP_DEADLINES.overall + Duration::from_secs(1),
            ..RESPONSES_HTTP_DEADLINES
        })
        .expect("Responses client builds");
        let request = tokio::spawn(async move {
            request_responses_with_client_async(
                client,
                &endpoint,
                ResponsesAuth::Subscription(&credential),
                &body,
                RESPONSES_HTTP_DEADLINES,
                &cancelled,
            )
            .await
        });
        settle_pending(&request).await;
        connected
            .recv_timeout(Duration::from_secs(2))
            .expect("provider request connects");
        settle_pending(&request).await;
        tokio::time::advance(RESPONSES_HTTP_DEADLINES.header - Duration::from_nanos(1)).await;
        assert_pending(&request).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        let error = expect_ready(request)
            .await
            .expect_err("missing headers time out");
        assert!(
            error
                .to_string()
                .contains("Responses header deadline elapsed")
        );
    });
    drop(writes);
    server.join().expect("fake provider completes");
}

#[test]
fn responses_request_observes_productive_cancellation_while_waiting_for_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake provider binds");
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
    let (request_sender, request_receiver) = mpsc::sync_channel(0);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one provider request");
        let mut request = [0_u8; 4096];
        assert!(stream.read(&mut request).expect("request reads") > 0);
        request_sender.send(()).expect("request is announced");
        thread::sleep(Duration::from_secs(2));
    });
    let cancelled = Arc::new(AtomicBool::new(false));
    let client_cancelled = Arc::clone(&cancelled);
    let client = thread::spawn(move || {
        request_responses_at_with_deadlines_and_cancellation(
            &endpoint,
            &fixture_credential(),
            &serde_json::json!({}),
            HttpDeadlines {
                connect: Duration::from_secs(3),
                header: Duration::from_secs(3),
                read: Duration::from_secs(3),
                overall: Duration::from_secs(5),
            },
            &client_cancelled,
        )
    });
    request_receiver
        .recv_timeout(Duration::from_secs(4))
        .expect("client sends the request");
    let started = Instant::now();
    cancelled.store(true, Ordering::Release);
    let error = client
        .join()
        .expect("client completes")
        .expect_err("cancelled provider request fails");
    assert!(matches!(error, RuntimeError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.join().expect("fake provider completes");
}

#[test]
fn responses_idle_deadline() {
    assert_eq!(RESPONSES_HTTP_DEADLINES.read, Duration::from_secs(120));
    let (endpoint, connected, writes, written, server) = spawn_scripted_http_server("/responses");
    let credential = fixture_credential();
    let body = serde_json::json!({});
    let cancelled = AtomicBool::new(false);
    block_on_paused_network(async {
        let client = build_http_client(HttpDeadlines {
            read: RESPONSES_HTTP_DEADLINES.read + Duration::from_secs(1),
            overall: RESPONSES_HTTP_DEADLINES.overall + Duration::from_secs(1),
            ..RESPONSES_HTTP_DEADLINES
        })
        .expect("Responses client builds");
        let request = tokio::spawn(async move {
            request_responses_with_client_async(
                client,
                &endpoint,
                ResponsesAuth::Subscription(&credential),
                &body,
                RESPONSES_HTTP_DEADLINES,
                &cancelled,
            )
            .await
        });
        assert_pending(&request).await;
        connected
            .recv_timeout(Duration::from_secs(2))
            .expect("provider request connects");
        settle_pending(&request).await;
        send_scripted_http_bytes(
            &writes,
            &written,
            concat!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n",
                "Connection: close\r\n\r\n",
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"one\"}}\n\n"
            ),
        );
        settle_pending(&request).await;
        tokio::time::advance(RESPONSES_HTTP_DEADLINES.read - Duration::from_nanos(1)).await;
        assert_pending(&request).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        let error = expect_ready(request)
            .await
            .expect_err("idle stream times out");
        assert!(error.to_string().contains("Responses stream failed"));
    });
    drop(writes);
    server.join().expect("fake provider completes");
}

#[test]
fn responses_overall_deadline() {
    assert_eq!(
        RESPONSES_HTTP_DEADLINES.overall,
        Duration::from_secs(30 * 60)
    );
    let (endpoint, connected, writes, written, server) = spawn_scripted_http_server("/responses");
    let credential = fixture_credential();
    let body = serde_json::json!({});
    let cancelled = AtomicBool::new(false);
    block_on_paused_network(async {
        let request = tokio::spawn(async move {
            request_responses_with_client_async(
                build_http_client(RESPONSES_HTTP_DEADLINES).expect("Responses client builds"),
                &endpoint,
                ResponsesAuth::Subscription(&credential),
                &body,
                RESPONSES_HTTP_DEADLINES,
                &cancelled,
            )
            .await
        });
        assert_pending(&request).await;
        connected
            .recv_timeout(Duration::from_secs(2))
            .expect("provider request connects");
        settle_pending(&request).await;
        send_scripted_http_bytes(
            &writes,
            &written,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        );
        settle_pending(&request).await;
        for _ in 0..17 {
            tokio::time::advance(Duration::from_secs(100)).await;
            send_scripted_http_bytes(&writes, &written, ": progress\n\n");
            settle_pending(&request).await;
        }
        tokio::time::advance(Duration::from_secs(100) - Duration::from_nanos(1)).await;
        assert_pending(&request).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        let error = expect_ready(request)
            .await
            .expect_err("progressing stream reaches its overall deadline");
        assert!(error.to_string().contains("Responses stream failed"));
    });
    drop(writes);
    server.join().expect("fake provider completes");
}

fn endpoint_credential(base_url: &str, key: &str) -> ProviderCredential {
    ProviderCredential::OpenAiCompatible(
        EndpointCredential::new(base_url, key).expect("fixture endpoint is valid"),
    )
}

/// Serves one reply and returns the request head it read; the base URL is `http://127.0.0.1:port/v1`.
fn spawn_endpoint(
    status_line: &'static str,
    extra_headers: &'static str,
    body: &'static str,
) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let base_url = format!(
        "http://{}/v1",
        listener.local_addr().expect("fake endpoint address")
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one endpoint request");
        let mut request = vec![0_u8; 16 * 1024];
        let read = stream.read(&mut request).expect("request reads");
        let head = String::from_utf8_lossy(&request[..read]).into_owned();
        write!(
            stream,
            "{status_line}\r\nContent-Type: text/event-stream\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("endpoint reply writes");
        head
    });
    (base_url, server)
}

#[test]
fn endpoint_dispatch_sends_only_the_key_to_the_responses_path() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 200 OK",
        "",
        concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        ),
    );
    let turn = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect("endpoint request completes");
    assert_eq!(turn.output_text, "ok");
    let head = server
        .join()
        .expect("fake endpoint completes")
        .to_ascii_lowercase();
    assert!(
        head.starts_with("post /v1/responses http/1.1\r\n"),
        "{head}"
    );
    assert!(
        head.contains("authorization: bearer sk-endpoint-fixture\r\n"),
        "{head}"
    );
    for subscription_header in ["chatgpt-account-id:", "originator:", "x-openai-fedramp:"] {
        assert!(!head.contains(subscription_header), "{head}");
    }
}

#[test]
fn endpoint_stream_ends_at_its_completed_event_without_the_done_sentinel() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 200 OK",
        "",
        concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        ),
    );
    let turn = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect("completed endpoint stream is a finished reply");
    assert_eq!(turn.response_id, "resp_1");
    server.join().expect("fake endpoint completes");
}

#[test]
fn subscription_stream_still_requires_the_done_sentinel() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 200 OK",
        "",
        concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        ),
    );
    let error = OpenAiCodexProvider::default()
        .turn_at(
            &format!("{base_url}/responses"),
            &fixture_credential_with_routing(None),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("subscription stream without the sentinel is uncertain");
    let failure = error
        .provider_failure()
        .expect("missing sentinel is a provider outcome");
    assert!(matches!(failure.kind(), ProviderFailureKind::Uncertain));
    server.join().expect("fake endpoint completes");
}

#[test]
fn endpoint_terminal_failure_events_end_the_attempt_as_provider_errors() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 200 OK",
        "",
        "data:{\"type\":\"response.failed\",\"response\":{\"id\":\"resp_1\",\"error\":{\"message\":\"quota exhausted\"}}}\n\n",
    );
    let error = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("failed terminal event fails the attempt");
    let failure = error
        .provider_failure()
        .expect("failure is a provider outcome");
    assert!(failure.is_definitive());
    assert_eq!(failure.message(), "quota exhausted");
    server.join().expect("fake endpoint completes");
}

#[test]
fn endpoint_stream_cut_before_a_terminal_event_is_uncertain() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 200 OK",
        "",
        "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
    );
    let error = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("stream without a terminal event is uncertain");
    let failure = error
        .provider_failure()
        .expect("failure is a provider outcome");
    assert!(matches!(failure.kind(), ProviderFailureKind::Uncertain));
    server.join().expect("fake endpoint completes");
}

#[test]
fn endpoint_redirects_are_not_followed_and_rejections_omit_the_key() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 307 Temporary Redirect",
        "Location: http://127.0.0.1:9/v1/responses\r\n",
        "",
    );
    let error = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("redirect is a definitive rejection");
    let failure = error
        .provider_failure()
        .expect("redirect is a provider outcome");
    assert!(failure.is_definitive());
    assert_eq!(failure.http_status(), Some(307));
    server.join().expect("fake endpoint completes");

    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 401 Unauthorized",
        "",
        "{\"error\":{\"message\":\"invalid api key\"}}",
    );
    let error = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("unauthorized endpoint is a definitive rejection");
    let failure = error
        .provider_failure()
        .expect("rejection is a provider outcome");
    assert_eq!(failure.http_status(), Some(401));
    assert_eq!(failure.message(), "invalid api key");
    assert!(!error.to_string().contains("sk-endpoint-fixture"));
    server.join().expect("fake endpoint completes");
}

#[test]
fn endpoint_turn_completes_at_its_terminal_event_while_the_connection_stays_open() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let endpoint = format!(
        "http://{}/v1/responses",
        listener.local_addr().expect("fake endpoint address")
    );
    let (release, released) = mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one endpoint request");
        let mut request = vec![0_u8; 16 * 1024];
        let read = stream.read(&mut request).expect("request reads");
        assert!(read > 0, "request arrives");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n{}",
            concat!(
                "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
                "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            )
        )
        .expect("endpoint reply writes");
        // The connection stays open after the terminal event; the client must not wait for it to close.
        let _ = released.recv_timeout(Duration::from_secs(20));
    });
    let deadlines = HttpDeadlines {
        connect: Duration::from_secs(5),
        header: Duration::from_secs(5),
        read: Duration::from_millis(500),
        overall: Duration::from_secs(10),
    };
    let cancelled = AtomicBool::new(false);
    let body = serde_json::json!({"model":"fixture"});
    let turn = block_on_network(async {
        let client = build_http_client(deadlines).expect("HTTP client builds");
        request_responses_with_client_async(
            client,
            &endpoint,
            ResponsesAuth::ApiKey("sk-endpoint-fixture"),
            &body,
            deadlines,
            &cancelled,
        )
        .await
    })
    .expect("network runtime runs")
    .expect("a reply that reached its terminal event is a finished turn");
    let _ = release.send(());
    assert_eq!(turn.response_id, "resp_1");
    server.join().expect("fake endpoint completes");
}

#[test]
fn endpoint_ignores_data_after_its_terminal_event() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 200 OK",
        "",
        concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data:{\"type\":\"response.output_text.delta\",\"delta\":\"late\"}\n\n",
            "data: not json at all\n\n",
        ),
    );
    let turn = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect("data after the terminal event is ignored");
    assert_eq!(turn.response_id, "resp_1");
    assert_eq!(
        turn.output_text, "",
        "data after the terminal event is not applied"
    );
    server.join().expect("fake endpoint completes");
}

#[test]
fn endpoint_terminal_failure_ignores_data_after_it() {
    let (base_url, server) = spawn_endpoint(
        "HTTP/1.1 200 OK",
        "",
        concat!(
            "data:{\"type\":\"response.failed\",\"response\":{\"id\":\"resp_1\",\"error\":{\"message\":\"quota exhausted\"}}}\n\n",
            "data: not json at all\n\n",
        ),
    );
    let error = OpenAiCodexProvider::default()
        .turn(
            &endpoint_credential(&base_url, "sk-endpoint-fixture"),
            &serde_json::json!({"model":"fixture"}),
        )
        .expect_err("failed terminal event fails the attempt");
    let failure = error
        .provider_failure()
        .expect("failure is a provider outcome");
    assert!(failure.is_definitive(), "{failure:?}");
    assert_eq!(failure.message(), "quota exhausted");
    server.join().expect("fake endpoint completes");
}
