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
    assert_eq!(choices, ["hello-flow", "hello-subflow"]);
    assert_eq!(options[0]["currentValue"], "hello-flow");

    let unknown = acp.send(
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": "flow", "value": "missing"}),
    );
    assert_eq!(acp.reply(unknown, &mut Vec::new())["error"]["code"], -32602);
    let chosen = acp.send(
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": "flow", "value": "hello-subflow"}),
    );
    assert_eq!(
        acp.reply(chosen, &mut Vec::new())["result"]["configOptions"][0]["currentValue"],
        "hello-subflow"
    );
    let unsupported = acp.send("session/set_mode", json!({"sessionId": session_id}));
    assert_eq!(
        acp.reply(unsupported, &mut Vec::new())["error"]["code"],
        -32601
    );
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

const NOTE_FLOW: [(&str, &str); 3] = [
    (
        "flows/note.yaml",
        "flow:\n  id: note\n  name: Note\n  phase_refs: [write]\n  subflow_refs: []\n",
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
    let home = session_home_path();
    let registry = home.join("registry");
    fs::remove_dir_all(&registry).expect("the fixture registry is replaced");
    for (path, source) in NOTE_FLOW {
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
    let endpoint = thread::spawn(move || {
        let (mut first, _) = listener.accept().expect("the first turn arrives");
        let first_body = read_request_body(&mut first);
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
    acp.next(
        |message| message["params"]["update"]["entries"][0]["status"] == "in_progress",
        &mut seen,
    );
    acp.notify("session/cancel", json!({"sessionId": session_id}));
    assert_eq!(
        acp.reply(first, &mut seen)["result"]["stopReason"],
        "cancelled"
    );
    let steer = acp.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": "Use British spelling"}]}),
    );
    release_first.send(()).expect("the endpoint waits");
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
    assert_eq!(
        flow_agent_core::conversation_state(&workspace, &session_id).expect("the chat is stored"),
        flow_agent_core::ConversationState::Settled
    );
}
