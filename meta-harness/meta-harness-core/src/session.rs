//! A running agent process behind the agent event model.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

use crate::approval::{Allowance, ApprovalMode};
use crate::pi::queue;
use crate::{AgentEvent, Answer, Decision, Delivery, History, PiAdapter, Queue, RequestId};
use supervisor::{Supervisor, read_end};

mod supervisor;

/// Largest record accepted from an agent; a larger one is skipped and fails its command.
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// Events buffered before the session stops reading agent output, which pauses the agent.
const EVENT_BUFFER: usize = 256;
/// Records buffered for the agent's input.
const WRITE_BUFFER: usize = 64;

/// How to start pi in RPC mode; the session adds pi's own arguments.
#[derive(Debug, Clone)]
pub struct AgentCommand {
    /// The executable.
    pub program: PathBuf,
    /// Arguments before the ones the session adds, such as an interpreter's script or more pi
    /// options.
    pub args: Vec<OsString>,
    /// Watershed's pi extension, which makes pi ask before tool calls; the session checks that
    /// pi loaded this file.
    pub extension: PathBuf,
    /// A session file pi reopens instead of starting a new session.
    pub session: Option<PathBuf>,
    /// Its working directory.
    pub cwd: PathBuf,
}

/// `path` with `.` and `..` resolved by name and repeated slashes dropped, as pi lists the
/// extension files it loaded; links stay as they are.
fn by_name(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => match resolved.components().next_back() {
                Some(Component::Normal(_)) => {
                    resolved.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => resolved.push(part),
            },
            part => resolved.push(part),
        }
    }
    resolved
}

impl AgentCommand {
    /// pi's arguments: RPC mode with Watershed's extension and the session to reopen.
    fn pi_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["--mode".into(), "rpc".into(), "-e".into()];
        args.push(by_name(&self.extension).into());
        if let Some(session) = &self.session {
            args.extend(["--session".into(), session.clone().into()]);
        }
        args
    }
}

/// Something that happened in a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// An event from the agent.
    Agent(AgentEvent),
    /// The agent broke its protocol, so the session stopped it.
    Failed(String),
    /// The agent process exited; this is always the last event.
    Exited {
        /// Whether the process exited successfully.
        success: bool,
        /// The end of what the agent wrote to its error output, such as why it did not start.
        error_output: String,
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
    /// Watershed's pi extension is not loaded, so tool calls would run without asking.
    Ungated,
}

type Reply = oneshot::Sender<Result<Value, CommandError>>;

/// A command the agent received but has not answered yet. Waiting for it needs no access to the
/// session, so the agent's questions can be answered meanwhile. Commands whose answer waits for
/// extension handlers, which may ask the user, or for the agent return one: prompts, abort,
/// session switches and model selection. pi answers every other command at once, since it
/// handles each input line on its own.
#[derive(Debug)]
pub struct Pending(oneshot::Receiver<Result<Value, CommandError>>);

impl Pending {
    /// Waits until the agent accepted or rejected the command.
    pub async fn accepted(self) -> Result<(), CommandError> {
        self.0.await.unwrap_or(Err(CommandError::Closed)).map(drop)
    }

    /// Waits until the agent accepted a prompt; `true` when the prompt started a run, `false`
    /// when the agent queued it for a run in progress or handled it itself, such as a command.
    pub async fn started(self) -> Result<bool, CommandError> {
        let data = self.0.await.unwrap_or(Err(CommandError::Closed))?;
        Ok(matches!(
            data.get("disposition").and_then(Value::as_str),
            Some("started") | None
        ))
    }

    /// Waits until the agent answered a session switch; `false` when an extension cancelled it.
    pub async fn switched(self) -> Result<bool, CommandError> {
        let data = self.0.await.unwrap_or(Err(CommandError::Closed))?;
        Ok(switched(&data))
    }
}

/// Whether a session switch's answer says it happened.
fn switched(data: &Value) -> bool {
    data.get("cancelled").and_then(Value::as_bool) != Some(true)
}

/// A model the agent can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// The provider, such as `anthropic`.
    pub provider: String,
    /// The provider's model id.
    pub id: String,
    /// The name to show; the id when the agent gives none.
    pub name: String,
}

/// What the agent reports about its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    /// The session file, unless the session is in memory only.
    pub session_file: Option<PathBuf>,
    /// The selected model as `provider/id`.
    pub model: Option<String>,
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
    /// What the user allowed for the rest of the session, and the session it belongs to.
    allowed: HashSet<Allowance>,
    allowed_in: Option<String>,
    /// Open approval requests, the allowance each one would use and its session.
    approvals: HashMap<RequestId, (Allowance, Option<String>)>,
    /// Open questions from agent extensions.
    questions: HashSet<RequestId>,
    /// The timers of questions with a time limit.
    expiries: HashMap<RequestId, tokio::task::AbortHandle>,
    /// Whether a turn is running, from the agent's events.
    running: bool,
    /// Commands that switch the session, awaiting their response.
    switches: HashSet<u64>,
    /// Whether the agent switched sessions since the history was read; the next read starts
    /// over, since sessions can share entry ids.
    switched: bool,
    /// Whether an abort waits for the turn to settle; dialogs arriving meanwhile are withdrawn
    /// or dismissed, since the agent stops only once they are answered.
    stopping: bool,
}

impl Shared {
    /// Ends an open question and its timer; `false` when it was not open.
    fn end_question(&mut self, request: &RequestId) -> bool {
        if let Some(expiry) = self.expiries.remove(request) {
            expiry.abort();
        }
        self.questions.remove(request)
    }

    /// Keeps allowances only for `session`: an extension command can switch sessions without
    /// the core seeing it, and a call from another session asks again.
    fn allow_in(&mut self, session: Option<String>) {
        if self.allowed_in != session {
            self.allowed.clear();
            self.allowed_in = session;
        }
    }
}

#[derive(Debug, Clone)]
struct State(Arc<Mutex<Shared>>);

impl State {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Stops an agent without its session, so a request the agent never answers cannot hold the
/// stop up; dropping it kills the process.
#[derive(Debug)]
pub struct Stopper {
    writer: mpsc::Sender<Write>,
    kill: Option<oneshot::Sender<()>>,
    exited: oneshot::Receiver<bool>,
}

impl Stopper {
    /// Asks the agent to exit and kills it after `grace`; returns whether it exited successfully.
    pub async fn stop(mut self, grace: Duration) -> bool {
        let mut exited = self.exited;
        // The grace period includes waiting to close the input, which an agent that stopped
        // reading never allows.
        let writer = self.writer.clone();
        let closed = async {
            let _ = writer.send(Write::Close).await;
            (&mut exited).await
        };
        if let Ok(success) = tokio::time::timeout(grace, closed).await {
            return success.unwrap_or(false);
        }
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
        exited.await.unwrap_or(false)
    }
}

/// A running agent process; dropping it, and its [`Stopper`] if taken, kills the process.
#[derive(Debug)]
pub struct Session {
    writer: mpsc::Sender<Write>,
    events: mpsc::Sender<SessionEvent>,
    /// Whether the supervisor sent `Exited`; resolutions sent later would follow it.
    ended: Arc<tokio::sync::Mutex<bool>>,
    state: State,
    next_id: u64,
    stopper: Option<Stopper>,
    /// The stored entries read so far; later reads continue after the last one.
    history: History,
    /// The directory the agent works in; a relative path it reports starts there.
    cwd: PathBuf,
    /// The extension file pi was told to load.
    extension: PathBuf,
}

impl Session {
    /// Starts the agent and returns the session with its event stream.
    pub fn start(command: &AgentCommand) -> std::io::Result<(Self, mpsc::Receiver<SessionEvent>)> {
        let mut child = crate::spawn(
            Command::new(&command.program)
                .args(&command.args)
                .args(command.pi_args())
                .current_dir(&command.cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true),
        )?;
        let stdout = child.stdout.take().expect("stdout is piped");
        // Drained all along, so a talkative agent never blocks on a full pipe.
        let error_output = Arc::new(Mutex::new(Vec::new()));
        let error_reader = tokio::spawn(read_end(
            child.stderr.take().expect("stderr is piped"),
            error_output.clone(),
        ));
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
        let ended = Arc::new(tokio::sync::Mutex::new(false));
        let supervisor = Supervisor {
            events: events.clone(),
            ended: ended.clone(),
            writer: writer.clone(),
            state: state.clone(),
            adapter: PiAdapter::new(),
            killed,
            kill_requested: false,
            error_output,
            error_reader,
        };
        tokio::spawn(supervisor.run(child, stdout, exit));
        let writer_for_stop = writer.clone();
        let session = Self {
            writer,
            events,
            ended,
            state,
            next_id: 0,
            stopper: Some(Stopper {
                writer: writer_for_stop,
                kill: Some(kill),
                exited,
            }),
            history: History::default(),
            cwd: command.cwd.clone(),
            extension: by_name(&command.extension),
        };
        Ok((session, receiver))
    }

    /// Sends a prompt that starts a turn.
    pub async fn prompt(&mut self, message: &str) -> Result<Pending, CommandError> {
        self.require_gate().await?;
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
        self.require_gate().await?;
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

    /// Aborts the running turn. pi answers once the agent is idle, which needs its open dialogs
    /// answered, so open approvals are withdrawn, which blocks their calls, and open questions
    /// dismissed.
    pub async fn abort(&mut self) -> Result<Pending, CommandError> {
        self.state.lock().stopping = true;
        let pending = self.submit(json!({ "type": "abort" })).await?;
        let (approvals, questions): (Vec<_>, Vec<_>) = {
            let shared = self.state.lock();
            (
                shared.approvals.keys().cloned().collect(),
                shared.questions.iter().cloned().collect(),
            )
        };
        // One the user answered meanwhile is already resolved.
        for request in approvals {
            let _ = self.withdraw(&request).await;
        }
        for request in questions {
            let _ = self.answer(&request, Answer::Cancelled).await;
        }
        Ok(pending)
    }

    /// Starts a new session in the same agent; the stored one stays available. Extensions may
    /// ask the user first or cancel it.
    pub async fn new_session(&mut self) -> Result<Pending, CommandError> {
        self.submit_as(json!({ "type": "new_session" }), true).await
    }

    /// Continues the session stored in `file`; extensions may ask the user first or cancel it.
    pub async fn switch_session(&mut self, file: &Path) -> Result<Pending, CommandError> {
        let command = json!({
            "type": "switch_session",
            "sessionPath": file.to_string_lossy(),
        });
        self.submit_as(command, true).await
    }

    /// Fails unless Watershed's pi extension is loaded from the file the session named, which
    /// pi shows by listing its `watershed-branch` command with that path: without it the agent
    /// would run tools without asking, so a session must not start, and every message that may
    /// start a run checks again: pi loads extensions anew for each session, and an extension can
    /// switch sessions unseen (ADR-0188).
    pub async fn require_gate(&mut self) -> Result<(), CommandError> {
        let data = self.command(json!({ "type": "get_commands" })).await?;
        let gated = data
            .get("commands")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|command| {
                command.get("name").and_then(Value::as_str) == Some("watershed-branch")
                    && command.get("source").and_then(Value::as_str) == Some("extension")
                    && command
                        .pointer("/sourceInfo/path")
                        .and_then(Value::as_str)
                        .is_some_and(|path| Path::new(path) == self.extension)
            });
        if gated {
            Ok(())
        } else {
            Err(CommandError::Ungated)
        }
    }

    /// The models the agent can use.
    pub async fn models(&mut self) -> Result<Vec<Model>, CommandError> {
        let data = self
            .command(json!({ "type": "get_available_models" }))
            .await?;
        let models = data.get("models").and_then(Value::as_array);
        Ok(models
            .into_iter()
            .flatten()
            .filter_map(|model| {
                let text = |field| model.get(field).and_then(Value::as_str);
                let id = text("id")?;
                Some(Model {
                    provider: text("provider")?.to_owned(),
                    id: id.to_owned(),
                    name: text("name").unwrap_or(id).to_owned(),
                })
            })
            .collect())
    }

    /// Selects the model the next requests use; pi runs extensions' handlers first.
    pub async fn set_model(&mut self, provider: &str, id: &str) -> Result<Pending, CommandError> {
        self.submit(json!({ "type": "set_model", "provider": provider, "modelId": id }))
            .await
    }

    /// Reads the session's file and model.
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
            session_file: text("sessionFile").map(|file| self.cwd.join(file)),
            model,
        })
    }

    /// Reads the session's stored entries, including abandoned branches. The first read takes
    /// them from pi's session file line by line, so no single answer has to hold the whole
    /// session; later reads ask the agent only for entries added since. When the agent no
    /// longer knows the last one, everything is read again.
    pub async fn history(&mut self) -> Result<History, CommandError> {
        if std::mem::take(&mut self.state.lock().switched) {
            self.history = History::default();
        }
        if self.history.last_id().is_none() {
            self.read_stored().await?;
        }
        match self.read_newer().await {
            // The agent no longer knows the last entry read, or what it added since does not fit
            // one answer: the session file holds them.
            Err(CommandError::Rejected(_) | CommandError::TooLarge)
                if self.history.last_id().is_some() =>
            {
                self.history = History::default();
                self.read_stored().await?;
                self.read_newer().await?;
            }
            result => result?,
        }
        Ok(self.history.clone())
    }

    /// Reads the entries pi stored in its session file; without a file yet, the agent's answer
    /// holds them all.
    async fn read_stored(&mut self) -> Result<(), CommandError> {
        let Some(file) = self.state().await?.session_file else {
            return Ok(());
        };
        let stored =
            tokio::task::spawn_blocking(move || History::from_file(&file, MAX_RECORD_BYTES)).await;
        if let Ok(Ok(stored)) = stored {
            self.history = stored;
        }
        Ok(())
    }

    /// Asks the agent for the entries after the last one read and for its current leaf.
    async fn read_newer(&mut self) -> Result<(), CommandError> {
        let mut request = json!({ "type": "get_entries" });
        if let Some(since) = self.history.last_id() {
            request["since"] = json!(since);
        }
        let data = self.command(request).await?;
        self.history.append(&data);
        Ok(())
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

    /// Answers an open approval request. Like every command, it runs to completion: dropped
    /// midway, the request may stay unanswered.
    pub async fn decide(
        &mut self,
        request: &RequestId,
        decision: Decision,
    ) -> Result<(), CommandError> {
        {
            let mut shared = self.state.lock();
            let (allowance, session) = shared
                .approvals
                .remove(request)
                .ok_or(CommandError::UnknownRequest)?;
            if decision == Decision::AllowSession {
                shared.allow_in(session);
                shared.allowed.insert(allowance);
            }
        }
        let value = match decision {
            Decision::AllowOnce => "allow-once",
            Decision::AllowSession => "allow-session",
            Decision::Reject => "reject",
        };
        self.respond(request, json!({ "value": value })).await?;
        self.resolved(AgentEvent::RequestResolved {
            request: request.clone(),
        })
        .await;
        Ok(())
    }

    /// Ends an open approval request without a decision of the user's; its call is blocked.
    async fn withdraw(&mut self, request: &RequestId) -> Result<(), CommandError> {
        if self.state.lock().approvals.remove(request).is_none() {
            return Err(CommandError::UnknownRequest);
        }
        self.respond(request, json!({ "cancelled": true })).await?;
        self.resolved(AgentEvent::RequestResolved {
            request: request.clone(),
        })
        .await;
        Ok(())
    }

    /// Answers an open question from an agent extension.
    pub async fn answer(
        &mut self,
        request: &RequestId,
        answer: Answer,
    ) -> Result<(), CommandError> {
        if !self.state.lock().end_question(request) {
            return Err(CommandError::UnknownRequest);
        }
        let reply = match answer {
            Answer::Value(value) => json!({ "value": value }),
            Answer::Confirmed(confirmed) => json!({ "confirmed": confirmed }),
            Answer::Cancelled => json!({ "cancelled": true }),
        };
        self.respond(request, reply).await?;
        self.resolved(AgentEvent::UserInputResolved {
            request: request.clone(),
        })
        .await;
        Ok(())
    }

    /// Reports a resolved dialog unless the agent's exit was reported, which stays last.
    async fn resolved(&self, event: AgentEvent) {
        let ended = self.ended.lock().await;
        if !*ended {
            let _ = self.events.send(SessionEvent::Agent(event)).await;
        }
    }

    /// Asks the agent to exit and kills it after `grace`; returns whether it exited successfully.
    pub async fn shutdown(mut self, grace: Duration) -> bool {
        match self.stopper.take() {
            Some(stopper) => stopper.stop(grace).await,
            None => false,
        }
    }

    /// Hands out what stops the agent, for an owner that must stop it while the session waits
    /// for an answer; [`Session::shutdown`] then stops nothing.
    pub fn stopper(&mut self) -> Option<Stopper> {
        self.stopper.take()
    }

    async fn command(&mut self, command: Value) -> Result<Value, CommandError> {
        let Pending(response) = self.submit(command).await?;
        response.await.unwrap_or(Err(CommandError::Closed))
    }

    async fn submit(&mut self, command: Value) -> Result<Pending, CommandError> {
        self.submit_as(command, false).await
    }

    /// Sends `command`; a `switch` of the session ends its allowances once the agent did it.
    async fn submit_as(
        &mut self,
        mut command: Value,
        switch: bool,
    ) -> Result<Pending, CommandError> {
        self.next_id += 1;
        command["id"] = Value::String(self.next_id.to_string());
        let (reply, response) = oneshot::channel();
        {
            let mut shared = self.state.lock();
            let Some(replies) = shared.replies.as_mut() else {
                return Err(CommandError::Closed);
            };
            replies.insert(self.next_id, reply);
            if switch {
                shared.switches.insert(self.next_id);
            }
        }
        self.writer
            .send(Write::Record(encode(&command)))
            .await
            .map_err(|_| CommandError::Closed)?;
        Ok(Pending(response))
    }

    async fn respond(&self, request: &RequestId, answer: Value) -> Result<(), CommandError> {
        self.writer
            .send(ui_response(request, answer))
            .await
            .map_err(|_| CommandError::Closed)
    }
}

/// The answer to an agent dialog.
fn ui_response(request: &RequestId, mut answer: Value) -> Write {
    answer["type"] = json!("extension_ui_response");
    answer["id"] = json!(request.0);
    Write::Record(encode(&answer))
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
