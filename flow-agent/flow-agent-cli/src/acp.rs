use crate::{interrupt::InterruptCoordinator, streaming::stream_live_operation_into};
use flow_agent_core::{ConversationState, EmitMode, RunOutput, RuntimeError};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::mpsc::{self, Sender},
    thread,
};

mod updates;

use updates::RunView;

const FLOW_OPTION: &str = "flow";
const MAX_FLOW_CHOICES: usize = 64;
const SESSION_PREFIX: &str = "chat-";
const SESSION_RANDOM_BYTES: usize = 16;
pub(crate) const MAX_ACP_LINE_BYTES: usize = 8 * flow_agent_core::MAX_STEERING_MESSAGE_BYTES;
const PARSE_ERROR: i64 = -32700;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

enum Input {
    Message(Value),
    Malformed,
    Event(Value),
    Finished(Result<RunOutput, RuntimeError>),
    Closed,
}

struct Session {
    id: String,
    cwd: PathBuf,
    flows: Vec<core_script::BlockIdentity>,
    flow: Option<String>,
}

struct Agent<W: Write> {
    out: W,
    sender: Sender<Input>,
    session: Option<Session>,
    prompt: Option<Value>,
    running: bool,
    view: RunView,
}

pub(crate) fn acp_command() -> Result<ExitCode, RuntimeError> {
    let (sender, receiver) = mpsc::channel();
    let reader = sender.clone();
    thread::Builder::new()
        .name("flow-acp-input".to_owned())
        .spawn(move || read_messages(io::stdin().lock(), &reader))
        .map_err(|source| RuntimeError::Io {
            path: PathBuf::from("<acp-input-thread>"),
            source,
        })?;
    let mut agent = Agent {
        out: io::stdout(),
        sender,
        session: None,
        prompt: None,
        running: false,
        view: RunView::default(),
    };
    for input in receiver {
        if !agent.handle(input)? {
            break;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn read_messages(mut input: impl BufRead, sender: &Sender<Input>) {
    loop {
        let mut line = Vec::new();
        match (&mut input)
            .take(MAX_ACP_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.len() > MAX_ACP_LINE_BYTES {
            let _ = sender.send(Input::Malformed);
            break;
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message = serde_json::from_slice(&line).map_or(Input::Malformed, Input::Message);
        if sender.send(message).is_err() {
            return;
        }
    }
    let _ = sender.send(Input::Closed);
}

impl<W: Write> Agent<W> {
    fn handle(&mut self, input: Input) -> Result<bool, RuntimeError> {
        match input {
            Input::Message(message) => self.message(message)?,
            Input::Malformed => self.fail(&Value::Null, PARSE_ERROR, "message is not JSON")?,
            Input::Event(event) => {
                let updates = self.view.observe(&event);
                if self.prompt.is_some() {
                    for update in updates {
                        self.update(update)?;
                    }
                }
            }
            Input::Finished(result) => self.finished(result)?,
            Input::Closed => return Ok(false),
        }
        Ok(true)
    }

    fn message(&mut self, message: Value) -> Result<(), RuntimeError> {
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = message.get("id").cloned() else {
            if message["method"] == "session/cancel" {
                self.cancel()?;
            }
            return Ok(());
        };
        let outcome = match message["method"].as_str() {
            Some("initialize") => Ok(initialize()),
            Some("session/new") => self.new_session(&params),
            Some("session/load") => self.load_session(&params),
            Some("session/set_config_option") => self.set_flow(&params),
            Some("session/prompt") => return self.prompt(id, &params),
            Some(_) => Err((METHOD_NOT_FOUND, "method is not supported".to_owned())),
            None => return Ok(()),
        };
        match outcome {
            Ok(result) => self.respond(&id, result),
            Err((code, message)) => self.fail(&id, code, &message),
        }
    }

    fn new_session(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let cwd = session_cwd(params)?;
        self.ensure_idle()?;
        let id = new_session_id().map_err(internal)?;
        let flows = installed_flows()?;
        let flow = flows.first().map(|flow| flow.id.clone());
        let session = self.session.insert(Session {
            id,
            cwd,
            flows,
            flow,
        });
        Ok(json!({"sessionId": session.id, "configOptions": config_options(session)}))
    }

    fn load_session(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let cwd = session_cwd(params)?;
        let id = params["sessionId"].as_str().unwrap_or_default();
        if !is_session_id(id) {
            return Err((
                INVALID_PARAMS,
                "sessionId is not a Flow Agent chat".to_owned(),
            ));
        }
        self.ensure_idle()?;
        let flows = installed_flows()?;
        let flow = flows.first().map(|flow| flow.id.clone());
        let session = self.session.insert(Session {
            id: id.to_owned(),
            cwd,
            flows,
            flow,
        });
        Ok(json!({"configOptions": config_options(session)}))
    }

    fn set_flow(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let session = self.session_for(params)?;
        if params["configId"] != FLOW_OPTION {
            return Err((INVALID_PARAMS, "configId is unknown".to_owned()));
        }
        let value = params["value"].as_str().unwrap_or_default();
        if !session.flows.iter().any(|flow| flow.id == value) {
            return Err((INVALID_PARAMS, format!("Flow {value} is not installed")));
        }
        if flow_agent_core::conversation_state(&session.cwd, &session.id).map_err(internal)?
            != ConversationState::Absent
        {
            return Err((
                INVALID_PARAMS,
                "the Flow is fixed once the chat has started".to_owned(),
            ));
        }
        session.flow = Some(value.to_owned());
        Ok(json!({"configOptions": config_options(session)}))
    }

    fn prompt(&mut self, id: Value, params: &Value) -> Result<(), RuntimeError> {
        let text = match self
            .session_for(params)
            .map(|_| user_text(&params["prompt"]))
        {
            Ok(Some(text)) => text,
            Ok(None) => return self.fail(&id, INVALID_PARAMS, "prompt contains no text"),
            Err((code, message)) => return self.fail(&id, code, &message),
        };
        if self.prompt.is_some() {
            return self.fail(&id, INVALID_PARAMS, "a prompt is already running");
        }
        if self.running {
            if let Err(error) = flow_agent_core::steer_productive_run(text) {
                return self.fail(&id, INVALID_PARAMS, &error.to_string());
            }
            self.prompt = Some(id);
            return self.update(self.view.plan());
        }
        self.prompt = Some(id.clone());
        if let Err((code, message)) = self.start(text, Vec::new()) {
            self.prompt = None;
            return self.fail(&id, code, &message);
        }
        Ok(())
    }

    fn start(&mut self, text: String, steering: Vec<String>) -> Result<(), (i64, String)> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| (INVALID_PARAMS, "no session is open".to_owned()))?;
        let state =
            flow_agent_core::conversation_state(&session.cwd, &session.id).map_err(internal)?;
        let flow = session.flow.clone();
        if state == ConversationState::Absent && flow.is_none() {
            return Err((INVALID_PARAMS, "no Flow is installed".to_owned()));
        }
        let cwd = session.cwd.clone();
        let conversation_id = session.id.clone();
        let sender = self.sender.clone();
        self.view = RunView::default();
        self.running = true;
        thread::Builder::new()
            .name("flow-acp-run".to_owned())
            .spawn(move || {
                let result = run(cwd, conversation_id, flow, state, text, steering, &sender);
                let _ = sender.send(Input::Finished(result));
            })
            .map(|_| ())
            .map_err(|error| {
                self.running = false;
                internal(error)
            })
    }

    fn cancel(&mut self) -> Result<(), RuntimeError> {
        if self.running {
            flow_agent_core::request_productive_pause();
        }
        match self.prompt.take() {
            Some(id) => self.respond(&id, json!({"stopReason": "cancelled"})),
            None => Ok(()),
        }
    }

    fn finished(&mut self, result: Result<RunOutput, RuntimeError>) -> Result<(), RuntimeError> {
        self.running = false;
        let mut waiting = flow_agent_core::drain_productive_steering();
        if result.is_ok() && !waiting.is_empty() {
            let text = waiting.remove(0);
            if self.start(text, waiting).is_ok() {
                return Ok(());
            }
        }
        let Some(id) = self.prompt.take() else {
            return Ok(());
        };
        match result {
            Ok(_) => self.respond(&id, json!({"stopReason": "end_turn"})),
            Err(error) => self.fail(&id, INTERNAL_ERROR, &error.to_string()),
        }
    }

    fn ensure_idle(&self) -> Result<(), (i64, String)> {
        if self.running {
            return Err((INVALID_PARAMS, "a Run is still active".to_owned()));
        }
        Ok(())
    }

    fn session_for(&mut self, params: &Value) -> Result<&mut Session, (i64, String)> {
        self.session
            .as_mut()
            .filter(|session| params["sessionId"] == session.id.as_str())
            .ok_or_else(|| (INVALID_PARAMS, "sessionId is unknown".to_owned()))
    }

    fn update(&mut self, update: Value) -> Result<(), RuntimeError> {
        let Some(session) = &self.session else {
            return Ok(());
        };
        let params = json!({"sessionId": session.id, "update": update});
        self.write(&json!({"jsonrpc": "2.0", "method": "session/update", "params": params}))
    }

    fn respond(&mut self, id: &Value, result: Value) -> Result<(), RuntimeError> {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn fail(&mut self, id: &Value, code: i64, message: &str) -> Result<(), RuntimeError> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message},
        }))
    }

    fn write(&mut self, message: &Value) -> Result<(), RuntimeError> {
        let mut line = serde_json::to_vec(message).map_err(RuntimeError::Json)?;
        line.push(b'\n');
        self.out
            .write_all(&line)
            .and_then(|()| self.out.flush())
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<stdout>"),
                source,
            })
    }
}

fn run(
    cwd: PathBuf,
    conversation_id: String,
    flow: Option<String>,
    state: ConversationState,
    text: String,
    steering: Vec<String>,
    sender: &Sender<Input>,
) -> Result<RunOutput, RuntimeError> {
    let operation = InterruptCoordinator::new().operation();
    let mut forwarder = EventForwarder {
        sender: sender.clone(),
        line: Vec::new(),
    };
    let workspace = cwd.clone();
    stream_live_operation_into(
        cwd,
        move |notifier| {
            let interrupted = matches!(state, ConversationState::Interrupted { .. });
            let input = core_script::FlowValue::String(text.clone());
            let mut pending = steering;
            if interrupted {
                pending.insert(0, text);
            }
            let activate = move |productive: bool| {
                if productive {
                    operation.activate()?;
                }
                for message in pending.drain(..) {
                    flow_agent_core::steer_productive_run(message)?;
                }
                Ok(operation)
            };
            match state {
                ConversationState::Absent => flow_agent_core::run_flow_with_execution_activation(
                    &workspace,
                    flow.as_deref().unwrap_or_default(),
                    Some(&conversation_id),
                    Some(input),
                    Some(notifier),
                    EmitMode::Jsonl,
                    activate,
                ),
                ConversationState::Settled => {
                    flow_agent_core::continue_conversation_with_execution_activation(
                        &workspace,
                        &conversation_id,
                        None,
                        Some(input),
                        Some(notifier),
                        EmitMode::Jsonl,
                        activate,
                    )
                }
                ConversationState::Interrupted { run_session_id } => {
                    flow_agent_core::resume_conversation_run_with_execution_activation(
                        &workspace,
                        &conversation_id,
                        &run_session_id,
                        Some(notifier),
                        EmitMode::Jsonl,
                        activate,
                    )
                }
            }
        },
        &mut forwarder,
    )
}

struct EventForwarder {
    sender: Sender<Input>,
    line: Vec<u8>,
}

impl Write for EventForwarder {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        for byte in buffer {
            if *byte != b'\n' {
                self.line.push(*byte);
                continue;
            }
            let line = std::mem::take(&mut self.line);
            if let Ok(event) = serde_json::from_slice(&line)
                && self.sender.send(Input::Event(event)).is_err()
            {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn initialize() -> Value {
    json!({
        "protocolVersion": 1,
        "agentCapabilities": {
            "loadSession": true,
            "promptCapabilities": {"image": false, "audio": false, "embeddedContext": false},
        },
        "agentInfo": {"name": "flow", "title": "Flow Agent", "version": env!("CARGO_PKG_VERSION")},
        "authMethods": [],
    })
}

fn config_options(session: &Session) -> Value {
    let choices = session
        .flows
        .iter()
        .take(MAX_FLOW_CHOICES)
        .map(|flow| json!({"value": flow.id, "name": flow.name}))
        .collect::<Vec<_>>();
    json!([{
        "id": FLOW_OPTION,
        "name": "Flow",
        "description": "The Flow this chat runs",
        "type": "select",
        "currentValue": session.flow.clone().unwrap_or_default(),
        "options": choices,
    }])
}

fn installed_flows() -> Result<Vec<core_script::BlockIdentity>, (i64, String)> {
    flow_agent_core::list_global_flows()
        .map(|flows| {
            flows
                .into_iter()
                .map(|flow| (flow.id.clone(), flow))
                .collect::<BTreeMap<_, _>>()
                .into_values()
                .collect()
        })
        .map_err(internal)
}

fn session_cwd(params: &Value) -> Result<PathBuf, (i64, String)> {
    params["cwd"]
        .as_str()
        .map(PathBuf::from)
        .filter(|cwd| cwd.is_absolute())
        .ok_or_else(|| (INVALID_PARAMS, "cwd must be an absolute path".to_owned()))
}

fn user_text(prompt: &Value) -> Option<String> {
    let text = prompt
        .as_array()?
        .iter()
        .find(|block| block["type"] == "text")?["text"]
        .as_str()?;
    const OPEN: &str = "<user_request>\n";
    const CLOSE: &str = "\n</user_request>";
    let request = match (text.rfind(OPEN), text.rfind(CLOSE)) {
        (Some(start), Some(end)) if start + OPEN.len() <= end => &text[start + OPEN.len()..end],
        _ => text,
    };
    (!request.trim().is_empty()).then(|| request.to_owned())
}

fn new_session_id() -> io::Result<String> {
    let mut bytes = [0_u8; SESSION_RANDOM_BYTES];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes
        .iter()
        .fold(SESSION_PREFIX.to_owned(), |mut id, byte| {
            id.push_str(&format!("{byte:02x}"));
            id
        }))
}

fn is_session_id(id: &str) -> bool {
    id.strip_prefix(SESSION_PREFIX).is_some_and(|random| {
        random.len() == 2 * SESSION_RANDOM_BYTES
            && random
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn internal(error: impl ToString) -> (i64, String) {
    (INTERNAL_ERROR, error.to_string())
}
