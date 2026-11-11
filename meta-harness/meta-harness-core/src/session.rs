//! A running agent process behind the agent event model.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

use crate::pi::queue;
use crate::{
    AgentEvent, Answer, Checkpoint, Checkpoints, Delivery, History, Phase, PiAdapter, Queue,
    RequestId, Whitespace,
};
use supervisor::{Supervisor, read_end};

mod supervisor;

/// Largest record accepted from an agent; a larger one is skipped and fails its command.
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// Events buffered before the session stops reading agent output, which pauses the agent.
const EVENT_BUFFER: usize = 256;
/// Records buffered for the agent's input.
const WRITE_BUFFER: usize = 64;
/// Attempts at reading the history while the session keeps changing before a read gives up.
const HISTORY_READS: usize = 3;

/// Checkpoint requests waiting for their capture; pi waits for each, so few ever wait.
const CAPTURE_BUFFER: usize = 8;

/// How to start pi in RPC mode; the session adds pi's own arguments.
#[derive(Debug, Clone)]
pub struct AgentCommand {
    /// The executable.
    pub program: PathBuf,
    /// Arguments before the ones the session adds, such as an interpreter's script or more pi
    /// options.
    pub args: Vec<OsString>,
    /// Watershed's pi extension, which marks each turn's checkpoints; the session checks that pi
    /// loaded this file. It must be absolute: pi would read a relative path inside the
    /// project, which could supply its own copy.
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
    /// Watershed's pi extension is not loaded, so a turn would run without its checkpoints.
    ExtensionMissing,
    /// The session kept changing while its history was read, so no read matched one session.
    SessionChanged,
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
    /// Tokens the model can take in one request, when the agent knows them.
    pub context_window: Option<u64>,
}

/// What the agent reports about its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    /// The session file, unless the session is in memory only.
    pub session_file: Option<PathBuf>,
    /// The selected model as `provider/id`.
    pub model: Option<String>,
    /// pi's id of the session, which tells sessions apart even when they share entry ids.
    pub session_id: Option<String>,
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
    /// Whether an abort waits for the turn to settle; questions arriving meanwhile are dismissed,
    /// since the agent stops only once they are answered.
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
    /// `true` kills the agent, as dropping it does; `false` says it exited, so its output is
    /// delivered in full.
    kill: Option<oneshot::Sender<bool>>,
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
            if let Some(kill) = self.kill.take() {
                let _ = kill.send(false);
            }
            return success.unwrap_or(false);
        }
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(true);
        }
        exited.await.unwrap_or(false)
    }
}

/// Answers the agent's questions and aborts its turn without its session, so a request the
/// agent has not answered yet cannot hold them up.
#[derive(Debug, Clone)]
pub struct Responder {
    writer: mpsc::Sender<Write>,
    /// Weak, so a responder kept after the session keeps no event stream open.
    events: mpsc::WeakSender<SessionEvent>,
    /// Whether the supervisor sent `Exited`; resolutions sent later would follow it.
    ended: Arc<tokio::sync::Mutex<bool>>,
    state: State,
    next_id: Arc<AtomicU64>,
}

impl Responder {
    /// Aborts the running turn. pi answers once the agent is idle, which needs its open
    /// questions answered, so they are dismissed.
    pub async fn abort(&self) -> Result<Pending, CommandError> {
        let stopping = |shared: &mut Shared, _| shared.stopping = true;
        let pending = self.submit_as(json!({ "type": "abort" }), stopping).await?;
        let questions: Vec<_> = self.state.lock().questions.iter().cloned().collect();
        // One the user answered meanwhile is already resolved.
        for request in questions {
            let _ = self.answer(&request, Answer::Cancelled).await;
        }
        Ok(pending)
    }

    /// Answers an open question from an agent extension.
    pub async fn answer(&self, request: &RequestId, answer: Answer) -> Result<(), CommandError> {
        if !self.state.lock().questions.contains(request) {
            return Err(CommandError::UnknownRequest);
        }
        // Room first, so an answer given up while it waits leaves the question open.
        let room = self
            .writer
            .reserve()
            .await
            .map_err(|_| CommandError::Closed)?;
        // It may have been answered or run out of time meanwhile.
        if !self.state.lock().end_question(request) {
            return Err(CommandError::UnknownRequest);
        }
        let reply = match answer {
            Answer::Value(value) => json!({ "value": value }),
            Answer::Confirmed(confirmed) => json!({ "confirmed": confirmed }),
            Answer::Cancelled => json!({ "cancelled": true }),
        };
        room.send(ui_response(request, reply));
        self.resolved(AgentEvent::UserInputResolved {
            request: request.clone(),
        })
        .await;
        Ok(())
    }

    /// Reports a resolved dialog unless the agent's exit was reported, which stays last.
    async fn resolved(&self, event: AgentEvent) {
        let ended = self.ended.lock().await;
        if !*ended && let Some(events) = self.events.upgrade() {
            let _ = events.send(SessionEvent::Agent(event)).await;
        }
    }

    /// Sends `command` once the agent's input has room, so a caller that gives up while it
    /// waits leaves nothing waiting for an answer; `mark` notes what the command starts under
    /// its id, such as a session switch.
    async fn submit_as(
        &self,
        mut command: Value,
        mark: impl FnOnce(&mut Shared, u64),
    ) -> Result<Pending, CommandError> {
        let room = self
            .writer
            .reserve()
            .await
            .map_err(|_| CommandError::Closed)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        command["id"] = Value::String(id.to_string());
        let (reply, response) = oneshot::channel();
        {
            let mut shared = self.state.lock();
            let Some(replies) = shared.replies.as_mut() else {
                return Err(CommandError::Closed);
            };
            replies.insert(id, reply);
            mark(&mut shared, id);
        }
        room.send(Write::Record(encode(&command)));
        Ok(Pending(response))
    }
}

/// A running agent process; dropping it, and its [`Stopper`] if taken, kills the process.
#[derive(Debug)]
pub struct Session {
    responder: Responder,
    /// Keeps the event stream open while the session lives.
    _events: mpsc::Sender<SessionEvent>,
    stopper: Option<Stopper>,
    /// The stored entries read so far; later reads continue after the last one.
    history: History,
    /// The session the entries in `history` belong to.
    history_session: Option<String>,
    /// The directory the agent works in; a relative path it reports starts there.
    cwd: PathBuf,
    /// The extension file pi was told to load.
    extension: PathBuf,
    /// The project's checkpoints at each turn's start and end.
    checkpoints: Arc<Checkpoints>,
}

impl Session {
    /// Starts the agent and returns the session with its event stream; a relative extension
    /// path is refused before the agent starts.
    pub fn start(command: &AgentCommand) -> std::io::Result<(Self, mpsc::Receiver<SessionEvent>)> {
        if !command.extension.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "the extension path must be absolute: {}",
                    command.extension.display()
                ),
            ));
        }
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
        let input_writer = tokio::spawn(write(stdin, writes));
        let checkpoints = Arc::new(Checkpoints::new(&command.cwd));
        let (captures, requests) = mpsc::channel(CAPTURE_BUFFER);
        tokio::spawn(capture(checkpoints.clone(), writer.clone(), requests));
        let ended = Arc::new(tokio::sync::Mutex::new(false));
        tokio::spawn(report_waiting(
            checkpoints.waiting(),
            events.downgrade(),
            ended.clone(),
        ));
        let supervisor = Supervisor {
            events: events.clone(),
            ended: ended.clone(),
            writer: writer.clone(),
            state: state.clone(),
            adapter: PiAdapter::new(),
            killed: Some(killed),
            kill_requested: false,
            error_output,
            error_reader,
            input_writer,
            captures,
            checkpoints: checkpoints.clone(),
        };
        tokio::spawn(supervisor.run(child, stdout, exit));
        let writer_for_stop = writer.clone();
        let session = Self {
            responder: Responder {
                writer,
                events: events.downgrade(),
                ended,
                state,
                next_id: Arc::new(AtomicU64::new(0)),
            },
            _events: events,
            stopper: Some(Stopper {
                writer: writer_for_stop,
                kill: Some(kill),
                exited,
            }),
            history: History::default(),
            history_session: None,
            cwd: command.cwd.clone(),
            extension: by_name(&command.extension),
            checkpoints,
        };
        Ok((session, receiver))
    }

    /// The project's checkpoints, which the Diff tab compares; reading them never holds the
    /// session.
    pub fn checkpoints(&self) -> Arc<Checkpoints> {
        self.checkpoints.clone()
    }

    /// Sends a prompt that starts a turn.
    pub async fn prompt(&mut self, message: &str) -> Result<Pending, CommandError> {
        self.require_extension().await?;
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
        self.require_extension().await?;
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

    /// Aborts the running turn, as [`Responder::abort`] does.
    pub async fn abort(&mut self) -> Result<Pending, CommandError> {
        self.responder.abort().await
    }

    /// Starts a new session in the same agent; the stored one stays available. Extensions may
    /// ask the user first or cancel it.
    pub async fn new_session(&mut self) -> Result<Pending, CommandError> {
        self.switch(json!({ "type": "new_session" })).await
    }

    /// Continues the session stored in `file`; extensions may ask the user first or cancel it.
    pub async fn switch_session(&mut self, file: &Path) -> Result<Pending, CommandError> {
        let command = json!({
            "type": "switch_session",
            "sessionPath": file.to_string_lossy(),
        });
        self.switch(command).await
    }

    /// Fails unless Watershed's pi extension is loaded from the file the session named, which
    /// pi shows by listing its `watershed-branch` command with that path: without it a turn
    /// would run without its checkpoints, so a session must not start, and every message that
    /// may start a run checks again: pi loads extensions anew for each session, and an extension
    /// can switch sessions unseen (ADR-0234).
    pub async fn require_extension(&mut self) -> Result<(), CommandError> {
        let data = self.command(json!({ "type": "get_commands" })).await?;
        let loaded = data
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
        if loaded {
            Ok(())
        } else {
            Err(CommandError::ExtensionMissing)
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
                    context_window: model
                        .get("contextWindow")
                        .and_then(Value::as_u64)
                        .filter(|tokens| *tokens > 0),
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
            session_id: text("sessionId").map(str::to_owned),
        })
    }

    /// Reads the session's stored entries, including abandoned branches. The first read takes
    /// them from pi's session file line by line, so no single answer has to hold the whole
    /// session; later reads ask the agent only for entries added since. When the agent no
    /// longer knows the last one, everything is read again. A read starts over unless the
    /// session is the same before and after it: sessions can share entry ids, and a switch can
    /// land during a read. The entries stay with the session, so callers read them while they
    /// hold it.
    pub async fn history(&mut self) -> Result<&History, CommandError> {
        if std::mem::take(&mut self.responder.state.lock().switched) {
            self.history = History::default();
        }
        for _ in 0..HISTORY_READS {
            let before = self.state().await?;
            if before.session_id != self.history_session {
                self.history = History::default();
                self.history_session = before.session_id.clone();
            }
            self.read_entries(before.session_file.as_deref()).await?;
            if self.state().await?.session_id == before.session_id {
                return Ok(&self.history);
            }
            self.history = History::default();
        }
        Err(CommandError::SessionChanged)
    }

    /// Adds the entries stored since the last one read, from `file` when nothing was read yet.
    async fn read_entries(&mut self, file: Option<&Path>) -> Result<(), CommandError> {
        if self.history.last_id().is_none() {
            self.read_stored(file).await;
        }
        match self.read_newer().await {
            // The agent no longer knows the last entry read, or what it added since does not fit
            // one answer: the session file holds them.
            Err(CommandError::Rejected(_) | CommandError::TooLarge)
                if self.history.last_id().is_some() =>
            {
                self.history = History::default();
                self.read_stored(file).await;
                self.read_newer().await
            }
            result => result,
        }
    }

    /// Reads the entries pi stored in its session file; without a file yet, the agent's answer
    /// holds them all.
    async fn read_stored(&mut self, file: Option<&Path>) {
        let Some(file) = file.map(Path::to_owned) else {
            return;
        };
        let stored =
            tokio::task::spawn_blocking(move || History::from_file(&file, MAX_RECORD_BYTES)).await;
        if let Ok(Ok(stored)) = stored {
            self.history = stored;
        }
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
        self.responder.state.lock().running
    }

    /// Answers an open question from an agent extension.
    pub async fn answer(
        &mut self,
        request: &RequestId,
        answer: Answer,
    ) -> Result<(), CommandError> {
        self.responder.answer(request, answer).await
    }

    /// Hands out what answers the agent's questions and aborts its turn while the session
    /// waits for an answer.
    pub fn responder(&self) -> Responder {
        self.responder.clone()
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
        self.responder.submit_as(command, |_, _| {}).await
    }

    /// Sends a command that switches the session; once the agent did it, the next history read
    /// starts over.
    async fn switch(&mut self, command: Value) -> Result<Pending, CommandError> {
        let switch = |shared: &mut Shared, id| {
            shared.switches.insert(id);
        };
        self.responder.submit_as(command, switch).await
    }
}

/// Takes the checkpoints the agent asks for, in order, and tells it whether each was taken; a
/// capture in a large project never holds up the agent's other records.
async fn capture(
    checkpoints: Arc<Checkpoints>,
    writer: mpsc::Sender<Write>,
    mut requests: mpsc::Receiver<supervisor::CheckpointRequest>,
) {
    while let Some((request, checkpoint)) = requests.recv().await {
        let outcome = match checkpoint {
            Ok(checkpoint) => match checkpoints.capture(&checkpoint).await {
                Ok(()) => taken(&checkpoints, &checkpoint).await,
                Err(error) => json!({
                    "error": error.to_string(),
                    "canRun": matches!(error, crate::DiffError::NotARepository)
                        || checkpoints.guards(&checkpoint),
                }),
            },
            Err(reason) => json!({ "error": reason, "canRun": false }),
        };
        let answer = json!({ "value": outcome.to_string() });
        if writer.send(ui_response(&request, answer)).await.is_err() {
            break;
        }
    }
}

/// Shows in the status while a turn's start waits for a revert or another Git action that holds
/// the checkout alone, until the agent's exit was reported, which stays last. It holds the
/// events weakly, so checkpoints kept after the session keep no event stream open.
async fn report_waiting(
    mut waiting: tokio::sync::watch::Receiver<bool>,
    events: mpsc::WeakSender<SessionEvent>,
    ended: Arc<tokio::sync::Mutex<bool>>,
) {
    while waiting.changed().await.is_ok() {
        let Some(events) = events.upgrade() else {
            break;
        };
        let text = (*waiting.borrow_and_update())
            .then(|| "Waiting for another Git action in this checkout to finish".to_owned());
        let status = AgentEvent::StatusChanged {
            key: "watershed.revert".to_owned(),
            text,
        };
        let ended = ended.lock().await;
        if *ended || events.send(SessionEvent::Agent(status)).await.is_err() {
            break;
        }
    }
}

/// The answer to a checkpoint that was taken; a turn's end also counts what the turn changed,
/// so listing turns needs no Git.
async fn taken(checkpoints: &Checkpoints, checkpoint: &Checkpoint) -> Value {
    if checkpoint.phase == Phase::Start {
        return json!({ "ok": true, "canRun": true });
    }
    let start = Checkpoint {
        phase: Phase::Start,
        ..checkpoint.clone()
    };
    match checkpoints
        .changes(&start, checkpoint, Whitespace::Shown)
        .await
    {
        Ok(changes) => json!({
            "ok": true,
            "files": changes.len(),
            "added": changes.iter().map(|change| change.added).sum::<u64>(),
            "removed": changes.iter().map(|change| change.removed).sum::<u64>(),
        }),
        Err(_) => json!({ "ok": true }),
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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Waker};

    use super::*;

    /// A responder whose agent input has no room left, with the question `q1` open.
    fn crowded() -> (Responder, mpsc::Receiver<Write>) {
        let (writer, writes) = mpsc::channel(1);
        assert!(writer.try_send(Write::Close).is_ok());
        let state = State(Arc::new(Mutex::new(Shared {
            replies: Some(HashMap::new()),
            questions: HashSet::from([RequestId("q1".to_owned())]),
            ..Shared::default()
        })));
        let responder = Responder {
            writer,
            events: mpsc::channel(1).0.downgrade(),
            ended: Arc::default(),
            state,
            next_id: Arc::default(),
        };
        (responder, writes)
    }

    /// Starts `work` and drops it while it waits for room, as a caller that gives up does.
    fn cancel(work: impl Future) {
        let mut work = std::pin::pin!(work);
        let polled = work.as_mut().poll(&mut Context::from_waker(Waker::noop()));
        assert!(polled.is_pending(), "it waits for room");
    }

    #[test]
    fn a_cancelled_answer_leaves_the_question_open() {
        let (responder, mut writes) = crowded();
        let request = RequestId("q1".to_owned());
        cancel(responder.answer(&request, Answer::Confirmed(true)));
        assert!(matches!(writes.try_recv(), Ok(Write::Close)));
        let answered = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime starts")
            .block_on(responder.answer(&request, Answer::Confirmed(true)));
        assert_eq!(answered, Ok(()), "the question is still open");
        assert!(matches!(writes.try_recv(), Ok(Write::Record(_))));
    }

    #[test]
    fn a_cancelled_abort_leaves_nothing_waiting_and_questions_to_the_user() {
        let (responder, _writes) = crowded();
        cancel(responder.abort());
        let shared = responder.state.lock();
        assert!(shared.replies.as_ref().is_some_and(HashMap::is_empty));
        assert!(!shared.stopping);
    }

    #[test]
    fn an_answered_or_expired_question_keeps_no_timer() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime starts");
        runtime.block_on(async {
            let state = State(Arc::new(Mutex::new(Shared {
                replies: Some(HashMap::new()),
                ..Shared::default()
            })));
            let (events, mut received) = mpsc::channel(4);
            let supervisor = Supervisor {
                events,
                ended: Arc::default(),
                writer: mpsc::channel(1).0,
                state: state.clone(),
                adapter: crate::PiAdapter::new(),
                killed: None,
                kill_requested: false,
                error_output: Arc::default(),
                error_reader: tokio::spawn(async {}),
                input_writer: tokio::spawn(async {}),
                captures: mpsc::channel(1).0,
                checkpoints: Arc::new(Checkpoints::new(Path::new("."))),
            };
            // A question the user answered before its timer started keeps no timer.
            let answered = RequestId("answered".to_owned());
            supervisor.expire(
                answered,
                tokio::time::Instant::now() + Duration::from_secs(60),
            );
            assert!(
                state.lock().expiries.is_empty(),
                "an answered question keeps no timer"
            );
            // A question whose time is up is resolved, and its timer ends with nothing kept.
            let late = RequestId("late".to_owned());
            state.lock().questions.insert(late.clone());
            supervisor.expire(late.clone(), tokio::time::Instant::now());
            let resolved = tokio::time::timeout(Duration::from_secs(10), received.recv())
                .await
                .expect("the timer resolves the question")
                .expect("the session is open");
            assert_eq!(
                resolved,
                SessionEvent::Agent(AgentEvent::UserInputResolved { request: late })
            );
            assert!(
                state.lock().expiries.is_empty(),
                "an expired question keeps no timer"
            );
        });
    }
}
