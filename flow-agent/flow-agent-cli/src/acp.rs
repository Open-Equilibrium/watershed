use crate::{
    interrupt::InterruptCoordinator,
    streaming::stream_live_operation_into,
    subflows::{InstanceEvent, root_launcher},
};
use flow_agent_core::{ConversationState, EmitMode, RunOutput, RuntimeError};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    process::ExitCode,
    sync::mpsc::{self, SyncSender},
    thread,
};

mod updates;

use updates::RunView;

const FLOW_OPTION: &str = "flow";
const MAX_FLOW_CHOICES: usize = 64;
const SESSION_PREFIX: &str = "chat-";
const SESSION_RANDOM_BYTES: usize = 16;
pub(crate) const MAX_ACP_LINE_BYTES: usize = 8 * flow_agent_core::MAX_STEERING_MESSAGE_BYTES;
const INPUT_CAPACITY: usize = 64;
const MAX_HELD_UPDATES: usize = 256;
const PARSE_ERROR: i64 = -32700;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

enum Input {
    Message(Value),
    Malformed,
    Event(Value),
    Instance(InstanceEvent),
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
    sender: SyncSender<Input>,
    session: Option<Session>,
    prompt: Option<Value>,
    running: bool,
    view: RunView,
    held: Vec<Value>,
}

pub(crate) fn acp_command() -> Result<ExitCode, RuntimeError> {
    let (sender, receiver) = mpsc::sync_channel(INPUT_CAPACITY);
    let reader = sender.clone();
    thread::Builder::new()
        .name("flow-acp-input".to_owned())
        .spawn(move || read_messages(io::stdin().lock(), &reader))
        .map_err(|source| RuntimeError::Io {
            path: PathBuf::from("<acp-input-thread>"),
            source,
        })?;
    if let Some(launcher) = root_launcher() {
        let (instances, observed) = mpsc::channel();
        launcher.observe(instances);
        let forward = sender.clone();
        thread::spawn(move || {
            for event in observed {
                if forward.send(Input::Instance(event)).is_err() {
                    return;
                }
            }
        });
    }
    let mut agent = Agent {
        out: io::stdout(),
        sender,
        session: None,
        prompt: None,
        running: false,
        view: RunView::default(),
        held: Vec::new(),
    };
    for input in receiver {
        if !agent.handle(input)? {
            break;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn read_messages(mut input: impl BufRead, sender: &SyncSender<Input>) {
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
            if !line.ends_with(b"\n") && !skip_line(&mut input) {
                break;
            }
            if sender.send(Input::Malformed).is_err() {
                return;
            }
            continue;
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

fn skip_line(input: &mut impl BufRead) -> bool {
    loop {
        let Ok(buffer) = input.fill_buf() else {
            return false;
        };
        if buffer.is_empty() {
            return false;
        }
        if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            input.consume(end + 1);
            return true;
        }
        let length = buffer.len();
        input.consume(length);
    }
}

impl<W: Write> Agent<W> {
    fn handle(&mut self, input: Input) -> Result<bool, RuntimeError> {
        match input {
            Input::Message(message) => self.message(message)?,
            Input::Malformed => self.fail(&Value::Null, PARSE_ERROR, "message is not JSON")?,
            Input::Event(event) => {
                let updates = self.view.observe(&event);
                self.forward(updates)?;
            }
            Input::Instance(event) => {
                let update = self.view.instance(&event);
                self.forward(vec![update])?;
            }
            Input::Finished(result) => self.finished(result)?,
            Input::Closed => return Ok(false),
        }
        Ok(true)
    }

    fn forward(&mut self, updates: Vec<Value>) -> Result<(), RuntimeError> {
        if self.prompt.is_some() {
            for update in updates {
                self.update(update)?;
            }
        } else {
            self.held.extend(updates);
            let excess = self.held.len().saturating_sub(MAX_HELD_UPDATES);
            self.held.drain(..excess);
        }
        Ok(())
    }

    fn message(&mut self, message: Value) -> Result<(), RuntimeError> {
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = message.get("id").cloned() else {
            if message["method"] == "session/cancel" && self.session_for(&params).is_ok() {
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
        self.held.clear();
        self.view = RunView::default();
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
        let flow = flow_agent_core::conversation_flow(&cwd, id)
            .map_err(internal)?
            .or_else(|| flows.first().map(|flow| flow.id.clone()));
        self.held.clear();
        self.view = RunView::default();
        let session = self.session.insert(Session {
            id: id.to_owned(),
            cwd,
            flows,
            flow,
        });
        Ok(json!({"configOptions": config_options(session)}))
    }

    fn set_flow(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        if params["configId"] != FLOW_OPTION {
            return Err((INVALID_PARAMS, "configId is unknown".to_owned()));
        }
        if self.running {
            return Err((
                INVALID_PARAMS,
                "the Flow is fixed once the chat has started".to_owned(),
            ));
        }
        let session = self.session_for(params)?;
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
        if text.len() > flow_agent_core::MAX_FLOW_RUN_INPUT_BYTES {
            return self.fail(
                &id,
                INVALID_PARAMS,
                &format!(
                    "a message may hold at most {} bytes",
                    flow_agent_core::MAX_FLOW_RUN_INPUT_BYTES
                ),
            );
        }
        let live = if self.running {
            root_launcher()
                .map(|launcher| launcher.live_names())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Some((target, message)) = mention(&text).filter(|_| !live.is_empty()) {
            if !live.iter().any(|name| name == target) {
                return self.fail(
                    &id,
                    INVALID_PARAMS,
                    &format!(
                        "no active subflow instance is named {target}; active: {}",
                        if live.is_empty() {
                            "none".to_owned()
                        } else {
                            live.join(", ")
                        }
                    ),
                );
            }
            if message.trim().is_empty() {
                return self.fail(
                    &id,
                    INVALID_PARAMS,
                    "a message to an instance must contain text",
                );
            }
            flow_agent_core::resume_productive_run();
            if let Some(launcher) = root_launcher() {
                launcher.resume_all();
                launcher.steer(target, message);
            }
            self.prompt = Some(id);
            self.update(self.view.plan())?;
            return self.release_held();
        }
        if self.running {
            if let Err(error) = flow_agent_core::steer_productive_run(text) {
                return self.fail(&id, INVALID_PARAMS, &error.to_string());
            }
            if let Some(launcher) = root_launcher() {
                launcher.resume_all();
            }
            self.prompt = Some(id);
            self.update(self.view.plan())?;
            return self.release_held();
        }
        self.prompt = Some(id.clone());
        if let Err((code, message)) = self.start(text, Vec::new()) {
            self.prompt = None;
            return self.fail(&id, code, &message);
        }
        self.release_held()
    }

    fn release_held(&mut self) -> Result<(), RuntimeError> {
        for update in std::mem::take(&mut self.held) {
            self.update(update)?;
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
        let resumed = matches!(state, ConversationState::Interrupted { .. });
        for message in resumed.then(|| text.clone()).into_iter().chain(steering) {
            if let Err(error) = flow_agent_core::steer_productive_run(message) {
                flow_agent_core::drain_productive_steering();
                return Err(internal(error));
            }
        }
        self.view = RunView::default();
        self.running = true;
        thread::Builder::new()
            .name("flow-acp-run".to_owned())
            .spawn(move || {
                let result = run(cwd, conversation_id, flow, state, text, &sender);
                let _ = sender.send(Input::Finished(result));
            })
            .map(|_| ())
            .map_err(|error| {
                self.running = false;
                flow_agent_core::drain_productive_steering();
                internal(error)
            })
    }

    fn cancel(&mut self) -> Result<(), RuntimeError> {
        if self.running {
            flow_agent_core::request_productive_pause();
            if let Some(launcher) = root_launcher() {
                launcher.pause_all();
            }
        }
        match self.prompt.take() {
            Some(id) => self.respond(&id, json!({"stopReason": "cancelled"})),
            None => Ok(()),
        }
    }

    fn finished(&mut self, result: Result<RunOutput, RuntimeError>) -> Result<(), RuntimeError> {
        self.running = false;
        let mut waiting = flow_agent_core::drain_productive_steering();
        let result = match result {
            Ok(output) if output.failed => Err(RuntimeError::Protocol(format!(
                "Run {} of Flow Agent failed",
                output.session_id
            ))),
            other => other,
        };
        let result = match result {
            Ok(_) if !waiting.is_empty() => {
                let count = waiting.len();
                let text = waiting.remove(0);
                match self.start(text, waiting) {
                    Ok(()) => return Ok(()),
                    Err((code, message)) => Err((
                        code,
                        format!("{message}; {count} waiting message(s) were not delivered"),
                    )),
                }
            }
            Ok(_) => Ok(()),
            Err(error) if waiting.is_empty() => Err(internal(error)),
            Err(error) => Err(internal(format!(
                "{error}; {} waiting message(s) were not delivered",
                waiting.len()
            ))),
        };
        let Some(id) = self.prompt.take() else {
            return Ok(());
        };
        match result {
            Ok(()) => self.respond(&id, json!({"stopReason": "end_turn"})),
            Err((code, message)) => self.fail(&id, code, &message),
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
    sender: &SyncSender<Input>,
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
            let input = core_script::FlowValue::String(text);
            let activate = move |productive: bool| {
                if productive {
                    operation.activate()?;
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
    sender: SyncSender<Input>,
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

fn mention(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('@')?;
    let (target, message) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    (!target.is_empty()).then(|| (target, message.trim_start()))
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

#[cfg(test)]
mod tests {
    use super::{Agent, Input, MAX_HELD_UPDATES, RunView, mention};
    use crate::subflows::InstanceEvent;
    use serde_json::json;

    #[test]
    fn updates_held_without_an_open_prompt_stay_bounded() {
        let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
        let mut agent = Agent {
            out: Vec::new(),
            sender,
            session: None,
            prompt: None,
            running: true,
            view: RunView::default(),
            held: Vec::new(),
        };

        for index in 0..MAX_HELD_UPDATES + 10 {
            let event = json!({
                "event_type": "tool.started",
                "payload": {"attempt_id": format!("tool-{index}"), "tool_name": "echo"},
            });
            agent
                .handle(Input::Event(event))
                .expect("the event is observed");
        }

        agent
            .handle(Input::Instance(InstanceEvent::Started {
                name: "scraper-1".to_owned(),
            }))
            .expect("the instance is observed");

        assert_eq!(agent.held.len(), MAX_HELD_UPDATES);
        assert_eq!(agent.held[0]["toolCallId"], "tool-11");
        assert!(agent.out.is_empty());
    }

    #[test]
    fn a_bare_mention_names_its_target_with_an_empty_message() {
        assert_eq!(mention("@scraper-2"), Some(("scraper-2", "")));
        assert_eq!(mention("@scraper-2 Go on"), Some(("scraper-2", "Go on")));
        assert_eq!(mention("@ Go on"), None);
    }

    #[test]
    fn an_instance_that_never_started_adds_no_plan_entry() {
        let mut view = RunView::default();

        let plan = view.instance(&InstanceEvent::Finished {
            name: "scraper-1".to_owned(),
            failed: true,
        });

        assert_eq!(plan["entries"], json!([]));
    }
}
