//! Agent sessions for the Liquid app. Calls run on flutter_rust_bridge's worker threads, never on
//! the Dart main isolate, and events stream to Dart as they arrive (ADR-0182).

use std::sync::{LazyLock, PoisonError};

use std::path::{Path, PathBuf};

use meta_harness_core::{
    AgentCommand, Answer, CommandError, Delivery, DiffError, RequestId, RunDiff, Session,
    SessionEvent, recent_sessions, session_title,
};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, mpsc};

use crate::api::events::{BridgeDecision, BridgeEvent, BridgeFileChange, BridgeQueue};
use crate::api::history::{BridgeContextDetail, BridgeHistory, BridgeTranscript};
use crate::convert::{coalesce, describe};
use crate::frb_generated::StreamSink;
use crate::ready::{SHUTDOWN_GRACE, START_TIMEOUT, error_output, ready};

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the bridge runtime starts")
});

/// How to start an agent process that speaks pi's RPC protocol.
pub struct SessionConfig {
    /// The executable.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Its working directory.
    pub cwd: String,
}

/// A session pi stored, as the start page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStoredSession {
    /// The session file.
    pub path: String,
    /// Its name, or else the first line of its first prompt, or else "Untitled session".
    pub title: String,
    /// When pi last wrote it, in Unix milliseconds.
    pub modified_ms: i64,
}

/// A model the agent can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModel {
    /// The provider, such as `anthropic`.
    pub provider: String,
    /// The provider's model id.
    pub id: String,
    /// The name to show.
    pub name: String,
}

/// How much an agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeApprovalMode {
    /// Ask for every call that is not read-only.
    Ask,
    /// Also allow file changes without asking.
    AllowEdits,
    /// Allow every call.
    FullAccess,
}

/// A running agent session; dropping it kills the agent.
#[flutter_rust_bridge::frb(opaque)]
pub struct AgentSession {
    session: Mutex<Option<Session>>,
    /// The session's events until Dart subscribes; the core buffers a bounded number and then
    /// pauses the agent.
    receiver: std::sync::Mutex<Option<mpsc::Receiver<SessionEvent>>>,
    /// Events that arrived while the agent started.
    early: std::sync::Mutex<Vec<SessionEvent>>,
    /// What the latest run changed in the project.
    diff: RunDiff,
    /// The directory the agent works in.
    cwd: PathBuf,
    /// Stops the agent even while a request holds the session.
    stopper: std::sync::Mutex<Option<meta_harness_core::Stopper>>,
}

impl AgentSession {
    /// Starts an agent; subscribe with [`AgentSession::events`].
    pub fn start(config: SessionConfig) -> Result<Self, String> {
        let command = AgentCommand {
            program: config.program.into(),
            args: config.args.into_iter().map(Into::into).collect(),
            cwd: config.cwd.into(),
        };
        let _runtime = RUNTIME.enter();
        let diff = RUNTIME.block_on(RunDiff::open(&command.cwd));
        let (mut session, mut receiver) = Session::start(&command)
            .map_err(|error| format!("could not run {}: {error}", command.program.display()))?;
        // Without the extension pi runs tool calls without asking, so the agent must confirm
        // it within a time limit; dropping the session ends the agent.
        let (gate, mut early) = RUNTIME.block_on(ready(&mut session, &mut receiver));
        match gate {
            Some(Ok(())) => {}
            // An agent that ended at once says why on its error output.
            Some(Err(CommandError::Closed)) => {
                let output = early
                    .iter()
                    .find_map(|event| match event {
                        SessionEvent::Exited { error_output, .. } => Some(error_output.clone()),
                        _ => None,
                    })
                    .or_else(|| RUNTIME.block_on(error_output(&mut receiver)));
                return Err(output
                    .filter(|output| !output.is_empty())
                    .unwrap_or_else(|| describe(CommandError::Closed)));
            }
            Some(Err(error)) => return Err(describe(error)),
            None => {
                return Err(format!(
                    "pi did not answer within {} s of starting",
                    START_TIMEOUT.as_secs()
                ));
            }
        }
        early.shrink_to_fit();
        let stopper = session.stopper();
        Ok(Self {
            stopper: std::sync::Mutex::new(stopper),
            session: Mutex::new(Some(session)),
            receiver: std::sync::Mutex::new(Some(receiver)),
            early: std::sync::Mutex::new(early),
            diff,
            cwd: command.cwd,
        })
    }

    /// Streams the session's events; a session has one subscriber.
    pub fn events(&self, sink: StreamSink<BridgeEvent>) -> Result<(), String> {
        self.forward(move |event| sink.add(event).is_ok())
    }

    /// Hands each batch of events to `deliver` until it returns `false`; Rust-side consumers
    /// such as tests and observation workloads use it instead of a Dart stream.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn forward(
        &self,
        mut deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut receiver = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| "the session's events already have a subscriber".to_owned())?;
        let early = std::mem::take(&mut *self.early.lock().unwrap_or_else(PoisonError::into_inner));
        RUNTIME.spawn(async move {
            // Events from before the start finished come first.
            let mut batch: Vec<_> = early.into_iter().map(BridgeEvent::from).collect();
            if !coalesce(std::mem::take(&mut batch))
                .into_iter()
                .all(&mut deliver)
            {
                return;
            }
            while let Some(event) = receiver.recv().await {
                batch.push(BridgeEvent::from(event));
                while let Ok(event) = receiver.try_recv() {
                    batch.push(BridgeEvent::from(event));
                }
                // Dart closed the stream: stop; dropping the session ends the agent.
                if !coalesce(std::mem::take(&mut batch))
                    .into_iter()
                    .all(&mut deliver)
                {
                    break;
                }
            }
        });
        Ok(())
    }

    /// Sends a message: an idle agent starts a run with it, a running one takes it before its
    /// next model request or, with `follow_up`, once it is done. A new run first captures the
    /// project, so the diff view shows what the run changes.
    pub fn send(&self, message: String, follow_up: bool) -> Result<(), String> {
        RUNTIME.block_on(async {
            // Captured without holding the session, so a slow project never holds up the
            // agent or shutdown.
            let idle = !open(&mut *self.session.lock().await)?.running();
            let capture = if idle {
                Some(self.diff.capture().await)
            } else {
                None
            };
            let (pending, capture) = {
                let mut session = self.session.lock().await;
                let session = open(&mut session)?;
                // A run that started meanwhile takes this message instead of starting one.
                let capture = capture.filter(|_| !session.running());
                let delivery = if follow_up {
                    Delivery::FollowUp
                } else {
                    Delivery::Steer
                };
                (
                    session.send(&message, delivery).await.map_err(describe)?,
                    capture,
                )
            };
            // The session stays free meanwhile, so questions asked before acceptance can be
            // answered.
            let started = pending.started().await.map_err(describe)?;
            // Only a prompt that started a run makes its capture count.
            if let Some(capture) = capture.filter(|_| started) {
                self.diff.started(capture);
            }
            Ok(())
        })
    }

    /// Removes the messages waiting for the running agent and returns them.
    pub fn clear_queue(&self) -> Result<BridgeQueue, String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            Ok(open(&mut session)?
                .clear_queue()
                .await
                .map_err(describe)?
                .into())
        })
    }

    /// Files the latest run changed, compared with the project before it; none outside Git.
    pub fn changes(&self) -> Result<Option<Vec<BridgeFileChange>>, String> {
        match RUNTIME.block_on(self.diff.changes()) {
            Ok(changes) => Ok(Some(changes.into_iter().map(Into::into).collect())),
            Err(DiffError::NotARepository) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    /// The unified diff of one file since the latest run started.
    pub fn patch(&self, path: String) -> Result<String, String> {
        RUNTIME
            .block_on(self.diff.patch(&path))
            .map_err(|error| error.to_string())
    }

    /// Reads the session's branch tree and the Trajectory of each branch ending at an entry in
    /// `leaves`, or of the current branch when `leaves` is empty.
    pub fn history(&self, leaves: Vec<String>) -> Result<BridgeHistory, String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            let history = open(&mut session)?.history().await.map_err(describe)?;
            let tree = history.tree();
            let trajectories = if leaves.is_empty() {
                vec![history.trajectory(None).into()]
            } else {
                leaves
                    .iter()
                    .map(|leaf| history.trajectory(Some(leaf)).into())
                    .collect()
            };
            Ok(BridgeHistory {
                tree: tree.nodes.into_iter().map(Into::into).collect(),
                leaf: tree.leaf,
                trajectories,
                usage: history.usage().into(),
            })
        })
    }

    /// The chat of the current branch: the items of up to `limit` stored entries before
    /// `before`, or of the newest ones.
    pub fn transcript(
        &self,
        before: Option<String>,
        limit: u32,
    ) -> Result<BridgeTranscript, String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            let history = open(&mut session)?.history().await.map_err(describe)?;
            let limit = usize::try_from(limit).unwrap_or(usize::MAX);
            Ok(history.transcript(before.as_deref(), limit).into())
        })
    }

    /// Everything the model saw for the request behind the response `entry`; `None` unless
    /// `entry` is a response.
    pub fn context(&self, entry: String) -> Result<Option<BridgeContextDetail>, String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            let history = open(&mut session)?.history().await.map_err(describe)?;
            Ok(history.context(&entry).map(Into::into))
        })
    }

    /// Continues the session from `entry`; the next prompt starts a new branch there.
    pub fn branch(&self, entry: String) -> Result<(), String> {
        RUNTIME.block_on(async {
            let pending = open(&mut *self.session.lock().await)?
                .branch(&entry, None)
                .await
                .map_err(describe)?;
            pending.accepted().await.map_err(describe)
        })
    }

    /// Up to `limit` stored sessions of this project, newest first, without the current one.
    pub fn recent_sessions(&self, limit: u32) -> Result<Vec<BridgeStoredSession>, String> {
        let current = RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?.state().await.map_err(describe)
        })?;
        // A session pi keeps in memory has no directory to list.
        let Some(file) = current.session_file else {
            return Ok(Vec::new());
        };
        let dir = file.parent().unwrap_or(Path::new("."));
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let stored = recent_sessions(dir, &self.cwd, Some(&file), limit)
            .map_err(|error| error.to_string())?;
        Ok(stored.into_iter().map(Into::into).collect())
    }

    /// The open session's title, as the start page names stored sessions; none before pi
    /// stored a name or a prompt.
    pub fn title(&self) -> Result<Option<String>, String> {
        let current = RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?.state().await.map_err(describe)
        })?;
        Ok(current
            .session_file
            .and_then(|file| session_title(&file, &self.cwd)))
    }

    /// Starts a new session in the same agent; `false` when an extension cancelled it.
    pub fn new_session(&self) -> Result<bool, String> {
        RUNTIME.block_on(async {
            let runs = self.diff.runs();
            let pending = open(&mut *self.session.lock().await)?
                .new_session()
                .await
                .map_err(describe)?;
            self.switched(pending, runs).await
        })
    }

    /// Continues the session stored in `path`; `false` when an extension cancelled it.
    pub fn switch_session(&self, path: String) -> Result<bool, String> {
        RUNTIME.block_on(async {
            let runs = self.diff.runs();
            let pending = open(&mut *self.session.lock().await)?
                .switch_session(Path::new(&path))
                .await
                .map_err(describe)?;
            self.switched(pending, runs).await
        })
    }

    /// Waits for a session switch without holding the session; the diff of the previous
    /// session's run no longer applies after it, unless a run started since `runs`.
    async fn switched(
        &self,
        pending: meta_harness_core::Pending,
        runs: u64,
    ) -> Result<bool, String> {
        let switched = pending.switched().await.map_err(describe)?;
        if switched {
            self.diff.switched(runs);
        }
        Ok(switched)
    }

    /// The models the agent can use.
    pub fn models(&self) -> Result<Vec<BridgeModel>, String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            Ok(open(&mut session)?
                .models()
                .await
                .map_err(describe)?
                .into_iter()
                .map(Into::into)
                .collect())
        })
    }

    /// The selected model as `provider/id`.
    pub fn current_model(&self) -> Result<Option<String>, String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            Ok(open(&mut session)?.state().await.map_err(describe)?.model)
        })
    }

    /// Selects the model the next requests use.
    pub fn set_model(&self, provider: String, id: String) -> Result<(), String> {
        RUNTIME.block_on(async {
            let pending = open(&mut *self.session.lock().await)?
                .set_model(&provider, &id)
                .await
                .map_err(describe)?;
            // pi runs extensions' handlers first; the session stays free for their questions.
            pending.accepted().await.map_err(describe)
        })
    }

    // The calls below wait for nothing but pi and run on flutter_rust_bridge's async runtime,
    // so calls waiting on a slow project or a hung agent in its thread pool never hold up a
    // decision, Stop or quitting.

    /// Aborts the running turn; open approvals are withdrawn and questions dismissed, so the
    /// agent can stop.
    pub async fn abort(&self) -> Result<(), String> {
        let pending = open(&mut *self.session.lock().await)?
            .abort()
            .await
            .map_err(describe)?;
        // pi answers once the agent is idle; the session stays free meanwhile.
        pending.accepted().await.map_err(describe)
    }

    /// Changes which tool calls need the user's approval.
    pub async fn set_approval_mode(&self, mode: BridgeApprovalMode) -> Result<(), String> {
        open(&mut *self.session.lock().await)?.set_approval_mode(mode.into());
        Ok(())
    }

    /// Answers an approval request.
    pub async fn decide(&self, request: String, decision: BridgeDecision) -> Result<(), String> {
        open(&mut *self.session.lock().await)?
            .decide(&RequestId(request), decision.into())
            .await
            .map_err(describe)
    }

    /// Answers a selection, input or editor question with text.
    pub async fn answer_text(&self, request: String, text: String) -> Result<(), String> {
        self.answer(request, Answer::Value(text)).await
    }

    /// Answers a confirmation question.
    pub async fn answer_confirm(&self, request: String, confirmed: bool) -> Result<(), String> {
        self.answer(request, Answer::Confirmed(confirmed)).await
    }

    /// Dismisses a question.
    pub async fn dismiss(&self, request: String) -> Result<(), String> {
        self.answer(request, Answer::Cancelled).await
    }

    /// Asks the agent to exit and kills it after a grace period; returns whether it exited
    /// successfully.
    pub async fn shutdown(&self) -> bool {
        let stopper = self
            .stopper
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let stopped = match stopper {
            Some(stopper) => stopper.stop(SHUTDOWN_GRACE).await,
            None => false,
        };
        // Requests still waiting end with the agent, so the session is free to drop.
        self.session.lock().await.take();
        self.diff.close().await;
        stopped
    }

    async fn answer(&self, request: String, answer: Answer) -> Result<(), String> {
        open(&mut *self.session.lock().await)?
            .answer(&RequestId(request), answer)
            .await
            .map_err(describe)
    }
}

fn open(session: &mut Option<Session>) -> Result<&mut Session, String> {
    session
        .as_mut()
        .ok_or_else(|| "the session was shut down".to_owned())
}
