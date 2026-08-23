use super::super::deadlines::{
    assert_pending, block_on_paused_network, expect_ready, send_scripted_http_bytes,
    settle_pending, spawn_scripted_http_server,
};
use crate::runtime::deadlines::{HttpDeadlines, RESPONSES_HTTP_DEADLINES, build_http_client};
use crate::runtime::openai_codex::{
    request_responses_async, request_responses_at,
    request_responses_at_with_deadlines_and_cancellation, request_responses_with_client_async,
};
use crate::runtime::responses::{MAX_RESPONSES_DECODED_STREAM_BYTES, MAX_RESPONSES_LINE_BYTES};
use crate::runtime::types::RuntimeError;
use std::io::{Read, Write};
use std::net::TcpListener;
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
    let endpoint = format!("http://{}/responses", listener.local_addr().unwrap());
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
    let turn = request_responses_at(
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

    request_responses_at(
        &endpoint,
        &fixture_credential_with_routing(Some(true)),
        &serde_json::json!({"model":"fixture"}),
    )
    .expect("FedRAMP request completes");
    server.join().expect("fake provider completes");
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

    let error = request_responses_at(&endpoint, &fixture_credential(), &serde_json::json!({}))
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

    let error = request_responses_at(&endpoint, &fixture_credential(), &serde_json::json!({}))
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

    let error = request_responses_at(
        &endpoint,
        &fixture_credential(),
        &serde_json::json!({"model":"fixture"}),
    )
    .expect_err("missing terminal sentinel fails");
    assert!(error.to_string().contains("terminal sentinel"));
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
                &credential,
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
                &credential,
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
            request_responses_async(
                &endpoint,
                &credential,
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
            assert_pending(&request).await;
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
