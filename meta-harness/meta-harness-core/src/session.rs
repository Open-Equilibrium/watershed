//! A running agent process behind the runtime event model.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::future::{Future, poll_fn};
use std::path::PathBuf;
use std::pin::{Pin, pin};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

use crate::approval::{ApprovalMode, needs_user};
use crate::pi::queue;
use crate::{
    Answer, Decision, Delivery, Frame, History, LineFramer, NotificationLevel, PiAdapter, Queue,
    RequestId, RuntimeEvent,
};

/// Largest record accepted from an agent; a larger one is skipped and fails its command.
const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// Events buffered before the session stops reading agent output, which pauses the agent.
const EVENT_BUFFER: usize = 256;
/// Records buffered for the agent's input.
const WRITE_BUFFER: usize = 64;
const READ_BUFFER: usize = 64 * 1024;

/// How to start an agent process that speaks pi's RPC protocol.
#[derive(Debug, Clone)]
pub struct AgentCommand {
    /// The executable.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// Its working directory.
    pub cwd: PathBuf,
}

/// Something that happened in a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// A runtime event from the agent.
    Runtime(RuntimeEvent),
    /// The agent broke its protocol, so the session stopped it.
    Failed(String),
    /// The agent process exited; this is always the last event.
    Exited {
        /// Whether the process exited successfully.
        success: bool,
    },
}

/// Error returned for a command the agent did not accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// The agent rejected the command.
    Rejected(String),
    /// The request is not open, for example because it was already answered.
    UnknownRequest,
    /// The agent process is gone.
    Closed,
    /// The agent's response exceeded the record limit and was skipped.
    TooLarge,
}

type Reply = oneshot::Sender<Result<Value, CommandError>>;

/// A command the agent received but has not answered yet. Waiting for it needs no access to the
/// session, so the agent's questions can be answered meanwhile: pi accepts a prompt only after
/// its command handlers ran, and they may ask the user first.
#[derive(Debug)]
pub struct Pending(oneshot::Receiver<Result<Value, CommandError>>);

impl Pending {
    /// Waits until the agent accepted or rejected the command.
    pub async fn accepted(self) -> Result<(), CommandError> {
        self.0.await.unwrap_or(Err(CommandError::Closed)).map(drop)
    }
}

/// What the agent reports about its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    /// The session file, unless the session is in memory only.
    pub session_file: Option<PathBuf>,
    /// The agent's session id.
    pub session_id: String,
    /// The selected model as `provider/id`.
    pub model: Option<String>,
    /// Whether a turn is streaming.
    pub streaming: bool,
}

enum Write {
    Record(Vec<u8>),
    Close,
}

/// State shared between the session handle and its supervisor.
#[derive(Debug, Default)]
struct Shared {
    /// Commands awaiting their response; `None` once the agent is gone.
    replies: Option<HashMap<u64, Reply>>,
    mode: ApprovalMode,
    /// Tools the user allowed for the rest of the session.
    allowed: HashSet<String>,
    /// Open approval requests and the tool each one would call.
    approvals: HashMap<RequestId, String>,
    /// Open questions from agent extensions.
    questions: HashSet<RequestId>,
    /// Whether a turn is running, from the agent's events.
    running: bool,
}

#[derive(Debug, Clone)]
struct State(Arc<Mutex<Shared>>);

impl State {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A running agent process; dropping it kills the process.
#[derive(Debug)]
pub struct Session {
    writer: mpsc::Sender<Write>,
    events: mpsc::Sender<SessionEvent>,
    state: State,
    next_id: u64,
    kill: Option<oneshot::Sender<()>>,
    exited: oneshot::Receiver<bool>,
    /// The stored entries read so far; later reads continue after the last one.
    history: History,
}

impl Session {
    /// Starts the agent and returns the session with its event stream.
    pub fn start(command: &AgentCommand) -> std::io::Result<(Self, mpsc::Receiver<SessionEvent>)> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stdin = child.stdin.take().expect("stdin is piped");
        let (writer, writes) = mpsc::channel(WRITE_BUFFER);
        let (events, receiver) = mpsc::channel(EVENT_BUFFER);
        let (kill, killed) = oneshot::channel();
        let (exit, exited) = oneshot::channel();
        let state = State(Arc::new(Mutex::new(Shared {
            replies: Some(HashMap::new()),
            ..Shared::default()
        })));
        tokio::spawn(write(stdin, writes));
        let supervisor = Supervisor {
            events: events.clone(),
            writer: writer.clone(),
            state: state.clone(),
            adapter: PiAdapter::new(),
            started: Instant::now(),
            killed,
            kill_requested: false,
        };
        tokio::spawn(supervisor.run(child, stdout, exit));
        let session = Self {
            writer,
            events,
            state,
            next_id: 0,
            kill: Some(kill),
            exited,
            history: History::default(),
        };
        Ok((session, receiver))
    }

    /// Sends a prompt that starts a turn.
    pub async fn prompt(&mut self, message: &str) -> Result<Pending, CommandError> {
        self.submit(json!({ "type": "prompt", "message": message }))
            .await
    }

    /// Sends a message the agent starts with when idle and takes as `delivery` says while
    /// running; pi decides which applies, so no turn state can race the send.
    pub async fn send(
        &mut self,
        message: &str,
        delivery: Delivery,
    ) -> Result<Pending, CommandError> {
        let behavior = match delivery {
            Delivery::Steer => "steer",
            Delivery::FollowUp => "followUp",
        };
        self.submit(json!({
            "type": "prompt",
            "message": message,
            "streamingBehavior": behavior,
        }))
        .await
    }

    /// Removes the waiting messages and returns them, so the user can edit and send them again.
    pub async fn clear_queue(&mut self) -> Result<Queue, CommandError> {
        let data = self.command(json!({ "type": "clear_queue" })).await?;
        Ok(queue(&data))
    }

    /// Aborts the running turn.
    pub async fn abort(&mut self) -> Result<(), CommandError> {
        self.command(json!({ "type": "abort" })).await.map(drop)
    }

    /// Reads the session's file, id, model and streaming state.
    pub async fn state(&mut self) -> Result<SessionState, CommandError> {
        let data = self.command(json!({ "type": "get_state" })).await?;
        let text = |field| data.get(field).and_then(Value::as_str);
        let model = data.get("model").and_then(|model| {
            Some(format!(
                "{}/{}",
                model.get("provider")?.as_str()?,
                model.get("id")?.as_str()?
            ))
        });
        Ok(SessionState {
            session_file: text("sessionFile").map(PathBuf::from),
            session_id: text("sessionId").unwrap_or_default().to_owned(),
            model,
            streaming: data.get("isStreaming").and_then(Value::as_bool) == Some(true),
        })
    }

    /// Reads the session's stored entries, including abandoned branches. Only entries added
    /// since the previous read cross the agent's output; when the agent no longer knows the
    /// last one, everything is read again.
    pub async fn history(&mut self) -> Result<History, CommandError> {
        let since = self.history.last_id().map(str::to_owned);
        let mut request = json!({ "type": "get_entries" });
        if let Some(since) = &since {
            request["since"] = json!(since);
        }
        let data = match self.command(request).await {
            Err(CommandError::Rejected(_)) if since.is_some() => {
                self.history = History::default();
                self.command(json!({ "type": "get_entries" })).await?
            }
            result => result?,
        };
        self.history.append(&data);
        Ok(self.history.clone())
    }

    /// Continues the session from `entry`, optionally labelling it; the next prompt starts a
    /// new branch there. Requires Watershed's pi extension.
    pub async fn branch(
        &mut self,
        entry: &str,
        label: Option<&str>,
    ) -> Result<Pending, CommandError> {
        let message = match label {
            Some(label) => format!("/watershed-branch {entry} {label}"),
            None => format!("/watershed-branch {entry}"),
        };
        self.prompt(&message).await
    }

    /// Whether the agent is running a turn, as its latest events say.
    pub fn running(&self) -> bool {
        self.state.lock().running
    }

    /// Changes which tool calls need the user's approval.
    pub fn set_approval_mode(&self, mode: ApprovalMode) {
        self.state.lock().mode = mode;
    }

    /// Answers an open approval request.
    pub async fn decide(
        &mut self,
        request: &RequestId,
        decision: Decision,
    ) -> Result<(), CommandError> {
        {
            let mut shared = self.state.lock();
            let tool = shared
                .approvals
                .remove(request)
                .ok_or(CommandError::UnknownRequest)?;
            if decision == Decision::AllowSession {
                shared.allowed.insert(tool);
            }
        }
        let value = match decision {
            Decision::AllowOnce => "allow-once",
            Decision::AllowSession => "allow-session",
            Decision::Reject => "reject",
        };
        self.respond(request, json!({ "value": value })).await?;
        let resolved = RuntimeEvent::RequestResolved {
            request: request.clone(),
            decision,
        };
        let _ = self.events.send(SessionEvent::Runtime(resolved)).await;
        Ok(())
    }

    /// Answers an open question from an agent extension.
    pub async fn answer(
        &mut self,
        request: &RequestId,
        answer: Answer,
    ) -> Result<(), CommandError> {
        if !self.state.lock().questions.remove(request) {
            return Err(CommandError::UnknownRequest);
        }
        let reply = match answer {
            Answer::Value(value) => json!({ "value": value }),
            Answer::Confirmed(confirmed) => json!({ "confirmed": confirmed }),
            Answer::Cancelled => json!({ "cancelled": true }),
        };
        self.respond(request, reply).await?;
        let resolved = RuntimeEvent::UserInputResolved {
            request: request.clone(),
        };
        let _ = self.events.send(SessionEvent::Runtime(resolved)).await;
        Ok(())
    }

    /// Asks the agent to exit and kills it after `grace`; returns whether it exited successfully.
    pub async fn shutdown(mut self, grace: Duration) -> bool {
        let _ = self.writer.send(Write::Close).await;
        let mut exited = self.exited;
        if let Ok(success) = tokio::time::timeout(grace, &mut exited).await {
            return success.unwrap_or(false);
        }
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
        exited.await.unwrap_or(false)
    }

    async fn command(&mut self, command: Value) -> Result<Value, CommandError> {
        let Pending(response) = self.submit(command).await?;
        response.await.unwrap_or(Err(CommandError::Closed))
    }

    async fn submit(&mut self, mut command: Value) -> Result<Pending, CommandError> {
        self.next_id += 1;
        command["id"] = Value::String(self.next_id.to_string());
        let (reply, response) = oneshot::channel();
        match self.state.lock().replies.as_mut() {
            Some(replies) => replies.insert(self.next_id, reply),
            None => return Err(CommandError::Closed),
        };
        self.writer
            .send(Write::Record(encode(&command)))
            .await
            .map_err(|_| CommandError::Closed)?;
        Ok(Pending(response))
    }

    async fn respond(&self, request: &RequestId, mut reply: Value) -> Result<(), CommandError> {
        reply["type"] = json!("extension_ui_response");
        reply["id"] = json!(request.0);
        self.writer
            .send(Write::Record(encode(&reply)))
            .await
            .map_err(|_| CommandError::Closed)
    }
}

/// Encodes one JSONL record.
fn encode(value: &Value) -> Vec<u8> {
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    line
}

/// Writes records to the agent until asked to close its input or the agent stops reading.
async fn write(mut stdin: ChildStdin, mut writes: mpsc::Receiver<Write>) {
    while let Some(Write::Record(line)) = writes.recv().await {
        if stdin.write_all(&line).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
    }
}

/// Reads the agent's output until it exits, fails or is killed, then reports the exit.
struct Supervisor {
    events: mpsc::Sender<SessionEvent>,
    writer: mpsc::Sender<Write>,
    state: State,
    adapter: PiAdapter,
    /// The session's monotonic clock for request timing.
    started: Instant,
    killed: oneshot::Receiver<()>,
    /// Set once `killed` completed; it must not be polled again.
    kill_requested: bool,
}

impl Supervisor {
    async fn run(mut self, mut child: Child, mut stdout: ChildStdout, exit: oneshot::Sender<bool>) {
        let mut framer = LineFramer::new(MAX_RECORD_BYTES);
        let mut buffer = vec![0; READ_BUFFER];
        let failure = 'read: loop {
            let Some(read) = self.unless_killed(stdout.read(&mut buffer)).await else {
                break None;
            };
            let frames = match read {
                Ok(0) | Err(_) => break None,
                Ok(length) => framer.push(&buffer[..length]),
            };
            for frame in frames {
                let record = match frame {
                    Frame::Record(record) => record,
                    Frame::Oversized { start } => {
                        if !self.skip(&start).await {
                            break 'read None;
                        }
                        continue;
                    }
                };
                let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(&record) else {
                    break 'read Some("agent wrote a record that is not a JSON object".to_owned());
                };
                if !self.handle(record).await {
                    break 'read None;
                }
            }
        };
        self.state.lock().replies = None;
        let stop = failure.is_some() || self.kill_requested;
        if let Some(reason) = failure {
            // A full event buffer must not keep a kill from ending the agent.
            let events = self.events.clone();
            let _ = self
                .unless_killed(events.send(SessionEvent::Failed(reason)))
                .await;
        }
        // An agent that closed its output may still run; a kill request ends the wait.
        let status = if stop {
            None
        } else {
            self.unless_killed(child.wait()).await
        };
        let status = match status {
            Some(status) => status,
            None => {
                let _ = child.start_kill();
                child.wait().await
            }
        };
        let success = status.is_ok_and(|status| status.success());
        // Shutdown learns of the exit first; the event waits for buffer space.
        let _ = exit.send(success);
        let _ = self.events.send(SessionEvent::Exited { success }).await;
    }

    /// Routes one record; returns `false` once the session was killed.
    async fn handle(&mut self, record: Map<String, Value>) -> bool {
        if record.get("type").and_then(Value::as_str) == Some("response") {
            self.answer_command(record);
            return true;
        }
        for event in self.adapter.translate(&record, self.started.elapsed()) {
            let forward = match &event {
                RuntimeEvent::RequestOpened {
                    request,
                    kind,
                    tool,
                    read_only,
                    ..
                } => {
                    let ask = {
                        let mut shared = self.state.lock();
                        let allowed = shared.allowed.contains(tool);
                        let ask = needs_user(shared.mode, *kind, *read_only, allowed);
                        if ask {
                            shared.approvals.insert(request.clone(), tool.clone());
                        }
                        ask
                    };
                    if !ask {
                        let reply = json!({
                            "type": "extension_ui_response",
                            "id": request.0,
                            "value": "allow-auto",
                        });
                        let _ = self.writer.send(Write::Record(encode(&reply))).await;
                    }
                    ask
                }
                RuntimeEvent::UserInputRequested { request, .. } => {
                    self.state.lock().questions.insert(request.clone());
                    true
                }
                RuntimeEvent::TurnStarted { .. } => {
                    self.state.lock().running = true;
                    true
                }
                RuntimeEvent::TurnCompleted { .. } | RuntimeEvent::TurnAborted { .. } => {
                    self.state.lock().running = false;
                    true
                }
                _ => true,
            };
            if forward {
                let events = self.events.clone();
                if self
                    .unless_killed(events.send(SessionEvent::Runtime(event)))
                    .await
                    .is_none()
                {
                    return false;
                }
            }
        }
        true
    }

    /// Fails the command an oversized response answers, or reports a skipped event; returns
    /// `false` once the session was killed.
    async fn skip(&mut self, start: &[u8]) -> bool {
        let start = String::from_utf8_lossy(start);
        if start.contains(r#""type":"response""#)
            && let Some(id) = start
                .split_once(r#""id":""#)
                .and_then(|(_, rest)| rest.split_once('"'))
                .and_then(|(id, _)| id.parse::<u64>().ok())
            && let Some(reply) = self
                .state
                .lock()
                .replies
                .as_mut()
                .and_then(|replies| replies.remove(&id))
        {
            let _ = reply.send(Err(CommandError::TooLarge));
            return true;
        }
        let skipped = RuntimeEvent::Notification {
            level: NotificationLevel::Warning,
            message: format!(
                "Skipped an agent record larger than {} MiB",
                MAX_RECORD_BYTES / (1024 * 1024)
            ),
        };
        let events = self.events.clone();
        self.unless_killed(events.send(SessionEvent::Runtime(skipped)))
            .await
            .is_some()
    }

    /// Resolves the command a response answers, moving its data rather than copying it.
    fn answer_command(&self, mut response: Map<String, Value>) {
        let id = response
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<u64>().ok());
        let reply = id.and_then(|id| self.state.lock().replies.as_mut()?.remove(&id));
        let Some(reply) = reply else {
            return;
        };
        let result = if response.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(response.remove("data").unwrap_or(Value::Null))
        } else {
            let error = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("command failed");
            Err(CommandError::Rejected(error.to_owned()))
        };
        let _ = reply.send(result);
    }

    /// Runs `work` unless a kill is requested first; then returns `None`.
    async fn unless_killed<F: Future>(&mut self, work: F) -> Option<F::Output> {
        if self.kill_requested {
            return None;
        }
        let mut work = pin!(work);
        let killed = &mut self.killed;
        let outcome = poll_fn(|context| {
            if let Poll::Ready(output) = work.as_mut().poll(context) {
                return Poll::Ready(Some(output));
            }
            Pin::new(&mut *killed).poll(context).map(|_| None)
        })
        .await;
        self.kill_requested = outcome.is_none();
        outcome
    }
}
