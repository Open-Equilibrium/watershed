//! Agent sessions for the Liquid app. Calls run on flutter_rust_bridge's worker threads, never on
//! the Dart main isolate, and events stream to Dart as they arrive (ADR-0182).

use std::sync::{LazyLock, PoisonError};
use std::time::Duration;

use meta_harness_core::{AgentCommand, Answer, RequestId, Session, SessionEvent};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, mpsc};

use crate::api::events::{BridgeDecision, BridgeEvent};
use crate::convert::{coalesce, describe};
use crate::frb_generated::StreamSink;

/// Time an agent gets to exit after its input closes before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

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
        let (session, receiver) = Session::start(&command).map_err(|error| error.to_string())?;
        Ok(Self {
            session: Mutex::new(Some(session)),
            receiver: std::sync::Mutex::new(Some(receiver)),
        })
    }

    /// Streams the session's events; a session has one subscriber.
    pub fn events(&self, sink: StreamSink<BridgeEvent>) -> Result<(), String> {
        self.forward(move |event| sink.add(event).is_ok())
    }

    /// Hands each batch of events to `deliver` until it returns `false`.
    pub(crate) fn forward(
        &self,
        mut deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut receiver = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| "the session's events already have a subscriber".to_owned())?;
        RUNTIME.spawn(async move {
            let mut batch = Vec::new();
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

    /// Sends a prompt that starts a turn.
    pub fn prompt(&self, message: String) -> Result<(), String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?.prompt(&message).await.map_err(describe)
        })
    }

    /// Steers the running turn.
    pub fn steer(&self, message: String) -> Result<(), String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?.steer(&message).await.map_err(describe)
        })
    }

    /// Aborts the running turn.
    pub fn abort(&self) -> Result<(), String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?.abort().await.map_err(describe)
        })
    }

    /// Changes which tool calls need the user's approval.
    pub fn set_approval_mode(&self, mode: BridgeApprovalMode) -> Result<(), String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?.set_approval_mode(mode.into());
            Ok(())
        })
    }

    /// Answers an approval request.
    pub fn decide(&self, request: String, decision: BridgeDecision) -> Result<(), String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?
                .decide(&RequestId(request), decision.into())
                .await
                .map_err(describe)
        })
    }

    /// Answers a selection, input or editor question with text.
    pub fn answer_text(&self, request: String, text: String) -> Result<(), String> {
        self.answer(request, Answer::Value(text))
    }

    /// Answers a confirmation question.
    pub fn answer_confirm(&self, request: String, confirmed: bool) -> Result<(), String> {
        self.answer(request, Answer::Confirmed(confirmed))
    }

    /// Dismisses a question.
    pub fn dismiss(&self, request: String) -> Result<(), String> {
        self.answer(request, Answer::Cancelled)
    }

    /// Asks the agent to exit and kills it after a grace period; returns whether it exited
    /// successfully.
    pub fn shutdown(&self) -> bool {
        RUNTIME.block_on(async {
            let session = self.session.lock().await.take();
            match session {
                Some(session) => session.shutdown(SHUTDOWN_GRACE).await,
                None => false,
            }
        })
    }

    fn answer(&self, request: String, answer: Answer) -> Result<(), String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            open(&mut session)?
                .answer(&RequestId(request), answer)
                .await
                .map_err(describe)
        })
    }
}

fn open(session: &mut Option<Session>) -> Result<&mut Session, String> {
    session
        .as_mut()
        .ok_or_else(|| "the session was shut down".to_owned())
}
