//! Routes an agent's records to the session: answers, approvals it may decide itself and
//! events, until the agent exits.

use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdout};
use tokio::sync::{mpsc, oneshot};

use super::{CommandError, MAX_RECORD_BYTES, SessionEvent, State, Write, switched, ui_response};
use crate::approval::needs_user;
use crate::{AgentEvent, Frame, LineFramer, NotificationLevel, PiAdapter, RequestId};

const READ_BUFFER: usize = 64 * 1024;

/// The end of the agent's error output kept to explain why it failed.
const MAX_ERROR_OUTPUT: usize = 16 * 1024;

/// How long an exited agent's error output may take to end; a process the agent started can
/// keep it open.
const ERROR_OUTPUT_WAIT: Duration = Duration::from_millis(500);

/// Reads the agent's output until it exits, fails or is killed, then reports the exit.
pub(super) struct Supervisor {
    pub(super) events: mpsc::Sender<SessionEvent>,
    pub(super) ended: Arc<tokio::sync::Mutex<bool>>,
    pub(super) writer: mpsc::Sender<Write>,
    pub(super) state: State,
    pub(super) adapter: PiAdapter,
    /// The session's monotonic clock for request timing.
    pub(super) started: Instant,
    pub(super) killed: oneshot::Receiver<()>,
    /// Set once `killed` completed; it must not be polled again.
    pub(super) kill_requested: bool,
    /// The end of the agent's error output so far, and the task that reads it.
    pub(super) error_output: Arc<Mutex<Vec<u8>>>,
    pub(super) error_reader: tokio::task::JoinHandle<()>,
}

impl Supervisor {
    pub(super) async fn run(
        mut self,
        mut child: Child,
        mut stdout: ChildStdout,
        exit: oneshot::Sender<bool>,
    ) {
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
                    Frame::Oversized { start, .. } => {
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
        {
            // Nothing is asked of a gone agent: open dialogs end with it, and their expiry
            // reports nothing after the exit.
            let mut shared = self.state.lock();
            shared.replies = None;
            shared.questions.clear();
            for (_, expiry) in shared.expiries.drain() {
                expiry.abort();
            }
            shared.approvals.clear();
        }
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
        let _ = tokio::time::timeout(ERROR_OUTPUT_WAIT, &mut self.error_reader).await;
        let error_output = {
            let kept = self
                .error_output
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            String::from_utf8_lossy(&kept).trim().to_owned()
        };
        // No resolution can follow: one being reported finishes first.
        let mut ended = self.ended.lock().await;
        *ended = true;
        let _ = self
            .events
            .send(SessionEvent::Exited {
                success,
                error_output,
            })
            .await;
    }

    /// Routes one record; returns `false` once the session was killed.
    async fn handle(&mut self, record: Map<String, Value>) -> bool {
        if record.get("type").and_then(Value::as_str) == Some("response") {
            self.answer_command(record);
            return true;
        }
        for event in self.adapter.translate(&record, self.started.elapsed()) {
            let mut expiry = None;
            let forward = match &event {
                AgentEvent::RequestOpened {
                    request,
                    kind,
                    tool,
                    read_only,
                    session,
                    ..
                } => {
                    let answer = {
                        let mut shared = self.state.lock();
                        shared.allow_in(session.clone());
                        let allowed = shared.allowed.contains(tool);
                        if shared.stopping {
                            Some(json!({ "cancelled": true }))
                        } else if needs_user(shared.mode, *kind, *read_only, allowed) {
                            shared
                                .approvals
                                .insert(request.clone(), (tool.clone(), session.clone()));
                            None
                        } else {
                            Some(json!({ "value": "allow-auto" }))
                        }
                    };
                    match answer {
                        Some(answer) => {
                            if !self.reply(request, answer).await {
                                return false;
                            }
                            false
                        }
                        None => true,
                    }
                }
                AgentEvent::UserInputRequested { request, .. } => {
                    let stopping = {
                        let mut shared = self.state.lock();
                        if !shared.stopping {
                            shared.questions.insert(request.clone());
                        }
                        shared.stopping
                    };
                    if stopping {
                        if !self.reply(request, json!({ "cancelled": true })).await {
                            return false;
                        }
                        continue;
                    }
                    expiry = record
                        .get("timeout")
                        .and_then(Value::as_u64)
                        // Like pi, which sets no timer for a zero timeout.
                        .filter(|timeout| *timeout > 0)
                        .map(|timeout| (request.clone(), Duration::from_millis(timeout)));
                    true
                }
                AgentEvent::TurnStarted { .. } => {
                    let mut shared = self.state.lock();
                    shared.running = true;
                    shared.stopping = false;
                    true
                }
                AgentEvent::TurnCompleted { .. } | AgentEvent::TurnAborted { .. } => {
                    let mut shared = self.state.lock();
                    shared.running = false;
                    shared.stopping = false;
                    true
                }
                _ => true,
            };
            if forward {
                let events = self.events.clone();
                if self
                    .unless_killed(events.send(SessionEvent::Agent(event)))
                    .await
                    .is_none()
                {
                    return false;
                }
            }
            if let Some((request, timeout)) = expiry {
                self.expire(request, timeout);
            }
        }
        true
    }

    /// Answers an agent dialog without the user; returns `false` once the session was killed,
    /// so an agent that stopped reading cannot hold the kill up.
    async fn reply(&mut self, request: &RequestId, answer: Value) -> bool {
        let writer = self.writer.clone();
        self.unless_killed(writer.send(ui_response(request, answer)))
            .await
            .is_some()
    }

    /// Forgets a question once its time is up: pi then answers it itself without telling.
    /// An answered question or the agent's exit ends the timer, so it keeps nothing alive.
    fn expire(&self, request: RequestId, timeout: Duration) {
        let (state, events, ended) = (self.state.clone(), self.events.clone(), self.ended.clone());
        let expired = request.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            // Like a user's answer, it never follows the exit.
            let ended = ended.lock().await;
            let open = {
                let mut shared = state.lock();
                // Its own timer ends here; aborting it would drop the report below.
                shared.expiries.remove(&expired);
                shared.questions.remove(&expired)
            };
            if !*ended && open {
                let resolved = AgentEvent::UserInputResolved { request: expired };
                let _ = events.send(SessionEvent::Agent(resolved)).await;
            }
        });
        self.state
            .lock()
            .expiries
            .insert(request, timer.abort_handle());
    }

    /// Fails the command an oversized response answers, or reports a skipped event; returns
    /// `false` once the session was killed.
    async fn skip(&mut self, start: &[u8]) -> bool {
        let start = String::from_utf8_lossy(start);
        // pi writes a record's type and id first, so its start names it.
        let id = start
            .split_once(r#""id":""#)
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(id, _)| id.to_owned());
        if start.contains(r#""type":"response""#)
            && let Some(id) = id.as_deref().and_then(|id| id.parse::<u64>().ok())
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
        // The agent waits for an answer to its dialog; dismissing it fails the dialog instead,
        // and an approval dismissed this way blocks its call.
        if start.contains(r#""type":"extension_ui_request""#)
            && let Some(id) = id
            && !self
                .reply(&RequestId(id), json!({ "cancelled": true }))
                .await
        {
            return false;
        }
        let skipped = AgentEvent::Notification {
            level: NotificationLevel::Warning,
            message: format!(
                "Skipped an agent record larger than {} MiB",
                MAX_RECORD_BYTES / (1024 * 1024)
            ),
        };
        let events = self.events.clone();
        self.unless_killed(events.send(SessionEvent::Agent(skipped)))
            .await
            .is_some()
    }

    /// Resolves the command a response answers, moving its data rather than copying it.
    fn answer_command(&self, mut response: Map<String, Value>) {
        let id = response
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<u64>().ok());
        let Some(id) = id else {
            return;
        };
        let (reply, switch) = {
            let mut shared = self.state.lock();
            let reply = shared
                .replies
                .as_mut()
                .and_then(|replies| replies.remove(&id));
            (reply, shared.switches.remove(&id))
        };
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
        // pi answers an abort once the agent is idle, also when no turn ran to end; later
        // dialogs belong to whatever comes next.
        if response.get("command").and_then(Value::as_str) == Some("abort") {
            self.state.lock().stopping = false;
        }
        // Before any later record is read: tools allowed for the rest of a session need asking
        // again once the agent switched to another one.
        if switch && result.as_ref().is_ok_and(switched) {
            let mut shared = self.state.lock();
            shared.allowed.clear();
            shared.switched = true;
            shared.unchecked = true;
        }
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

/// Reads `reader` to its end and keeps the last [`MAX_ERROR_OUTPUT`] bytes in `kept`.
pub(super) async fn keep_end(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    kept: Arc<Mutex<Vec<u8>>>,
) {
    let mut chunk = [0; 8192];
    while let Ok(read @ 1..) = reader.read(&mut chunk).await {
        let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
        kept.extend_from_slice(&chunk[..read]);
        let excess = kept.len().saturating_sub(MAX_ERROR_OUTPUT);
        kept.drain(..excess);
    }
}
