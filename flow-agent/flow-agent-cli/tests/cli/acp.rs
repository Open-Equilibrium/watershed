use super::{
    flow_command,
    process::cli_child_watchdog,
    test_support::{
        empty_workspace_under, run_current_test_isolated_session_home, session_home_path,
        workspace_copy,
    },
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, ChildStdin, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

struct Acp {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    next_id: u64,
}

impl Acp {
    fn start(workspace: &Path, config_root: &Path) -> Self {
        let mut child = flow_command()
            .current_dir(workspace)
            .env("HOME", config_root)
            .env("XDG_CONFIG_HOME", config_root)
            .arg("acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("flow acp starts");
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.expect("stdout is UTF-8 lines");
                let message = serde_json::from_str(&line).expect("stdout carries only JSON-RPC");
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
            next_id: 0,
        }
    }

    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        self.write(
            json!({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}),
        );
        self.next_id
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn write(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").expect("a message reaches flow acp");
    }

    fn next(&self, matches: impl Fn(&Value) -> bool, seen: &mut Vec<Value>) -> Value {
        loop {
            let message = self
                .lines
                .recv_timeout(cli_child_watchdog())
                .unwrap_or_else(|_| panic!("flow acp answered in time; saw {seen:#?}"));
            seen.push(message.clone());
            if matches(&message) {
                return message;
            }
        }
    }

    fn reply(&self, id: u64, seen: &mut Vec<Value>) -> Value {
        self.next(|message| message["id"] == id, seen)
    }

    fn open(&mut self, cwd: &Path) -> (String, Value) {
        let id = self.send(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": {}}),
        );
        let initialized = self.reply(id, &mut Vec::new());
        assert_eq!(
            initialized["result"]["agentCapabilities"]["loadSession"],
            true
        );
        let id = self.send("session/new", json!({"cwd": cwd, "mcpServers": []}));
        let session = self.reply(id, &mut Vec::new());
        let session_id = session["result"]["sessionId"]
            .as_str()
            .expect("the session has an id")
            .to_owned();
        (session_id, session["result"]["configOptions"].clone())
    }
}

impl Drop for Acp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_chat_offers_the_installed_flows_and_keeps_its_choice_until_it_starts() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let workspace = workspace_copy("hello-flow");
    fs::write(
        session_home_path().join("registry/flows/hello-summary.yaml"),
        "flow:\n  id: hello-summary\n  name: HelloSummary\n  phase_refs: [summarize]\n",
    )
    .expect("a second Flow is installed");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    let mut acp = Acp::start(&workspace, &config_root);

    let (session_id, options) = acp.open(&workspace);

    assert!(session_id.starts_with("chat-"), "{session_id}");
    assert_eq!(options[0]["id"], "flow");
    assert_eq!(options[0]["type"], "select");
    let choices = options[0]["options"]
        .as_array()
        .expect("the Flow option lists choices")
        .iter()
        .map(|choice| choice["value"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(choices, ["hello-flow", "hello-summary"]);
    assert_eq!(options[0]["currentValue"], "hello-flow");

    let unknown = acp.send(
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": "flow", "value": "missing"}),
    );
    assert_eq!(acp.reply(unknown, &mut Vec::new())["error"]["code"], -32602);
    let chosen = acp.send(
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": "flow", "value": "hello-summary"}),
    );
    assert_eq!(
        acp.reply(chosen, &mut Vec::new())["result"]["configOptions"][0]["currentValue"],
        "hello-summary"
    );
    let unsupported = acp.send("session/set_mode", json!({"sessionId": session_id}));
    assert_eq!(
        acp.reply(unsupported, &mut Vec::new())["error"]["code"],
        -32601
    );
}

#[test]
fn an_oversized_line_is_refused_and_the_chat_goes_on() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    let mut acp = Acp::start(&workspace, &config_root);

    let oversized = "x".repeat(8 * 1024 * 1024 + 1);
    writeln!(acp.stdin, "{oversized}").expect("the oversized line is written");

    let refused = acp.next(|_| true, &mut Vec::new());
    assert_eq!(refused["error"]["code"], -32700);
    let (session_id, _) = acp.open(&workspace);
    assert!(session_id.starts_with("chat-"));
}

#[test]
fn a_first_message_beyond_the_run_input_limit_is_refused() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    let mut acp = Acp::start(&workspace, &config_root);
    let (session_id, _) = acp.open(&workspace);

    let text = "x".repeat(flow_agent_core::MAX_FLOW_RUN_INPUT_BYTES + 1);
    let prompt = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
    );

    let answer = acp.reply(prompt, &mut Vec::new());
    assert_eq!(answer["error"]["code"], -32602, "{answer}");
}

#[test]
fn a_chat_needs_a_productive_provider() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    let mut acp = Acp::start(&workspace, &config_root);
    let (session_id, _) = acp.open(&workspace);

    let prompt = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "hello"}]}),
    );

    let answer = acp.reply(prompt, &mut Vec::new());
    assert!(
        answer["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("productive"),
        "{answer}"
    );
}

const NOTE_FLOW: [(&str, &str); 4] = [
    (
        "flows/alpha.yaml",
        "flow:\n  id: alpha\n  name: Alpha\n  phase_refs: [write]\n",
    ),
    (
        "flows/note.yaml",
        "flow:\n  id: note\n  name: Note\n  phase_refs: [write]\n",
    ),
    (
        "phases/write.yaml",
        "phase:\n  id: write\n  name: Write\n  instruction_refs: [write-note]\n  tool_refs: []\n  output:\n    type: string\n",
    ),
    (
        "instructions/write-note.yaml",
        "instruction:\n  id: write-note\n  name: WriteNote\n  prompt: \"Write a short note about the input.\"\n",
    ),
];

fn install_note_flow(base_url: &str, config_root: &Path) {
    install_flows(base_url, config_root, &NOTE_FLOW);
}

fn install_flows(base_url: &str, config_root: &Path, definitions: &[(&str, &str)]) {
    let home = session_home_path();
    let registry = home.join("registry");
    fs::remove_dir_all(&registry).expect("the fixture registry is replaced");
    for (path, source) in definitions {
        let path = registry.join(path);
        fs::create_dir_all(path.parent().expect("definition has a folder"))
            .expect("registry folder exists");
        fs::write(path, source).expect("definition is installed");
    }
    fs::write(
        home.join("config.yaml"),
        "model: gpt-fixture\nmodel_context_limit: 128000\noutput_reserve: 16384\nprovider: openai-compatible\nregistry_root: registry\n",
    )
    .expect("productive config is installed");
    let mut login = flow_command()
        .current_dir(config_root)
        .env("HOME", config_root)
        .env("XDG_CONFIG_HOME", config_root)
        .args([
            "auth",
            "login",
            "openai-compatible",
            "--base-url",
            base_url,
            "--api-key",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("login starts");
    login
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"sk-acp-fixture\n")
        .expect("key is written");
    assert!(login.wait().expect("login ends").success());
}

fn read_request_body(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("request has a content length");
            while buffer.len() < end + 4 + length {
                let read = stream.read(&mut chunk).expect("request body reads");
                buffer.extend_from_slice(&chunk[..read]);
            }
            return String::from_utf8_lossy(&buffer[end + 4..end + 4 + length]).into_owned();
        }
        let read = stream.read(&mut chunk).expect("request head reads");
        assert!(read > 0, "request ends before its head");
        buffer.extend_from_slice(&chunk[..read]);
    }
}

fn reply_with_text(stream: &mut TcpStream, response_id: &str, value: &str) {
    let output = json!({"type": "string", "value": value}).to_string();
    let body = [
        json!({"type": "response.created", "response": {"id": response_id}}),
        json!({"type": "response.output_text.delta", "delta": output}),
        json!({"type": "response.output_item.done", "item": {"type": "message", "id": format!("{response_id}-message"), "role": "assistant", "content": []}}),
        json!({"type": "response.completed", "response": {"id": response_id}}),
    ]
    .iter()
    .map(|event| format!("data:{event}\n\n"))
    .collect::<String>();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("endpoint reply writes");
}

#[test]
fn a_message_sent_while_a_flow_works_steers_its_active_phase() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let base_url = format!(
        "http://{}/v1",
        listener.local_addr().expect("endpoint address")
    );
    let (release_first, first_released) = mpsc::channel::<()>();
    let (first_arrived, first_in_flight) = mpsc::channel::<()>();
    let endpoint = thread::spawn(move || {
        let (mut first, _) = listener.accept().expect("the first turn arrives");
        let first_body = read_request_body(&mut first);
        first_arrived
            .send(())
            .expect("the test waits for the first turn");
        first_released
            .recv_timeout(Duration::from_secs(30))
            .expect("the test releases the first turn");
        reply_with_text(&mut first, "resp-draft", "Colorful draft");
        let (mut second, _) = listener.accept().expect("the steered turn arrives");
        let second_body = read_request_body(&mut second);
        reply_with_text(&mut second, "resp-final", "Colourful note");
        (first_body, second_body)
    });
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    install_note_flow(&base_url, &config_root);
    let mut acp = Acp::start(&workspace, &config_root);
    let (session_id, _) = acp.open(&workspace);
    let mut seen = Vec::new();

    let first = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "<t3_code_instructions>\nT3 rules\n</t3_code_instructions>\n\n<user_request>\nA note about colour\n</user_request>"}, {"type": "text", "text": "runtime instructions"}]}),
    );
    first_in_flight
        .recv_timeout(cli_child_watchdog())
        .expect("the first turn reaches the endpoint");
    let switched = acp.send(
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": "flow", "value": "note"}),
    );
    assert_eq!(acp.reply(switched, &mut seen)["error"]["code"], -32602);
    acp.notify("session/cancel", json!({"sessionId": session_id}));
    assert_eq!(
        acp.reply(first, &mut seen)["result"]["stopReason"],
        "cancelled"
    );
    release_first.send(()).expect("the endpoint waits");
    wait_for_committed_draft(&workspace, &session_id);
    let steer = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "Use British spelling"}]}),
    );
    let finished = acp.reply(steer, &mut seen);

    assert_eq!(finished["result"]["stopReason"], "end_turn", "{seen:#?}");
    let (first_body, second_body) = endpoint.join().expect("the endpoint completes");
    assert!(first_body.contains("A note about colour"), "{first_body}");
    assert!(!first_body.contains("T3 rules"), "{first_body}");
    assert!(
        second_body.contains("Use British spelling"),
        "{second_body}"
    );
    let replies = seen
        .iter()
        .filter(|message| message["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .map(|message| {
            message["params"]["update"]["content"]["text"]
                .as_str()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    assert_eq!(replies, ["Colorful draft", "Colourful note"]);
    let plan = seen
        .iter()
        .rfind(|message| message["params"]["update"]["sessionUpdate"] == "plan")
        .expect("the Phases show as a plan");
    assert_eq!(
        plan["params"]["update"]["entries"],
        json!([{"content": "Write", "priority": "medium", "status": "completed"}])
    );
    assert_eq!(
        flow_agent_core::conversation_state(&workspace, &session_id).expect("the chat is stored"),
        flow_agent_core::ConversationState::Settled
    );
}

fn wait_for_committed_draft(workspace: &Path, session_id: &str) {
    for _ in 0..400 {
        let replay = flow_agent_core::replay_conversation_run(
            workspace,
            session_id,
            session_id,
            flow_agent_core::EmitMode::Jsonl,
        );
        if replay.is_ok_and(|output| output.stdout.contains("\"message.completed\"")) {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "the first turn's reply is committed: {:?}",
        flow_agent_core::replay_conversation_run(
            workspace,
            session_id,
            session_id,
            flow_agent_core::EmitMode::Jsonl
        )
    );
}

#[test]
fn an_interrupted_run_resumes_with_the_next_message_unless_its_flow_changed() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let base_url = format!(
        "http://{}/v1",
        listener.local_addr().expect("endpoint address")
    );
    let (first_arrived, first_in_flight) = mpsc::channel::<()>();
    let (release_first, first_released) = mpsc::channel::<()>();
    let endpoint = thread::spawn(move || {
        let (mut first, _) = listener.accept().expect("the first turn arrives");
        read_request_body(&mut first);
        first_arrived
            .send(())
            .expect("the test waits for the first turn");
        first_released
            .recv_timeout(Duration::from_secs(30))
            .expect("the test releases the first turn");
        reply_with_text(&mut first, "resp-draft", "Colorful draft");
        let (mut resumed, _) = listener.accept().expect("the resumed turn arrives");
        let body = read_request_body(&mut resumed);
        reply_with_text(&mut resumed, "resp-final", "Colourful note");
        body
    });
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    install_note_flow(&base_url, &config_root);
    let mut acp = Acp::start(&workspace, &config_root);
    let (session_id, _) = acp.open(&workspace);
    let chosen = acp.send(
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": "flow", "value": "note"}),
    );
    assert!(acp.reply(chosen, &mut Vec::new())["result"].is_object());
    let first = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "A note about colour"}]}),
    );
    first_in_flight
        .recv_timeout(cli_child_watchdog())
        .expect("the first turn reaches the endpoint");
    acp.notify("session/cancel", json!({"sessionId": session_id}));
    assert_eq!(
        acp.reply(first, &mut Vec::new())["result"]["stopReason"],
        "cancelled"
    );
    release_first.send(()).expect("the endpoint waits");
    wait_for_committed_draft(&workspace, &session_id);
    drop(acp);
    assert_eq!(
        flow_agent_core::conversation_state(&workspace, &session_id).expect("the chat is stored"),
        flow_agent_core::ConversationState::Interrupted {
            run_session_id: session_id.clone()
        }
    );

    let instruction = session_home_path().join("registry/instructions/write-note.yaml");
    let original = fs::read_to_string(&instruction).expect("instruction reads");
    fs::write(&instruction, original.replace("short", "long")).expect("the Flow changes");
    let mut acp = Acp::start(&workspace, &config_root);
    acp.send("initialize", json!({"protocolVersion": 1}));
    let load = acp.send(
        "session/load",
        json!({"sessionId": session_id, "cwd": workspace.to_path_buf(), "mcpServers": []}),
    );
    assert_eq!(
        acp.reply(load, &mut Vec::new())["result"]["configOptions"][0]["currentValue"],
        "note"
    );
    let refused = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "Use British spelling"}]}),
    );
    let answer = acp.reply(refused, &mut Vec::new());
    assert!(answer["error"].is_object(), "{answer}");

    fs::write(&instruction, original).expect("the Flow is restored");
    let steer = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "Use British spelling"}]}),
    );
    let mut seen = Vec::new();
    let finished = acp.reply(steer, &mut seen);

    assert_eq!(finished["result"]["stopReason"], "end_turn", "{seen:#?}");
    assert!(
        endpoint
            .join()
            .expect("endpoint completes")
            .contains("Use British spelling")
    );
    assert_eq!(
        flow_agent_core::conversation_state(&workspace, &session_id).expect("the chat is stored"),
        flow_agent_core::ConversationState::Settled
    );
}

#[test]
fn a_run_the_provider_fails_answers_its_prompt_with_an_error() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let base_url = format!(
        "http://{}/v1",
        listener.local_addr().expect("endpoint address")
    );
    let endpoint = thread::spawn(move || {
        let (mut request, _) = listener.accept().expect("the turn arrives");
        read_request_body(&mut request);
        let body = r#"{"error":{"message":"quota exhausted"}}"#;
        write!(
            request,
            "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("endpoint reply writes");
    });
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    install_note_flow(&base_url, &config_root);
    let mut acp = Acp::start(&workspace, &config_root);
    let (session_id, _) = acp.open(&workspace);

    let prompt = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "A note"}]}),
    );
    let mut seen = Vec::new();
    let answer = acp.reply(prompt, &mut seen);

    endpoint.join().expect("endpoint completes");
    assert!(answer["error"].is_object(), "{answer} after {seen:#?}");
}

const RESEARCH_FLOW: [(&str, &str); 5] = [
    (
        "flows/research.yaml",
        "flow:\n  id: research\n  name: Research\n  phase_refs: [explore]\n",
    ),
    (
        "phases/explore.yaml",
        "phase:\n  id: explore\n  name: Explore\n  instruction_refs: [plan]\n  tool_refs: []\n  subflows:\n    - flow: scraper\n      start: llm\n  output:\n    type: string\n",
    ),
    (
        "instructions/plan.yaml",
        "instruction:\n  id: plan\n  name: Plan\n  prompt: \"Research the topic with scrapers.\"\n",
    ),
    (
        "flows/scraper.yaml",
        "flow:\n  id: scraper\n  name: Scraper\n  phase_refs: [scrape]\n",
    ),
    (
        "phases/scrape.yaml",
        "phase:\n  id: scrape\n  name: Scrape\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
    ),
];

fn reply_with_calls(stream: &mut TcpStream, inputs: &[&str]) {
    let mut events = vec![json!({"type": "response.created", "response": {"id": "resp-calls"}})];
    for (index, input) in inputs.iter().enumerate() {
        events.push(json!({"type": "response.output_item.done", "item": {
            "type": "function_call",
            "call_id": format!("call-{index}"),
            "name": "subflow-scraper",
            "arguments": json!({"input": input}).to_string(),
        }}));
    }
    events.push(json!({"type": "response.completed", "response": {"id": "resp-calls"}}));
    let body = events
        .iter()
        .map(|event| format!("data:{event}\n\n"))
        .collect::<String>();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("endpoint reply writes");
}

fn research_endpoint(
    listener: TcpListener,
    hold_children: Option<Receiver<()>>,
) -> (
    thread::JoinHandle<()>,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = std::sync::Arc::clone(&bodies);
    let hold = std::sync::Arc::new(std::sync::Mutex::new(hold_children));
    let handle = thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let seen = std::sync::Arc::clone(&seen);
            let hold = std::sync::Arc::clone(&hold);
            thread::spawn(move || {
                let body = read_request_body(&mut stream);
                seen.lock().expect("bodies lock").push(body.clone());
                if body.contains("subflow-scraper") && !body.contains("function_call_output") {
                    reply_with_calls(&mut stream, &["rust", "dart"]);
                } else if body.contains("function_call_output") {
                    reply_with_text(&mut stream, "resp-summary", "summary");
                } else {
                    if let Some(hold) = hold.lock().expect("hold lock").as_ref() {
                        let _ = hold.recv_timeout(Duration::from_secs(30));
                    }
                    let topic = if body.contains("dart") {
                        "dart"
                    } else {
                        "rust"
                    };
                    let steered = if body.contains("Focus on Flutter") {
                        " with Flutter"
                    } else {
                        ""
                    };
                    reply_with_text(
                        &mut stream,
                        "resp-scrape",
                        &format!("{topic} facts{steered}"),
                    );
                }
            });
        }
    });
    (handle, bodies)
}

#[test]
fn a_phase_runs_two_scraper_instances_at_once_and_reads_their_results() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let base_url = format!(
        "http://{}/v1",
        listener.local_addr().expect("endpoint address")
    );
    let (_endpoint, bodies) = research_endpoint(listener, None);
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    install_flows(&base_url, &config_root, &RESEARCH_FLOW);
    let mut acp = Acp::start(&workspace, &config_root);
    let (session_id, _) = acp.open(&workspace);
    let mut seen = Vec::new();

    let prompt = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "Compare Rust and Dart"}]}),
    );
    let finished = acp.reply(prompt, &mut seen);

    assert_eq!(finished["result"]["stopReason"], "end_turn", "{seen:#?}");
    let summary = bodies
        .lock()
        .unwrap()
        .iter()
        .find(|body| body.contains("function_call_output"))
        .cloned()
        .expect("the Phase reads its instances' results");
    assert!(
        summary.contains("rust facts") && summary.contains("dart facts"),
        "{summary}"
    );
    let plan = seen
        .iter()
        .rfind(|message| message["params"]["update"]["sessionUpdate"] == "plan")
        .expect("the plan shows the instances");
    let entries = plan["params"]["update"]["entries"].to_string();
    assert!(
        entries.contains("Explore › scraper-1") && entries.contains("Explore › scraper-2"),
        "{entries}"
    );
    drop(acp);
    let mut acp = Acp::start(&workspace, &config_root);
    acp.send("initialize", json!({"protocolVersion": 1}));
    let load = acp.send(
        "session/load",
        json!({"sessionId": session_id, "cwd": workspace.to_path_buf(), "mcpServers": []}),
    );
    acp.reply(load, &mut Vec::new());
    let again = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "Once more"}]}),
    );
    assert_eq!(
        acp.reply(again, &mut Vec::new())["result"]["stopReason"],
        "end_turn"
    );
    for name in ["scraper-1", "scraper-2", "scraper-3", "scraper-4"] {
        assert_eq!(
            flow_agent_core::conversation_state(&workspace, &format!("{session_id}__{name}"))
                .expect("the instance has its own Conversation"),
            flow_agent_core::ConversationState::Settled,
            "{name}"
        );
    }
}

#[test]
fn a_mention_steers_one_running_instance_and_an_unknown_name_is_refused() {
    if run_current_test_isolated_session_home() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("fake endpoint binds");
    let base_url = format!(
        "http://{}/v1",
        listener.local_addr().expect("endpoint address")
    );
    let (release, held) = mpsc::channel::<()>();
    let (_endpoint, bodies) = research_endpoint(listener, Some(held));
    let workspace = workspace_copy("hello-flow");
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    install_flows(&base_url, &config_root, &RESEARCH_FLOW);
    let mut acp = Acp::start(&workspace, &config_root);
    let (session_id, _) = acp.open(&workspace);
    let mut seen = Vec::new();
    let first = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "Compare Rust and Dart"}]}),
    );
    acp.next(
        |message| {
            message["params"]["update"]["entries"]
                .to_string()
                .contains("scraper-2")
        },
        &mut seen,
    );
    for _ in 0..200 {
        if bodies
            .lock()
            .unwrap()
            .iter()
            .filter(|body| !body.contains("subflow-scraper"))
            .count()
            == 2
        {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    acp.notify("session/cancel", json!({"sessionId": session_id}));
    assert_eq!(
        acp.reply(first, &mut seen)["result"]["stopReason"],
        "cancelled"
    );

    let unknown = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "@scraper-9 Hello"}]}),
    );
    let refused = acp.reply(unknown, &mut seen);
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("scraper-2"),
        "{refused}"
    );
    let steer = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "@scraper-2 Focus on Flutter"}]}),
    );
    for _ in 0..3 {
        release.send(()).expect("a held turn is released");
    }
    let finished = acp.reply(steer, &mut seen);

    assert_eq!(finished["result"]["stopReason"], "end_turn", "{seen:#?}");
    let all = bodies.lock().unwrap().clone();
    assert!(
        all.iter()
            .any(|body| body.contains("dart") && body.contains("Focus on Flutter")),
        "the steering message reaches scraper-2"
    );
    assert!(
        !all.iter()
            .any(|body| body.contains("rust") && body.contains("Focus on Flutter")),
        "scraper-1 never sees it"
    );
    let summary = all
        .iter()
        .find(|body| body.contains("function_call_output"))
        .expect("the Phase reads the results");
    assert!(summary.contains("dart facts with Flutter"), "{summary}");
}
