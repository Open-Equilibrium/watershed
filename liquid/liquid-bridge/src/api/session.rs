//! Agent sessions for the Liquid app. Calls run on flutter_rust_bridge's worker threads, never on
//! the Dart main isolate, and events stream to Dart as they arrive (ADR-0182).

use std::sync::{Arc, LazyLock, PoisonError};

use std::path::{Path, PathBuf};

use meta_harness_core::{
    AgentCommand, Answer, Checkpoint, Checkpoints, CommandError, Delivery, DiffError, Phase,
    RequestId, Session, SessionEvent, TurnCheckpoints, Whitespace, recent_sessions, session_id,
    session_title, session_title_of, visible_lines, visible_name,
};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex, mpsc};

use crate::api::events::{BridgeDecision, BridgeEvent, BridgeQueue};
use crate::api::history::{
    BridgeChangesState, BridgeContextDetail, BridgeHistory, BridgeRevertPlan, BridgeRunningAgent,
    BridgeTranscript, BridgeTurn, BridgeTurnChanges,
};
use crate::convert::describe;
use crate::frb_generated::StreamSink;
use crate::ready::{SHUTDOWN_GRACE, START_TIMEOUT, error_output, ready};
use crate::stream::{STREAM_WINDOW, gather, push};

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the bridge runtime starts")
});

/// How to start pi; the core adds its arguments.
pub struct SessionConfig {
    /// The executable.
    pub program: String,
    /// Watershed's pi extension, by absolute path.
    pub extension_path: String,
    /// A stored session file to reopen, by absolute path.
    pub session: Option<String>,
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

/// A session with checkpoints in the project's repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCheckpointSession {
    /// Its title, as the start page names stored sessions; "Unknown session" when no session
    /// of this project is stored with its id.
    pub title: String,
    /// Its turns with checkpoints.
    pub turns: i64,
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
    /// Tokens the model can take in one request, when the agent knows them.
    pub context_window: Option<i64>,
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
    /// The project's checkpoints at each turn's start and end.
    checkpoints: Arc<Checkpoints>,
    /// The directory the agent works in.
    cwd: PathBuf,
    /// Stops the agent even while a request holds the session.
    stopper: std::sync::Mutex<Option<meta_harness_core::Stopper>>,
}

impl AgentSession {
    /// Starts pi; subscribe with [`AgentSession::events`].
    pub fn start(config: SessionConfig) -> Result<Self, String> {
        Self::launch(AgentCommand {
            program: config.program.into(),
            args: Vec::new(),
            extension: config.extension_path.into(),
            session: config.session.map(Into::into),
            cwd: config.cwd.into(),
        })
    }

    /// Starts an agent as `command` says.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn launch(command: AgentCommand) -> Result<Self, String> {
        let _runtime = RUNTIME.enter();
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
        let checkpoints = session.checkpoints();
        Ok(Self {
            stopper: std::sync::Mutex::new(stopper),
            session: Mutex::new(Some(session)),
            receiver: std::sync::Mutex::new(Some(receiver)),
            early: std::sync::Mutex::new(early),
            checkpoints,
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
            let mut batch = Vec::new();
            for event in early {
                push(&mut batch, BridgeEvent::from(event));
            }
            if !std::mem::take(&mut batch).into_iter().all(&mut deliver) {
                return;
            }
            while let Some(event) = receiver.recv().await {
                push(&mut batch, BridgeEvent::from(event));
                // Bounded by the window, so a busy agent never holds delivery back.
                gather(&mut batch, &mut receiver, tokio::time::sleep(STREAM_WINDOW)).await;
                // Dart closed the stream: stop; dropping the session ends the agent.
                if !std::mem::take(&mut batch).into_iter().all(&mut deliver) {
                    break;
                }
            }
        });
        Ok(())
    }

    /// Sends a message: an idle agent starts a run with it, a running one takes it before its
    /// next model request or, with `follow_up`, once it is done. Each run's start and end are
    /// checkpointed, so the Diff tab shows what it changed.
    pub fn send(&self, message: String, follow_up: bool) -> Result<(), String> {
        RUNTIME.block_on(async {
            let pending = {
                let mut session = self.session.lock().await;
                let delivery = if follow_up {
                    Delivery::FollowUp
                } else {
                    Delivery::Steer
                };
                open(&mut session)?
                    .send(&message, delivery)
                    .await
                    .map_err(describe)?
            };
            // The session stays free meanwhile, so questions asked before acceptance can be
            // answered.
            pending.started().await.map_err(describe).map(drop)
        })
    }

    /// Whether the project is in a Git repository, so its turns can have checkpoints.
    pub fn in_repository(&self) -> Result<bool, String> {
        RUNTIME
            .block_on(self.checkpoints.in_repository())
            .map_err(shown)
    }

    /// The turns of the current branch, oldest first, with what each changed.
    pub fn turns(&self) -> Result<Vec<BridgeTurn>, String> {
        RUNTIME.block_on(async {
            let turns = branch_turns(&mut *self.session.lock().await).await?;
            Ok(turns.into_iter().map(Into::into).collect())
        })
    }

    /// The files that changed from the start of turn `from` to the end of turn `to`, both on
    /// the current branch; with `hide_whitespace`, changes that are only whitespace are left
    /// out.
    pub fn turn_changes(
        &self,
        from: String,
        to: String,
        hide_whitespace: bool,
    ) -> Result<BridgeTurnChanges, String> {
        RUNTIME.block_on(async {
            let (from, to) = self.checkpoints_of(&from, &to).await?;
            let read = self
                .checkpoints
                .changes(&from, &to, whitespace(hide_whitespace))
                .await;
            let (state, files) = match read {
                Ok(changes) => (
                    BridgeChangesState::Shown,
                    changes.into_iter().map(Into::into).collect(),
                ),
                Err(error) => (unread(error)?, Vec::new()),
            };
            Ok(BridgeTurnChanges { state, files })
        })
    }

    /// The unified diff of one file from the start of turn `from` to the end of turn `to`;
    /// characters that could hide or reorder its lines are written out, as in the file's shown
    /// name, but for the carriage returns of Windows line ends, which the Diff tab marks.
    pub fn turn_patch(
        &self,
        from: String,
        to: String,
        path: String,
        hide_whitespace: bool,
    ) -> Result<String, String> {
        RUNTIME.block_on(async {
            let (from, to) = self.checkpoints_of(&from, &to).await?;
            self.checkpoints
                .patch(&from, &to, &path, whitespace(hide_whitespace))
                .await
                .map(|patch| visible_lines(&patch))
                .map_err(shown)
        })
    }

    /// The start checkpoint of turn `from` and the end checkpoint of turn `to`, which must be
    /// turns of the current branch, so Dart never names a checkpoint itself.
    async fn checkpoints_of(
        &self,
        from: &str,
        to: &str,
    ) -> Result<(Checkpoint, Checkpoint), String> {
        let turns = branch_turns(&mut *self.session.lock().await).await?;
        Ok((
            checkpoint(turn_of(&turns, from)?, Phase::Start),
            checkpoint(turn_of(&turns, to)?, Phase::End),
        ))
    }

    /// What reverting the files to before turn `turn` of the current branch would change;
    /// files changed after the branch's last turn ended are marked.
    pub fn revert_plan(&self, turn: String) -> Result<BridgeRevertPlan, String> {
        RUNTIME.block_on(async {
            let (to, since, dir) = {
                let mut session = self.session.lock().await;
                let turns = branch_turns(&mut session).await?;
                let to = checkpoint(turn_of(&turns, &turn)?, Phase::Start);
                let since = turns
                    .iter()
                    .rev()
                    .find(|turn| turn.ended)
                    .map(|turn| checkpoint(turn, Phase::End));
                (to, since, session_dir(&mut session).await)
            };
            let plan = match self.checkpoints.revert_plan(&to, since.as_ref()).await {
                Ok(plan) => plan,
                Err(error) => {
                    return Ok(BridgeRevertPlan {
                        state: unread(error)?,
                        files: Vec::new(),
                        running: Vec::new(),
                        branch: None,
                    });
                }
            };
            let running = plan
                .running
                .iter()
                .map(|session| BridgeRunningAgent {
                    own: *session == to.session,
                    title: dir
                        .as_deref()
                        .and_then(|dir| session_title_of(dir, &self.cwd, session)),
                })
                .collect();
            Ok(BridgeRevertPlan {
                state: BridgeChangesState::Shown,
                files: plan.files.into_iter().map(Into::into).collect(),
                running,
                branch: plan.branch.as_deref().map(visible_name),
            })
        })
    }

    /// Reverts the files to before turn `turn` of the current branch and continues the session
    /// from before its prompt, which returns to the input area; the undone turns keep their
    /// branch. Refused while an agent runs in the checkout. Each error says whether the files
    /// changed.
    pub fn revert_to_turn(&self, turn: String) -> Result<(), String> {
        RUNTIME.block_on(async {
            let not_reverted = |error: String| format!("the files were not reverted: {error}");
            // Held until the branch is asked for, so a prompt sent meanwhile starts on it.
            let mut session = self.session.lock().await;
            let turns = branch_turns(&mut session).await.map_err(not_reverted)?;
            let found = turn_of(&turns, &turn).map_err(not_reverted)?;
            let prompt = found.prompt_entry.clone().ok_or_else(|| {
                not_reverted(format!("turn {turn} has no prompt to continue before"))
            })?;
            self.checkpoints
                .revert(&checkpoint(found, Phase::Start))
                .await
                .map_err(|error| match error {
                    DiffError::NotReverted(paths) => left_unreverted(&paths),
                    DiffError::PartlyReverted(error) => format!(
                        "the files may be partly reverted ({}); reverting again finishes it",
                        shown(*error)
                    ),
                    error => not_reverted(shown(error)),
                })?;
            let continued = async move {
                let pending = open(&mut session)?
                    .branch(&prompt, None)
                    .await
                    .map_err(describe)?;
                drop(session);
                pending.accepted().await.map_err(describe)?;
                // pi reports the command handled even when an extension cancelled the move, so
                // the branch is read back: the turn's prompt must be off it.
                let turns = branch_turns(&mut *self.session.lock().await).await?;
                if turns
                    .iter()
                    .any(|turn| turn.prompt_entry.as_deref() == Some(prompt.as_str()))
                {
                    return Err("pi stayed on the turn's branch".to_owned());
                }
                Ok(())
            };
            continued.await.map_err(|error| {
                format!(
                    "the files were reverted, but the session could not continue from before \
                     the turn: {error}"
                )
            })
        })
    }

    /// The sessions with checkpoints in the project's repository, sorted by their ids.
    pub fn checkpoint_sessions(&self) -> Result<Vec<BridgeCheckpointSession>, String> {
        RUNTIME.block_on(async {
            let sessions = match self.checkpoints.sessions().await {
                Ok(sessions) => sessions,
                Err(DiffError::NotARepository) => return Ok(Vec::new()),
                Err(error) => return Err(shown(error)),
            };
            let dir = session_dir(&mut *self.session.lock().await).await;
            Ok(sessions
                .into_iter()
                .map(|(id, turns)| BridgeCheckpointSession {
                    title: dir
                        .as_deref()
                        .and_then(|dir| session_title_of(dir, &self.cwd, &id))
                        .unwrap_or_else(|| "Unknown session".to_owned()),
                    turns: i64::try_from(turns).unwrap_or(i64::MAX),
                })
                .collect())
        })
    }

    /// Removes every checkpoint in the project's repository; refused while an agent runs in any
    /// of its worktrees. Outside Git there is nothing to remove.
    pub fn remove_all_checkpoints(&self) -> Result<(), String> {
        match RUNTIME.block_on(self.checkpoints.remove_all()) {
            Ok(()) | Err(DiffError::NotARepository) => Ok(()),
            Err(error) => Err(shown(error)),
        }
    }

    /// The file pi stores the open session in, unless it keeps the session in memory.
    pub fn session_file(&self) -> Result<Option<String>, String> {
        RUNTIME.block_on(async {
            let mut session = self.session.lock().await;
            let state = open(&mut session)?.state().await.map_err(describe)?;
            Ok(state.session_file.map(|file| file.display().to_string()))
        })
    }

    /// The turns with checkpoints of the session stored in `path`, the open one too; none
    /// outside Git or before pi stored the session.
    pub fn stored_turns(&self, path: String) -> Result<i64, String> {
        RUNTIME.block_on(async {
            let Some(id) = self.stored_session(&path, true).await? else {
                return Ok(0);
            };
            let sessions = match self.checkpoints.sessions().await {
                Ok(sessions) => sessions,
                Err(DiffError::NotARepository) => return Ok(0),
                Err(error) => return Err(shown(error)),
            };
            let turns = sessions
                .into_iter()
                .find(|(session, _)| *session == id)
                .map_or(0, |(_, turns)| turns);
            Ok(i64::try_from(turns).unwrap_or(i64::MAX))
        })
    }

    /// Deletes the session stored in `path`, a session of this project in pi's session
    /// directory other than the open one, with its checkpoints first; refused while its agent
    /// runs in any worktree of the repository. Neither comes back. A session pi never stored
    /// leaves nothing to delete.
    pub fn delete_session(&self, path: String) -> Result<(), String> {
        RUNTIME.block_on(async {
            let Some(id) = self.stored_session(&path, false).await? else {
                return Ok(());
            };
            match self.checkpoints.remove_session(&id).await {
                Ok(()) | Err(DiffError::NotARepository) => {}
                Err(error) => return Err(shown(error)),
            }
            std::fs::remove_file(&path).map_err(|error| error.to_string())
        })
    }

    /// The id of the session stored in `path`, which must be a session file of this project in
    /// pi's session directory, and with `open_too` false not the open session, whatever its
    /// name; none when pi has not stored it yet. So Dart never names another file.
    async fn stored_session(&self, path: &str, open_too: bool) -> Result<Option<String>, String> {
        let path = Path::new(path);
        let current = {
            let mut session = self.session.lock().await;
            open(&mut session)?
                .state()
                .await
                .map_err(describe)?
                .session_file
        };
        let Some(current) = current else {
            return Err("pi keeps this session in memory and stores no others".to_owned());
        };
        let not_open = || "the open session cannot be deleted from inside it".to_owned();
        if path == current && !open_too {
            return Err(not_open());
        }
        let refused = || "not a session file in pi's session directory".to_owned();
        if path.parent() != current.parent() {
            return Err(refused());
        }
        // A pipe or device would block the read; pi writes plain `.jsonl` files.
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if !meta.is_file()
            || path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
        {
            return Err(refused());
        }
        let id =
            session_id(path, &self.cwd).ok_or_else(|| "no session of this project".to_owned())?;
        if !open_too && session_id(&current, &self.cwd).as_deref() == Some(id.as_str()) {
            return Err(not_open());
        }
        Ok(Some(id))
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
            let pending = open(&mut *self.session.lock().await)?
                .new_session()
                .await
                .map_err(describe)?;
            // Waits without holding the session.
            pending.switched().await.map_err(describe)
        })
    }

    /// Continues the session stored in `path`; `false` when an extension cancelled it.
    pub fn switch_session(&self, path: String) -> Result<bool, String> {
        RUNTIME.block_on(async {
            let pending = open(&mut *self.session.lock().await)?
                .switch_session(Path::new(&path))
                .await
                .map_err(describe)?;
            pending.switched().await.map_err(describe)
        })
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

    /// Removes the messages waiting for the running agent and returns them.
    pub async fn clear_queue(&self) -> Result<BridgeQueue, String> {
        Ok(open(&mut *self.session.lock().await)?
            .clear_queue()
            .await
            .map_err(describe)?
            .into())
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
        self.checkpoints.close().await;
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

/// Whether a diff hides changes that are only whitespace.
fn whitespace(hide: bool) -> Whitespace {
    if hide {
        Whitespace::Hidden
    } else {
        Whitespace::Shown
    }
}

/// The turns of the current branch.
async fn branch_turns(session: &mut Option<Session>) -> Result<Vec<TurnCheckpoints>, String> {
    Ok(open(session)?
        .history()
        .await
        .map_err(describe)?
        .turns(None))
}

/// pi's session directory, unless pi keeps the session in memory.
async fn session_dir(session: &mut Option<Session>) -> Option<PathBuf> {
    let file = open(session).ok()?.state().await.ok()?.session_file?;
    file.parent().map(Path::to_owned)
}

/// Turn `id` among `turns`.
fn turn_of<'a>(turns: &'a [TurnCheckpoints], id: &str) -> Result<&'a TurnCheckpoints, String> {
    turns
        .iter()
        .find(|turn| turn.turn == id)
        .ok_or_else(|| format!("no turn {id} on the current branch"))
}

/// `turn`'s checkpoint at `phase`.
fn checkpoint(turn: &TurnCheckpoints, phase: Phase) -> Checkpoint {
    Checkpoint {
        session: turn.session.clone(),
        turn: turn.turn.clone(),
        phase,
    }
}

/// Says which files Git left unreverted, at most a few of them, as Liquid shows file names.
pub(crate) fn left_unreverted(paths: &[String]) -> String {
    const NAMED: usize = 3;
    let mut named: Vec<String> = paths
        .iter()
        .take(NAMED)
        .map(|path| visible_name(path))
        .collect();
    if paths.len() > NAMED {
        named.push(format!("{} more", paths.len() - NAMED));
    }
    format!(
        "the other files were reverted and the session stays on its branch; Git could not \
         revert {}",
        named.join(", ")
    )
}

/// A checkpoint error as Liquid shows it: Git's messages name files, so what could hide or
/// reorder them is written out, as in file names.
fn shown(error: DiffError) -> String {
    visible_name(&error.to_string())
}

/// Why checkpoints could not be read, when they cannot exist here or were removed.
pub(crate) fn unread(error: DiffError) -> Result<BridgeChangesState, String> {
    match error {
        DiffError::NotARepository => Ok(BridgeChangesState::OutsideGit),
        DiffError::NoCheckpoint => Ok(BridgeChangesState::Removed),
        error => Err(shown(error)),
    }
}
