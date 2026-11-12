use super::super::test_support::workspace_copy;
use super::support::{DefaultRecovery, FakeToolExecutor, MemoryAttempts, MemorySink};
use crate::runtime::{
    endpoint_credential::EndpointCredential,
    productive::{
        OpenAiCodexProvider, ProviderCredential,
        execute_productive_flow_with_tool_executor_and_recovery,
    },
};
use crate::tests::helpers::load_productive_execution_fixture_with_provider_credential;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

const ENDPOINT_KEY: &str = "sk-endpoint-e2e-fixture";

/// Reads one HTTP request and returns its head and its body.
fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let head_end = loop {
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let read = stream.read(&mut chunk).expect("request head reads");
        assert!(read > 0, "request ends before its head");
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8(buffer[..head_end].to_vec()).expect("request head is UTF-8");
    let length = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(|value| value.trim().parse::<usize>().expect("content length"))
        })
        .expect("request has a content length");
    while buffer.len() < head_end + length {
        let read = stream.read(&mut chunk).expect("request body reads");
        assert!(read > 0, "request ends before its body");
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8(buffer[head_end..head_end + length].to_vec())
        .expect("request body is UTF-8");
    (head, body)
}

fn write_event_stream(stream: &mut TcpStream, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("endpoint reply writes");
}

#[test]
fn endpoint_provider_completes_a_flow_through_a_tool_call_over_http() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let base_url = format!(
        "http://{}/v1",
        listener.local_addr().expect("fake endpoint address")
    );
    let server = thread::spawn(move || {
        let tool_turn = concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp-1\"}}\n\n",
            "data:{\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"call-endpoint-1\",\"name\":\"echo\",\"arguments\":\"{}\"}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp-1\"}}\n\n",
        );
        let final_turn = concat!(
            "data:{\"type\":\"response.created\",\"response\":{\"id\":\"resp-2\"}}\n\n",
            "data:{\"type\":\"response.output_text.delta\",\"delta\":\"{\\\"type\\\":\\\"string\\\",\\\"value\\\":\\\"productive\\\"}\"}\n\n",
            "data:{\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"id\":\"message-2\",\"role\":\"assistant\",\"content\":[]}}\n\n",
            "data:{\"type\":\"response.completed\",\"response\":{\"id\":\"resp-2\"}}\n\n",
        );
        let mut requests = Vec::new();
        for reply in [tool_turn, final_turn] {
            let (mut stream, _) = listener.accept().expect("one endpoint request");
            requests.push(read_request(&mut stream));
            write_event_stream(&mut stream, reply);
        }
        requests
    });

    let workspace = workspace_copy("smoke-flow");
    let fixture = load_productive_execution_fixture_with_provider_credential(
        &workspace,
        "smoke-flow",
        ProviderCredential::OpenAiCompatible(
            EndpointCredential::new(&base_url, ENDPOINT_KEY).expect("endpoint credential is valid"),
        ),
    );
    let flow = fixture.smoke_flow().clone();
    let mut provider = OpenAiCodexProvider::default();
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    let mut recovery = DefaultRecovery;
    let execution = execute_productive_flow_with_tool_executor_and_recovery(
        fixture.execution(&flow, "endpoint-provider-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
        &mut recovery,
    )
    .expect("endpoint Flow runs");
    let requests = server.join().expect("fake endpoint completes");

    assert!(!execution.failed, "{:?}", execution.terminal_error);
    assert_eq!(
        tools.invocations.len(),
        1,
        "the endpoint's Tool call runs once"
    );
    assert_eq!(requests.len(), 2, "the Tool result returns to the endpoint");
    for (head, _) in &requests {
        assert!(
            head.starts_with("POST /v1/responses HTTP/1.1\r\n"),
            "{head}"
        );
        assert!(
            head.to_ascii_lowercase()
                .contains(&format!("authorization: bearer {ENDPOINT_KEY}\r\n")),
            "{head}"
        );
    }
    assert!(requests[1].1.contains("call-endpoint-1"));
    assert!(
        requests
            .iter()
            .all(|(_, body)| !body.contains(ENDPOINT_KEY)),
        "the key travels only in the Authorization header"
    );
    assert!(
        !format!("{:?}", sink.0).contains(ENDPOINT_KEY),
        "run events never record the key"
    );
}
