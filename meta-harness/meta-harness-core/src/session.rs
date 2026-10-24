//! A running agent process behind the runtime event model.

use std::collections::HashMap;
use std::ffi::OsString;
use std::future::{Future, poll_fn};
use std::path::PathBuf;
use std::pin::pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

use crate::{LineFramer, PiAdapter, RuntimeEvent};

/// Largest record accepted from an agent; a larger record ends the session.
const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// Events buffered before the session stops reading agent output, which pauses the agent.
const EVENT_BUFFER: usize = 256;
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
    /// The agent process is gone.
    Closed,
}

type Reply = oneshot::Sender<Result<(), CommandError>>;

/// Commands awaiting their response; closed once the agent is gone.
#[derive(Debug, Clone, Default)]
struct Pending(Arc<Mutex<Option<HashMap<u64, Reply>>>>);

impl Pending {
    fn open() -> Self {
        Self(Arc::new(Mutex::new(Some(HashMap::new()))))
    }

    fn insert(&self, id: u64, reply: Reply) -> bool {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        guard
            .as_mut()
            .map(|replies| replies.insert(id, reply))
            .is_some()
    }

    fn take(&self, id: u64) -> Option<Reply> {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        guard.as_mut().and_then(|replies| replies.remove(&id))
    }

    /// Fails every waiting command by dropping its reply.
    fn close(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
}

/// A running agent process; dropping it kills the process.
#[derive(Debug)]
pub struct Session {
    stdin: Option<ChildStdin>,
    pending: Pending,
    next_id: u64,
    kill: Option<oneshot::Sender<()>>,
    exited: oneshot::Receiver<bool>,
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
        let stdin = child.stdin.take();
        let (events, receiver) = mpsc::channel(EVENT_BUFFER);
        let (kill, killed) = oneshot::channel();
        let (exit, exited) = oneshot::channel();
        let pending = Pending::open();
        tokio::spawn(supervise(
            child,
            stdout,
            events,
            pending.clone(),
            killed,
            exit,
        ));
        let session = Self {
            stdin,
            pending,
            next_id: 0,
            kill: Some(kill),
            exited,
        };
        Ok((session, receiver))
    }

    /// Sends a prompt that starts a turn.
    pub async fn prompt(&mut self, message: &str) -> Result<(), CommandError> {
        self.send(json!({ "type": "prompt", "message": message }))
            .await
    }

    /// Steers the running turn with a further message.
    pub async fn steer(&mut self, message: &str) -> Result<(), CommandError> {
        self.send(json!({ "type": "steer", "message": message }))
            .await
    }

    /// Aborts the running turn.
    pub async fn abort(&mut self) -> Result<(), CommandError> {
        self.send(json!({ "type": "abort" })).await
    }

    /// Asks the agent to exit and kills it after `grace`; returns whether it exited successfully.
    pub async fn shutdown(mut self, grace: Duration) -> bool {
        drop(self.stdin.take());
        let mut exited = self.exited;
        if let Ok(success) = tokio::time::timeout(grace, &mut exited).await {
            return success.unwrap_or(false);
        }
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
        exited.await.unwrap_or(false)
    }

    async fn send(&mut self, mut command: Value) -> Result<(), CommandError> {
        self.next_id += 1;
        command["id"] = Value::String(self.next_id.to_string());
        let (reply, response) = oneshot::channel();
        if !self.pending.insert(self.next_id, reply) {
            return Err(CommandError::Closed);
        }
        let mut line = command.to_string().into_bytes();
        line.push(b'\n');
        let stdin = self.stdin.as_mut().ok_or(CommandError::Closed)?;
        if stdin.write_all(&line).await.is_err() || stdin.flush().await.is_err() {
            return Err(CommandError::Closed);
        }
        response.await.unwrap_or(Err(CommandError::Closed))
    }
}

/// Reads the agent's output until it exits, fails or is killed, then reports the exit.
async fn supervise(
    mut child: Child,
    mut stdout: ChildStdout,
    events: mpsc::Sender<SessionEvent>,
    pending: Pending,
    mut killed: oneshot::Receiver<()>,
    exit: oneshot::Sender<bool>,
) {
    let mut framer = LineFramer::new(MAX_RECORD_BYTES);
    let mut adapter = PiAdapter::new();
    let mut buffer = vec![0; READ_BUFFER];
    let failure = loop {
        let read = match first(stdout.read(&mut buffer), &mut killed).await {
            Either::First(read) => read,
            Either::Second(_) => break None,
        };
        let records = match read {
            Ok(0) | Err(_) => break None,
            Ok(length) => match framer.push(&buffer[..length]) {
                Ok(records) => records,
                Err(error) => break Some(format!("agent output broke framing: {error}")),
            },
        };
        let mut failure = None;
        for record in records {
            let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(&record) else {
                failure = Some("agent wrote a record that is not a JSON object".to_owned());
                break;
            };
            if record.get("type").and_then(Value::as_str) == Some("response") {
                answer(&pending, &record);
                continue;
            }
            for event in adapter.translate(&record) {
                let sent = events.send(SessionEvent::Runtime(event));
                if let Either::Second(_) = first(sent, &mut killed).await {
                    break;
                }
            }
        }
        if failure.is_some() {
            break failure;
        }
    };
    pending.close();
    let stop = failure.is_some() || killed.is_terminated();
    if let Some(reason) = failure {
        let _ = events.send(SessionEvent::Failed(reason)).await;
    }
    // An agent that closed its output may still run; a kill request ends the wait.
    let status = if stop {
        None
    } else {
        match first(child.wait(), &mut killed).await {
            Either::First(status) => Some(status),
            Either::Second(_) => None,
        }
    };
    let status = match status {
        Some(status) => status,
        None => {
            let _ = child.start_kill();
            child.wait().await
        }
    };
    let success = status.is_ok_and(|status| status.success());
    let _ = events.send(SessionEvent::Exited { success }).await;
    let _ = exit.send(success);
}

enum Either<A, B> {
    First(A),
    Second(B),
}

/// Waits for whichever future completes first, preferring `a`.
async fn first<A: Future, B: Future + Unpin>(a: A, b: &mut B) -> Either<A::Output, B::Output> {
    let mut a = pin!(a);
    poll_fn(|context| {
        if let Poll::Ready(output) = a.as_mut().poll(context) {
            return Poll::Ready(Either::First(output));
        }
        match std::pin::Pin::new(&mut *b).poll(context) {
            Poll::Ready(output) => Poll::Ready(Either::Second(output)),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

/// Resolves the command a response answers.
fn answer(pending: &Pending, response: &Map<String, Value>) {
    let id = response
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.parse().ok());
    let Some(reply) = id.and_then(|id| pending.take(id)) else {
        return;
    };
    let result = if response.get("success").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("command failed");
        Err(CommandError::Rejected(error.to_owned()))
    };
    let _ = reply.send(result);
}
