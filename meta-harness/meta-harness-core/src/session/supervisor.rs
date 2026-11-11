//! Routes an agent's records to the session: answers, checkpoint requests and events, until
//! the agent exits.

use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdout};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::{CommandError, MAX_RECORD_BYTES, SessionEvent, State, Write, switched, ui_response};
use crate::framing::keep_end;
use crate::{
    AgentEvent, Checkpoint, Checkpoints, Frame, LineFramer, NotificationLevel, PiAdapter, RequestId,
};

const READ_BUFFER: usize = 64 * 1024;

/// The end of the agent's error output kept to explain why it failed.
const MAX_ERROR_OUTPUT: usize = 16 * 1024;

/// How long an exited agent's error output may take to end; a process the agent started can
/// keep it open.
const ERROR_OUTPUT_WAIT: Duration = Duration::from_millis(500);

// Two seconds lets ordinary final RPC writes drain without letting an inherited pipe hold
// session completion indefinitely. Event delivery keeps its own backpressure after collection.
const OUTPUT_DRAIN_WAIT: Duration = Duration::from_secs(2);
/// Aggregate bytes read after exit, independently of the per-record framing limit.
const MAX_OUTPUT_DRAIN_BYTES: usize = 16 * 1024 * 1024;

enum Output {
    Bytes(Vec<u8>),
    CutOff,
}

/// Reads the agent's output until it exits, fails or is killed, then reports the exit.
pub(super) struct Supervisor {
    pub(super) events: mpsc::Sender<SessionEvent>,
    pub(super) ended: Arc<tokio::sync::Mutex<bool>>,
    pub(super) writer: mpsc::Sender<Write>,
    pub(super) state: State,
    pub(super) adapter: PiAdapter,
    /// Kills the agent with `true` or when dropped; none once shutdown saw it exit.
    pub(super) killed: Option<oneshot::Receiver<bool>>,
    /// Set once `killed` asked to kill; it must not be polled again.
    pub(super) kill_requested: bool,
    /// The end of the agent's error output so far, and the task that reads it.
    pub(super) error_output: Arc<Mutex<Vec<u8>>>,
    pub(super) error_reader: tokio::task::JoinHandle<()>,
    pub(super) input_writer: tokio::task::JoinHandle<()>,
    /// Checkpoint requests, taken one after another in the order the agent asked.
    pub(super) captures: mpsc::Sender<CheckpointRequest>,
    /// Closed once the agent is gone, so no capture outlives it.
    pub(super) checkpoints: Arc<Checkpoints>,
}

/// A checkpoint the agent asked for, or why its request is unreadable.
pub(super) type CheckpointRequest = (RequestId, Result<Checkpoint, String>);

impl Supervisor {
    pub(super) async fn run(
        mut self,
        mut child: Child,
        stdout: ChildStdout,
        exit: oneshot::Sender<bool>,
    ) {
        let mut framer = LineFramer::new(MAX_RECORD_BYTES);
        let (output, mut reads) = mpsc::channel(1);
        let (exited, drain) = oneshot::channel();
        let mut exited = Some(exited);
        let reader = tokio::spawn(read_output(stdout, output, drain));
        let mut exit = Some(exit);
        let mut cut_off = false;
        let checkpoints = self.checkpoints.clone();
        let input_writer = self.input_writer.abort_handle();
        let failure = 'read: loop {
            let read = watching(
                self.unless_killed(reads.recv()),
                &mut child,
                &mut exited,
                &mut exit,
                &checkpoints,
                &input_writer,
            )
            .await;
            let Some(Some(read)) = read else {
                break None;
            };
            let frames = match read {
                Output::CutOff => {
                    cut_off = true;
                    break None;
                }
                Output::Bytes(bytes) => framer.push(&bytes),
            };
            for frame in frames {
                let record = match frame {
                    Frame::Record(record) => record,
                    Frame::Oversized { start, .. } => {
                        let skipped = watching(
                            self.skip(&start),
                            &mut child,
                            &mut exited,
                            &mut exit,
                            &checkpoints,
                            &input_writer,
                        );
                        if !skipped.await {
                            break 'read None;
                        }
                        continue;
                    }
                };
                let Ok(Value::Object(record)) = serde_json::from_slice::<Value>(&record) else {
                    break 'read Some("agent wrote a record that is not a JSON object".to_owned());
                };
                if !watching(
                    self.handle(record),
                    &mut child,
                    &mut exited,
                    &mut exit,
                    &checkpoints,
                    &input_writer,
                )
                .await
                {
                    break 'read None;
                }
            }
        };
        reader.abort();
        let _ = reader.await;
        // Keep the checkout guard until the process is gone, even when publishing its
        // failure will wait for room in the event buffer.
        let stop = failure.is_some() || self.kill_requested;
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
        self.input_writer.abort();
        let _ = (&mut self.input_writer).await;
        {
            // Nothing is asked of a gone agent: open dialogs end with it, and their expiry
            // reports nothing after the exit.
            let mut shared = self.state.lock();
            shared.replies = None;
            shared.questions.clear();
            for (_, expiry) in shared.expiries.drain() {
                expiry.abort();
            }
        }
        // A capture in progress stops with its Git and the programs Git started, before the
        // exit is reported.
        self.checkpoints.close().await;
        // Shutdown learns of the exit first; the event waits for buffer space.
        if let Some(exit) = exit.take() {
            let _ = exit.send(success);
        }
        if tokio::time::timeout(ERROR_OUTPUT_WAIT, &mut self.error_reader)
            .await
            .is_err()
        {
            self.error_reader.abort();
            let _ = (&mut self.error_reader).await;
        }
        let error_output = {
            let kept = self
                .error_output
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            String::from_utf8_lossy(&kept).trim().to_owned()
        };
        if let Some(reason) = failure {
            let events = self.events.clone();
            let _ = self
                .unless_killed(events.send(SessionEvent::Failed(reason)))
                .await;
        }
        if cut_off {
            let _ = self
                .events
                .send(SessionEvent::Agent(AgentEvent::Notification {
                    level: NotificationLevel::Warning,
                    message: "Some agent output may be missing.".to_owned(),
                }))
                .await;
        }
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
        // The core answers checkpoint requests itself; the user never sees them.
        if let Some(request) = self.adapter.checkpoint_request(&record) {
            let captures = self.captures.clone();
            return self.unless_killed(captures.send(request)).await.is_some();
        }
        for event in self.adapter.translate(&record) {
            let mut expiry = None;
            let forward = match &event {
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
                    // Counted from now: pi's timer runs while the event waits for room.
                    expiry = record
                        .get("timeout")
                        .and_then(Value::as_u64)
                        // Like pi, which sets no timer for a zero timeout.
                        .filter(|timeout| *timeout > 0)
                        .map(|timeout| {
                            (
                                request.clone(),
                                Instant::now() + Duration::from_millis(timeout),
                            )
                        });
                    true
                }
                AgentEvent::TurnStarted { .. } => {
                    self.state.lock().running = true;
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
            if let Some((request, deadline)) = expiry {
                self.expire(request, deadline);
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

    /// Forgets a question at `deadline`: pi then answers it itself without telling. An
    /// answered question or the agent's exit ends the timer, so it keeps nothing alive.
    pub(super) fn expire(&self, request: RequestId, deadline: Instant) {
        let (state, events, ended) = (self.state.clone(), self.events.clone(), self.ended.clone());
        let expired = request.clone();
        // Held until the timer's handle is registered, so the timer's own removal cannot run
        // first and leave the handle behind; a question answered meanwhile gets no timer.
        let mut shared = self.state.lock();
        if !shared.questions.contains(&request) {
            return;
        }
        let timer = tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
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
        shared.expiries.insert(request, timer.abort_handle());
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
        // The agent waits for an answer to its dialog; dismissing it fails the dialog instead.
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
        // Once the agent switched sessions, the next history read starts over.
        if switch && result.as_ref().is_ok_and(switched) {
            self.state.lock().switched = true;
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
            if let Some(kill) = killed {
                match Pin::new(kill).poll(context) {
                    Poll::Ready(Ok(false)) => *killed = None,
                    Poll::Ready(_) => return Poll::Ready(None),
                    Poll::Pending => {}
                }
            }
            Poll::Pending
        })
        .await;
        self.kill_requested = outcome.is_none();
        outcome
    }
}

/// Runs `work` while watching for the agent's exit. Once the agent exited, the checkpoints close
/// at once, so no capture outlives it, input delivery ends, output collection gets its deadline
/// and shutdown learns of the exit through `stopped`, so its grace period never ends in a kill
/// that would cut delivery short. Already read records still finish delivery, even when the
/// event buffer is full.
async fn watching<F: Future>(
    work: F,
    child: &mut Child,
    exited: &mut Option<oneshot::Sender<Instant>>,
    stopped: &mut Option<oneshot::Sender<bool>>,
    checkpoints: &Checkpoints,
    input_writer: &tokio::task::AbortHandle,
) -> F::Output {
    let mut work = pin!(work);
    if exited.is_some() {
        let done = {
            let mut waiting = pin!(child.wait());
            poll_fn(|context| {
                if let Poll::Ready(status) = waiting.as_mut().poll(context) {
                    return Poll::Ready(Err(status));
                }
                work.as_mut().poll(context).map(Ok)
            })
            .await
        };
        let status = match done {
            Ok(output) => return output,
            Err(status) => status,
        };
        if let Some(exited) = exited.take() {
            let _ = exited.send(Instant::now() + OUTPUT_DRAIN_WAIT);
        }
        // A descendant may retain stdin without reading it. Dropping the writer's receiver
        // releases queued automatic answers while already collected output finishes delivery.
        input_writer.abort();
        checkpoints.close().await;
        if let Some(stopped) = stopped.take() {
            let _ = stopped.send(status.is_ok_and(|status| status.success()));
        }
    }
    work.await
}

/// Streams with bounded backpressure while pi runs. After exit, collect independently of
/// event delivery until EOF, the deadline or the byte cap, then close the pipe before forwarding
/// the tail.
/// This retains ordinary buffered output even when Liquid is slow to consume accepted events.
async fn read_output(
    mut stdout: ChildStdout,
    output: mpsc::Sender<Output>,
    mut exited: oneshot::Receiver<Instant>,
) {
    let mut buffer = vec![0; READ_BUFFER];
    let (deadline, mut tail) = loop {
        let length = match race(&mut exited, stdout.read(&mut buffer)).await {
            Ok(Ok(deadline)) => break (deadline, Vec::new()),
            Ok(Err(_)) | Err(Ok(0) | Err(_)) => return,
            Err(Ok(length)) => length,
        };
        // Reserve before moving the bytes so cancellation at process exit keeps this chunk.
        match race(&mut exited, output.reserve()).await {
            Ok(Ok(deadline)) => break (deadline, vec![buffer[..length].to_vec()]),
            Ok(Err(_)) | Err(Err(_)) => return,
            Err(Ok(permit)) => permit.send(Output::Bytes(buffer[..length].to_vec())),
        }
    };
    let deadline = tokio::time::sleep_until(deadline);
    tokio::pin!(deadline);
    let mut remaining = MAX_OUTPUT_DRAIN_BYTES;
    let cut_off = loop {
        if remaining == 0 {
            break true;
        }
        let capacity = buffer.len().min(remaining);
        match race(&mut deadline, stdout.read(&mut buffer[..capacity])).await {
            Ok(()) => break true,
            Err(Ok(0) | Err(_)) => break false,
            Err(Ok(length)) => {
                remaining -= length;
                tail.push(buffer[..length].to_vec());
            }
        }
    };
    drop(stdout);
    for bytes in tail {
        if output.send(Output::Bytes(bytes)).await.is_err() {
            return;
        }
    }
    if cut_off {
        let _ = output.send(Output::CutOff).await;
    }
}

/// Waits for either future, polling `first` first so a busy pipe cannot starve a deadline.
async fn race<A: Future, B: Future>(first: A, second: B) -> Result<A::Output, B::Output> {
    let (mut first, mut second) = (pin!(first), pin!(second));
    poll_fn(|context| {
        if let Poll::Ready(output) = first.as_mut().poll(context) {
            return Poll::Ready(Ok(output));
        }
        second.as_mut().poll(context).map(Err)
    })
    .await
}

/// Reads `reader` to its end and keeps the last [`MAX_ERROR_OUTPUT`] bytes in `kept`.
pub(super) async fn read_end(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    kept: Arc<Mutex<Vec<u8>>>,
) {
    let mut chunk = [0; 8192];
    while let Ok(read @ 1..) = reader.read(&mut chunk).await {
        let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
        keep_end(&mut kept, &chunk[..read], MAX_ERROR_OUTPUT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_s_time_limit_runs_while_its_event_waits_for_room() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("test runtime starts");
        runtime.block_on(async {
            let (events, mut received) = mpsc::channel(1);
            let (writer, _writes) = mpsc::channel(1);
            let (captures, _requests) = mpsc::channel(1);
            let state = State(Arc::default());
            let mut supervisor = Supervisor {
                events: events.clone(),
                ended: Arc::default(),
                writer,
                state: state.clone(),
                adapter: PiAdapter::new(),
                killed: None,
                kill_requested: false,
                error_output: Arc::default(),
                error_reader: tokio::spawn(async {}),
                input_writer: tokio::spawn(async {}),
                captures,
                checkpoints: Arc::new(Checkpoints::new(&std::env::temp_dir())),
            };
            // The event buffer is full, so the question waits for room.
            let waiting = AgentEvent::QueueChanged(Default::default());
            events
                .send(SessionEvent::Agent(waiting.clone()))
                .await
                .expect("room");
            let Value::Object(record) = json!({
                "type": "extension_ui_request",
                "id": "q1",
                "method": "confirm",
                "title": "Sure?",
                "message": "Really",
                "timeout": 1000,
            }) else {
                unreachable!()
            };
            let handled = tokio::spawn(async move { supervisor.handle(record).await });
            let request = RequestId("q1".to_owned());
            while !state.lock().questions.contains(&request) {
                tokio::task::yield_now().await;
            }
            tokio::time::advance(Duration::from_secs(2)).await;
            assert_eq!(received.recv().await, Some(SessionEvent::Agent(waiting)));
            assert!(matches!(
                received.recv().await,
                Some(SessionEvent::Agent(AgentEvent::UserInputRequested { .. }))
            ));
            let delivered = Instant::now();
            assert_eq!(
                received.recv().await,
                Some(SessionEvent::Agent(AgentEvent::UserInputResolved {
                    request
                }))
            );
            assert_eq!(
                Instant::now(),
                delivered,
                "pi answered it itself while it waited"
            );
            assert!(handled.await.expect("handled"));
        });
    }
}
