//! Loop Agent M1 deterministic runtime.

#![deny(missing_docs)]

use core_policy::{
    protected_path_match_mode_for_policy_target, protected_path_pattern_matches,
    ProtectedPathMatchMode,
};
use proto::{EventEnvelope, EventType};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Workspace-relative directory containing persisted session JSONL logs.
pub const LOCAL_SESSION_DIR: &str = ".loop/sessions";
/// Workspace-relative directory containing structured sidecar run logs.
pub const LOCAL_LOG_DIR: &str = ".loop/logs";
/// Maximum bytes accepted for one persisted session log.
pub const MAX_SESSION_LOG_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum canonical JSONL bytes emitted by one loop run.
pub const MAX_LOOP_EVENT_STREAM_BYTES: usize = 10 * 1024 * 1024;
/// Maximum events emitted by one loop run.
pub const MAX_LOOP_EVENTS: u64 = 64 * 1024;
/// Maximum loop invocations executed by one loop run.
pub const MAX_LOOP_INVOCATIONS: u64 = 8 * 1024;
const MAX_WORKSPACE_CONFIG_BYTES: u64 = core_script::MAX_REGISTRY_FILE_BYTES;
const TAIL_TRANSIENT_READ_RETRY_ATTEMPTS: usize = 200;
const TAIL_TRANSIENT_READ_RETRY_MS: u64 = 5;
const FIXTURE_CLOCK_UNIX_SECONDS: i64 = 1_767_225_600;
const RUNTIME_ERROR_REASON: &str = "runtime_error";
const TRUSTED_PREDEFINED_COMMANDS: &[TrustedPredefinedCommand] = &[
    TrustedPredefinedCommand {
        command_id: "agent-echo",
        progress: None,
    },
    TrustedPredefinedCommand {
        command_id: "agent-negative",
        progress: None,
    },
    TrustedPredefinedCommand {
        command_id: "agent-read",
        progress: Some("stub read completed"),
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TrustedPredefinedCommand {
    command_id: &'static str,
    progress: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EventClock {
    base_unix_seconds: i64,
}

impl EventClock {
    fn fixed_fixture() -> Self {
        Self {
            base_unix_seconds: FIXTURE_CLOCK_UNIX_SECONDS,
        }
    }

    fn wall_clock() -> Self {
        let base_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self { base_unix_seconds }
    }

    fn from_first_event(event: &EventEnvelope) -> Option<Self> {
        parse_rfc3339_utc_timestamp(&event.timestamp).map(|base_unix_seconds| Self {
            base_unix_seconds: base_unix_seconds.saturating_sub(
                i64::try_from(event.sequence.saturating_sub(1)).unwrap_or(i64::MAX),
            ),
        })
    }

    fn timestamp(self, sequence: u64) -> String {
        let offset = i64::try_from(sequence.saturating_sub(1)).unwrap_or(i64::MAX);
        format_unix_timestamp(self.base_unix_seconds.saturating_add(offset))
    }
}

/// Runtime surfaces tracked by the Loop Agent MVP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSurface {
    /// Human CLI commands.
    HumanCli,
    /// Headless JSONL event stream.
    JsonlEventStream,
    /// Local append-only session log.
    LocalSessionLog,
    /// Session tail, replay and resume commands.
    TailReplayResume,
    /// Designed future RPC surface.
    DesignedRpc,
    /// Designed future embedded core API.
    FutureEmbeddedCoreApi,
}

/// Returns the runtime surfaces implemented in M1.
pub fn m1_runtime_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::HumanCli,
        RuntimeSurface::JsonlEventStream,
        RuntimeSurface::LocalSessionLog,
        RuntimeSurface::TailReplayResume,
    ]
}

/// Returns runtime surfaces intentionally deferred beyond M1.
pub fn designed_future_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::DesignedRpc,
        RuntimeSurface::FutureEmbeddedCoreApi,
    ]
}

/// Output format for CLI/runtime calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitMode {
    /// Human-readable one-line status output.
    Human,
    /// Canonical JSONL event stream output.
    Jsonl,
}

/// Result of a run, replay, tail or resume operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutput {
    /// Number of events represented by this output.
    pub event_count: usize,
    /// Whether the represented session ended or is known to be failed.
    pub failed: bool,
    /// Session id.
    pub session_id: String,
    /// Path to the persisted session log.
    pub session_path: PathBuf,
    /// Captured stdout for non-streaming callers.
    pub stdout: String,
}

/// Tail behavior options.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TailOptions {
    /// Whether to wait for appended events until a terminal event or timeout.
    pub follow: bool,
    /// Optional maximum follow duration.
    pub timeout: Option<Duration>,
}

impl TailOptions {
    /// Follows until the session reaches a terminal event.
    pub fn follow() -> Self {
        Self {
            follow: true,
            timeout: None,
        }
    }

    /// Reads the current complete prefix and exits immediately.
    pub fn no_follow() -> Self {
        Self {
            follow: false,
            timeout: None,
        }
    }
}

/// Error returned by Loop Agent runtime operations.
#[derive(Debug)]
pub enum RuntimeError {
    /// Filesystem I/O failed.
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// JSON parsing or serialization failed.
    Json(serde_json::Error),
    /// Policy compilation failed.
    Policy(core_policy::PolicyCompileError),
    /// Registry loading or validation failed.
    Registry(core_script::RegistryError),
    /// Runtime enforcement denied a side effect.
    Denied {
        /// Structured denial reason.
        reason: core_policy::DenyReasonCode,
        /// Human-readable denial message.
        message: String,
    },
    /// Runtime protocol invariant was violated.
    Protocol(String),
    /// A session lock already exists for the requested session.
    ActiveSession {
        /// Requested session id.
        session_id: String,
        /// Existing lock path.
        lock_path: PathBuf,
    },
    /// A requested new session log already exists.
    SessionLogExists(String),
    /// Resume was requested for a terminal session.
    TerminalSession(String),
    /// CLI/user input was invalid.
    Usage(String),
}

impl RuntimeError {
    /// Returns the process exit code associated with this runtime error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Denied { .. }
            | Self::Protocol(_)
            | Self::ActiveSession { .. }
            | Self::SessionLogExists(_)
            | Self::TerminalSession(_) => 65,
            Self::Usage(_) => 64,
            Self::Io { .. } | Self::Json(_) | Self::Policy(_) | Self::Registry(_) => 65,
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Json(err) => write!(f, "{err}"),
            Self::Policy(err) => write!(f, "{err}"),
            Self::Registry(err) => write!(f, "{err}"),
            Self::Denied { message, .. } => f.write_str(message),
            Self::Protocol(message) | Self::Usage(message) => f.write_str(message),
            Self::ActiveSession {
                session_id,
                lock_path,
            } => f.write_str(&active_session_lock_message(lock_path, session_id)),
            Self::SessionLogExists(session_id) => {
                write!(f, "session log already exists for {session_id}")
            }
            Self::TerminalSession(session_id) => {
                write!(f, "cannot resume terminal session {session_id}")
            }
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(err) => Some(err),
            Self::Policy(err) => Some(err),
            Self::Registry(err) => Some(err),
            Self::Denied { .. }
            | Self::Protocol(_)
            | Self::ActiveSession { .. }
            | Self::SessionLogExists(_)
            | Self::TerminalSession(_)
            | Self::Usage(_) => None,
        }
    }
}

impl From<core_script::RegistryError> for RuntimeError {
    fn from(err: core_script::RegistryError) -> Self {
        Self::Registry(err)
    }
}

impl From<core_policy::PolicyCompileError> for RuntimeError {
    fn from(err: core_policy::PolicyCompileError) -> Self {
        Self::Policy(err)
    }
}

impl From<serde_json::Error> for RuntimeError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

/// Returns whether `session_id` is a valid path-safe v0 session id.
pub fn validate_session_id(session_id: &str) -> bool {
    proto::is_valid_session_id(session_id)
}

/// Returns the current runtime enforcement notice used by docs/tests.
pub fn m0_runtime_notice() -> &'static str {
    "M1 runs deterministic in-process Loop Agent execution; OS sandbox enforcement is post-M1"
}

/// Runs a loop from a workspace registry and persists a new session log.
pub fn run_loop(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let config = load_workspace_config(workspace)?;
    let registry_path = registry_root_path(workspace, &config.registry_root)?;
    let registry = core_script::load_registry_root(registry_path)?;
    let loop_block = registry
        .loop_block(loop_ref)
        .ok_or_else(|| RuntimeError::Usage(format!("unknown loop {loop_ref}")))?;
    let definition_hashes = session_definition_hashes(&registry, loop_block)?;
    let artifacts =
        core_policy::compile_policy_artifacts(&loop_block.identity.id, &registry, loop_ref)?;
    let policy = runtime_policy_artifact(&artifacts)?;
    preflight_loop_tools(workspace, &registry, policy, loop_block)?;
    let base_session_id = session_id_for_loop(&loop_block.identity.id);
    let reservation = reserve_unique_session_log(workspace, &base_session_id)?;
    let expected_session_id = reservation.session_id.clone();
    if let Err(err) =
        write_initial_session_log_with_clock(&reservation, &expected_session_id, config.event_clock)
    {
        reservation.rollback();
        return Err(err);
    }
    if let Err(err) = write_reserved_session_metadata(
        &reservation,
        &expected_session_id,
        1,
        Some(&definition_hashes),
    ) {
        reservation.rollback();
        return Err(err);
    }
    let planned_runtime = match execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        &expected_session_id,
        LoopExecutionOptions::new(
            config.event_clock,
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
        ),
    ) {
        Ok(runtime) => runtime,
        Err(err) => {
            reservation.rollback();
            return Err(err);
        }
    };
    let (planned_stream, planned_events) = match preflight_session_completion_stream(
        &reservation,
        &expected_session_id,
        &planned_runtime.events,
    ) {
        Ok(planned) => planned,
        Err(err) => {
            reservation.rollback();
            return Err(err);
        }
    };
    let durable_prefix_event_count = durable_run_prefix_event_count(&planned_events);
    if let Err(err) = persist_reserved_session_prefix(
        &reservation,
        &expected_session_id,
        &planned_events,
        durable_prefix_event_count,
        Some(&definition_hashes),
    ) {
        reservation.rollback();
        return Err(err);
    }
    let result = (|| {
        let session_id = planned_events
            .first()
            .expect("validated streams contain at least one event")
            .session_id
            .clone();
        let runtime = execute_loop(
            workspace,
            &registry,
            policy,
            loop_block,
            &expected_session_id,
            LoopExecutionOptions::new(
                config.event_clock,
                ToolSideEffectMode::ApplyAll,
                SideEffectRecorder::for_reservation(&reservation),
            ),
        )?;
        reservation.mark_side_effects_applied();
        let runtime_failed = runtime.failed;
        let terminal_error = runtime.terminal_error;
        if !runtime_failed && runtime.events != planned_runtime.events {
            return Err(RuntimeError::Protocol(format!(
                "{} runtime did not match deterministic replay",
                reservation.session_path.display()
            )));
        }
        let (final_stream, final_events) = if runtime_failed {
            preflight_session_completion_stream_from_prefix(
                &reservation,
                &expected_session_id,
                &runtime.events,
                durable_prefix_event_count,
            )?
        } else {
            (planned_stream, planned_events)
        };
        commit_reserved_session_log_from_prefix(
            &reservation,
            &session_id,
            &final_stream,
            final_events.len(),
            Some(&definition_hashes),
            durable_prefix_event_count,
        )?;
        reservation.release_lock()?;
        if let Some(err) = terminal_error {
            return Err(err);
        }

        Ok(RunOutput {
            event_count: final_events.len(),
            failed: runtime_failed,
            session_id,
            session_path: reservation.session_path.clone(),
            stdout: match emit {
                EmitMode::Jsonl => final_stream,
                EmitMode::Human if runtime_failed => {
                    format!("loop {} failed\n", loop_block.identity.id)
                }
                EmitMode::Human => format!("loop {} completed\n", loop_block.identity.id),
            },
        })
    })();
    if result.is_err() {
        reservation.rollback();
    }
    result
}

/// Replays a persisted terminal or partial session log without modifying it.
pub fn replay_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    read_existing_session(workspace.as_ref(), session_id, emit)
}

/// Tails a session log and captures output in the returned [`RunOutput`].
pub fn tail_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let mut stdout = Vec::new();
    let mut output = tail_session_to_writer(workspace, session_id, emit, &mut stdout)?;
    output.stdout = String::from_utf8(stdout)
        .map_err(|err| RuntimeError::Protocol(format!("tail output was not valid UTF-8: {err}")))?;
    Ok(output)
}

/// Tails a session log to a caller-provided writer with default follow behavior.
pub fn tail_session_to_writer(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    writer: &mut impl Write,
) -> Result<RunOutput, RuntimeError> {
    tail_session_to_writer_with_options(workspace, session_id, emit, TailOptions::follow(), writer)
}

/// Tails a session log to a caller-provided writer with explicit options.
pub fn tail_session_to_writer_with_options(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
    writer: &mut impl Write,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let path = session_path(workspace, session_id)?;
    ensure_existing_session_log_path(workspace, &path)?;
    let initial = read_session_log_to_string(&path)?;
    let mut stream = complete_jsonl_prefix(&initial).to_owned();
    let mut events = if stream.is_empty() {
        Vec::new()
    } else {
        validate_session_log_text(&path, session_id, &stream)?
    };
    let mut append_state = if events.is_empty() {
        None
    } else {
        Some(SessionAppendValidationState::from_prior_events(
            &path,
            session_id,
            &events,
            stream.len(),
        )?)
    };
    let mut pending = initial[stream.len()..].to_owned();
    let mut observed_len = initial.len();
    if initial.len() > stream.len() && (stream_is_failed(&events) || stream_is_completed(&events)) {
        return Err(RuntimeError::Protocol(format!(
            "{} contains a partial line after a terminal event",
            path.display()
        )));
    }
    if (!stream.is_empty() || emit == EmitMode::Jsonl)
        && !write_tail_chunk(writer, emit, session_id, &stream)?
    {
        return Ok(RunOutput {
            event_count: events.len(),
            failed: stream_is_failed(&events),
            session_id: session_id.to_owned(),
            session_path: path,
            stdout: String::new(),
        });
    }

    let started = Instant::now();
    while !stream_is_failed(&events) && !stream_is_completed(&events) {
        if !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            break;
        }
        thread::sleep(tail_poll_interval(&options, started));
        let current_len = tail_session_log_len(&path)?;
        if current_len < observed_len {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside append-only tail semantics",
                path.display()
            )));
        }
        if current_len == observed_len {
            continue;
        }
        let suffix = read_tail_file_suffix_to_string(&path, observed_len, current_len)?;
        observed_len = current_len;
        pending.push_str(&suffix);
        if !pending.ends_with('\n') {
            continue;
        }
        let appended = std::mem::take(&mut pending);
        let appended_events = if let Some(state) = &mut append_state {
            state.validate_appended(&path, &appended)?
        } else {
            let appended_events = validate_session_log_text(&path, session_id, &appended)?;
            append_state = Some(SessionAppendValidationState::from_prior_events(
                &path,
                session_id,
                &appended_events,
                appended.len(),
            )?);
            appended_events
        };
        events.extend(appended_events);
        if !write_tail_chunk(writer, emit, session_id, &appended)? {
            return Ok(RunOutput {
                event_count: events.len(),
                failed: stream_is_failed(&events),
                session_id: session_id.to_owned(),
                session_path: path,
                stdout: String::new(),
            });
        }
        stream.push_str(&appended);
    }

    if emit == EmitMode::Human && !write_tail_chunk(writer, emit, session_id, "")? {
        return Ok(RunOutput {
            event_count: events.len(),
            failed: stream_is_failed(&events),
            session_id: session_id.to_owned(),
            session_path: path,
            stdout: String::new(),
        });
    }

    Ok(RunOutput {
        event_count: events.len(),
        failed: stream_is_failed(&events),
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: String::new(),
    })
}

fn tail_poll_interval(options: &TailOptions, started: Instant) -> Duration {
    let default = Duration::from_millis(25);
    options.timeout.map_or(default, |timeout| {
        timeout.saturating_sub(started.elapsed()).min(default)
    })
}

fn complete_jsonl_prefix(text: &str) -> &str {
    text.rfind('\n')
        .map_or("", |newline_index| &text[..=newline_index])
}

/// Lists valid persisted session ids in canonical order.
pub fn list_sessions(workspace: impl AsRef<Path>) -> Result<Vec<String>, RuntimeError> {
    let workspace = workspace.as_ref();
    let loop_dir = workspace.join(".loop");
    if !ensure_optional_real_directory(&loop_dir)? {
        return Ok(Vec::new());
    }
    let dir = workspace.join(LOCAL_SESSION_DIR);
    if !ensure_optional_real_directory(&dir)? {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|source| RuntimeError::Io {
        path: dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| RuntimeError::Io {
            path: dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if validate_session_id(stem) {
            sessions.push(stem.to_owned());
        }
    }
    sessions.sort();
    Ok(sessions)
}

/// Resumes a non-terminal persisted session after validating registry drift.
pub fn resume_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let path = session_path(workspace, session_id)?;
    ensure_existing_session_log_path(workspace, &path)?;
    ensure_non_hardlinked_real_file(&path)?;
    let _lock = acquire_session_lock(workspace, session_id)?;
    let before = read_session_log_to_string(&path)?;
    let events = validate_session_log_text(&path, session_id, &before)?;
    if stream_is_failed(&events) || stream_is_completed(&events) {
        return Err(RuntimeError::TerminalSession(session_id.to_owned()));
    }
    ensure_resume_has_durable_loop_progress(&path, session_id, &events)?;

    let config = load_workspace_config(workspace)?;
    let registry_path = registry_root_path(workspace, &config.registry_root)?;
    let registry = core_script::load_registry_root(registry_path)?;
    let loop_id = resumable_loop_id(&events, &registry, session_id)?;
    let loop_block = registry.loop_block(&loop_id).ok_or_else(|| {
        RuntimeError::Protocol(format!("resolved registry missing loop {loop_id}"))
    })?;
    verify_resume_definition_metadata(workspace, session_id, &registry, loop_block)?;
    let artifacts =
        core_policy::compile_policy_artifacts(&loop_block.identity.id, &registry, &loop_id)?;
    let policy = runtime_policy_artifact(&artifacts)?;
    let planned_runtime = execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        session_id,
        LoopExecutionOptions::new(
            resume_event_clock(&config, &events)?,
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
        ),
    )?;
    let resume_prefix = validate_resume_replay_prefix(
        &path,
        &events,
        &planned_runtime.events,
        loop_block,
        resume_event_clock(&config, &events)?,
    )?;
    if let Some(tool_id) = started_tool_without_progress(&events) {
        return Err(RuntimeError::Protocol(format!(
            "cannot resume session {session_id} with in-flight tool {tool_id:?} before progress or terminal event"
        )));
    }
    let preflight_runtime = execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        session_id,
        LoopExecutionOptions::new(
            resume_event_clock(&config, &events)?,
            ToolSideEffectMode::PreflightResume {
                prefix_event_count: resume_prefix.planned_event_count as u64,
            },
            SideEffectRecorder::none(),
        ),
    )?;
    if preflight_runtime.events != planned_runtime.events {
        return Err(RuntimeError::Protocol(format!(
            "{} resume preflight did not match deterministic replay",
            path.display()
        )));
    }
    let append_plan = preflight_resume_append_plan(
        &path,
        session_id,
        &before,
        &events,
        &planned_runtime.events,
        &resume_prefix,
        resume_event_clock(&config, &events)?,
    )?;

    // WHY: resume side effects need a durable attempt marker before they run, while success
    // events are only appended after the resumed side-effect replay matches the dry run.
    append_session_log_text(&path, &append_plan.marker_stream)?;

    let resumed_runtime = execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        session_id,
        LoopExecutionOptions::new(
            resume_event_clock(&config, &events)?,
            ToolSideEffectMode::Resume {
                prefix_event_count: resume_prefix.planned_event_count as u64,
            },
            SideEffectRecorder::none(),
        ),
    )?;
    let terminal_error = resumed_runtime.terminal_error;
    let resumed_failed = resumed_runtime.failed;
    if terminal_error.is_none() && resumed_runtime.events != planned_runtime.events {
        return Err(RuntimeError::Protocol(format!(
            "{} resumed runtime did not match deterministic replay",
            path.display()
        )));
    }

    let (suffix_stream, combined_events) = if terminal_error.is_some() {
        preflight_resume_runtime_suffix(
            &path,
            session_id,
            &before,
            &append_plan.marker_stream,
            &resumed_runtime.events,
            &resume_prefix,
            resume_event_clock(&config, &events)?,
        )?
    } else {
        (append_plan.suffix_stream, append_plan.combined_events)
    };

    append_session_log_text(&path, &suffix_stream)?;
    let appended_stream = format!("{}{}", append_plan.marker_stream, suffix_stream);
    if let Some(err) = terminal_error {
        return Err(err);
    }

    Ok(RunOutput {
        event_count: combined_events.len(),
        failed: resumed_failed,
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: match emit {
            EmitMode::Jsonl => appended_stream,
            EmitMode::Human => format!("session {session_id} resumed\n"),
        },
    })
}

struct ResumeReplayPrefix {
    planned_event_count: usize,
    resume_marker_count: usize,
}

struct ResumeAppendPlan {
    marker_stream: String,
    suffix_stream: String,
    combined_events: Vec<EventEnvelope>,
}

fn validate_resume_replay_prefix(
    path: &Path,
    events: &[EventEnvelope],
    planned_events: &[EventEnvelope],
    loop_block: &core_script::LoopBlock,
    clock: EventClock,
) -> Result<ResumeReplayPrefix, RuntimeError> {
    let mut planned_event_count = 0usize;
    let mut resume_marker_count = 0usize;

    for event in events {
        if event.event_type == EventType::SessionResumed {
            resume_marker_count += 1;
            continue;
        }

        let Some(planned_event) = planned_events.get(planned_event_count) else {
            return Err(invalid_resume_prefix_error(path, loop_block));
        };
        let expected_event =
            shift_resumed_event(planned_event.clone(), resume_marker_count as u64, clock);
        if event != &expected_event {
            return Err(invalid_resume_prefix_error(path, loop_block));
        }
        planned_event_count += 1;
    }

    if matches!(events.last(), Some(event) if event.event_type == EventType::SessionResumed) {
        return Err(incomplete_resume_marker_error(path, loop_block));
    }

    Ok(ResumeReplayPrefix {
        planned_event_count,
        resume_marker_count,
    })
}

fn invalid_resume_prefix_error(path: &Path, loop_block: &core_script::LoopBlock) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} is not a valid prefix of loop {}",
        path.display(),
        loop_block.identity.id
    ))
}

fn incomplete_resume_marker_error(
    path: &Path,
    loop_block: &core_script::LoopBlock,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} has incomplete resume marker for loop {}",
        path.display(),
        loop_block.identity.id
    ))
}

fn ensure_resume_has_durable_loop_progress(
    path: &Path,
    session_id: &str,
    events: &[EventEnvelope],
) -> Result<(), RuntimeError> {
    if events
        .iter()
        .any(|event| event.event_type == EventType::LoopStarted && event.parent_loop_id.is_none())
    {
        return Ok(());
    }

    Err(RuntimeError::Protocol(format!(
        "{} cannot resume session {session_id} before durable loop progress",
        path.display()
    )))
}

fn preflight_resume_append_plan(
    path: &Path,
    session_id: &str,
    before: &str,
    events: &[EventEnvelope],
    planned_events: &[EventEnvelope],
    resume_prefix: &ResumeReplayPrefix,
    clock: EventClock,
) -> Result<ResumeAppendPlan, RuntimeError> {
    let sequence = events
        .last()
        .expect("validated streams contain at least one event")
        .sequence
        + 1;
    let resume_event = EventEnvelope::new(
        next_event_id(sequence, events),
        EventType::SessionResumed,
        session_id.to_owned(),
        sequence,
        clock.timestamp(sequence),
        "loop-agent-cli",
        serde_json::json!({"reason":"resume"}),
    );
    let resumed_suffix_offset = resume_prefix.resume_marker_count as u64 + 1;
    let suffix_events = planned_events[resume_prefix.planned_event_count..]
        .iter()
        .cloned()
        .map(|event| shift_resumed_event(event, resumed_suffix_offset, clock))
        .collect::<Vec<_>>();
    let marker_stream = canonical_event_stream(std::slice::from_ref(&resume_event))?;
    let suffix_stream = canonical_event_stream(&suffix_events)?;
    let appended_stream = format!("{marker_stream}{suffix_stream}");
    let marker_combined = format!("{before}{marker_stream}");
    validate_session_log_text(path, session_id, &marker_combined)?;
    let combined = format!("{before}{appended_stream}");
    let combined_events = validate_session_log_text(path, session_id, &combined)?;
    prepare_session_log_append(path, &appended_stream)?;
    Ok(ResumeAppendPlan {
        marker_stream,
        suffix_stream,
        combined_events,
    })
}

fn preflight_resume_runtime_suffix(
    path: &Path,
    session_id: &str,
    before: &str,
    marker_stream: &str,
    runtime_events: &[EventEnvelope],
    resume_prefix: &ResumeReplayPrefix,
    clock: EventClock,
) -> Result<(String, Vec<EventEnvelope>), RuntimeError> {
    let resumed_suffix_offset = resume_prefix.resume_marker_count as u64 + 1;
    let suffix_events = runtime_events[resume_prefix.planned_event_count..]
        .iter()
        .cloned()
        .map(|event| shift_resumed_event(event, resumed_suffix_offset, clock))
        .collect::<Vec<_>>();
    let suffix_stream = canonical_event_stream(&suffix_events)?;
    let combined = format!("{before}{marker_stream}{suffix_stream}");
    let combined_events = validate_session_log_text(path, session_id, &combined)?;
    prepare_session_log_append(path, &suffix_stream)?;
    Ok((suffix_stream, combined_events))
}

fn append_session_log_text(path: &Path, text: &str) -> Result<(), RuntimeError> {
    append_session_log_bytes(path, text.as_bytes())
}

fn prepare_session_log_append(path: &Path, text: &str) -> Result<(), RuntimeError> {
    ensure_session_log_growth_within_limit(path, text.len())?;
    append_existing_file(path, b"")
}

fn append_session_log_bytes(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_session_log_growth_within_limit(path, contents.len())?;
    append_existing_file(path, contents)
}

fn ensure_session_log_growth_within_limit(
    path: &Path,
    appended_bytes: usize,
) -> Result<(), RuntimeError> {
    let existing_bytes = u64::try_from(session_log_len(path)?).unwrap_or(u64::MAX);
    let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
    let total = existing_bytes.saturating_add(appended_bytes);
    if total > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} session log size {total} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    Ok(())
}

fn shift_resumed_event(
    mut event: EventEnvelope,
    sequence_offset: u64,
    clock: EventClock,
) -> EventEnvelope {
    event.sequence += sequence_offset;
    event.event_id = format!("evt-{:03}", event.sequence);
    event.timestamp = clock.timestamp(event.sequence);
    event
}

fn resumable_loop_id(
    events: &[EventEnvelope],
    registry: &core_script::ResolvedRegistry,
    session_id: &str,
) -> Result<String, RuntimeError> {
    if let Some(event) = events
        .iter()
        .find(|event| event.event_type == EventType::LoopStarted && event.parent_loop_id.is_none())
    {
        return event
            .payload
            .get("loop_definition_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                RuntimeError::Protocol("loop.started missing loop_definition_id".to_owned())
            });
    }

    let matches = registry
        .loops
        .values()
        .filter(|loop_block| session_id_matches_loop(session_id, &loop_block.identity.id))
        .map(|loop_block| loop_block.identity.id.clone())
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [loop_id] => Ok(loop_id.clone()),
        [] => Err(RuntimeError::Protocol(format!(
            "session {session_id} does not identify a resumable loop"
        ))),
        _ => Err(RuntimeError::Protocol(format!(
            "session {session_id} ambiguously identifies a resumable loop"
        ))),
    }
}

fn session_id_matches_loop(session_id: &str, loop_id: &str) -> bool {
    let base = session_id_for_loop(loop_id);
    if session_id == base {
        return true;
    }
    let Some((_, suffix)) = session_id.rsplit_once('-') else {
        return false;
    };
    let Ok(ordinal) = suffix.parse::<u32>() else {
        return false;
    };
    (2..=10_000).contains(&ordinal) && suffixed_session_id(&base, ordinal) == session_id
}

fn read_existing_session(
    workspace: &Path,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let path = session_path(workspace, session_id)?;
    ensure_existing_session_log_path(workspace, &path)?;
    let stream = read_session_log_to_string(&path)?;
    let events = validate_session_log_text(&path, session_id, &stream)?;
    Ok(RunOutput {
        event_count: events.len(),
        failed: stream_is_failed(&events),
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: match emit {
            EmitMode::Jsonl => stream,
            EmitMode::Human => format!("session {session_id} replayed\n"),
        },
    })
}

fn write_tail_chunk(
    writer: &mut impl Write,
    emit: EmitMode,
    session_id: &str,
    jsonl: &str,
) -> Result<bool, RuntimeError> {
    match emit {
        EmitMode::Jsonl => write_tail_bytes(writer, jsonl.as_bytes()),
        EmitMode::Human => {
            if jsonl.is_empty() {
                return write_tail_bytes(
                    writer,
                    format!("session {session_id} tailed\n").as_bytes(),
                );
            }
            Ok(true)
        }
    }
}

fn write_tail_bytes(writer: &mut impl Write, bytes: &[u8]) -> Result<bool, RuntimeError> {
    match writer.write_all(bytes).and_then(|_| writer.flush()) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: PathBuf::from("<tail>"),
            source,
        }),
    }
}

fn session_path(workspace: &Path, session_id: &str) -> Result<PathBuf, RuntimeError> {
    if !validate_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    Ok(workspace
        .join(LOCAL_SESSION_DIR)
        .join(format!("{session_id}.jsonl")))
}

#[derive(Debug)]
struct SessionReservation {
    log_path: PathBuf,
    lock_path: PathBuf,
    session_path: PathBuf,
    session_id: String,
    cleanup_on_drop: Cell<bool>,
    committed: Cell<bool>,
    side_effects_applied: Cell<bool>,
}

impl SessionReservation {
    fn rollback(&self) {
        // WHY: committed JSONL streams are durable audit records, and once side effects
        // have applied, even an incomplete started stream ties workspace mutation to a
        // session attempt.
        if !self.committed.get() && !self.side_effects_applied.get() {
            let _ = fs::remove_file(&self.session_path);
            let _ = fs::remove_file(&self.log_path);
        }
        let _ = fs::remove_file(&self.lock_path);
        self.cleanup_on_drop.set(false);
    }

    fn release_lock(&self) -> Result<(), RuntimeError> {
        fs::remove_file(&self.lock_path).map_err(|source| RuntimeError::Io {
            path: self.lock_path.clone(),
            source,
        })?;
        self.cleanup_on_drop.set(false);
        Ok(())
    }

    fn mark_committed(&self) {
        self.committed.set(true);
    }

    fn mark_side_effects_applied(&self) {
        self.side_effects_applied.set(true);
    }
}

impl Drop for SessionReservation {
    fn drop(&mut self) {
        if self.cleanup_on_drop.get() {
            self.rollback();
        }
    }
}

struct SessionLockGuard {
    path: PathBuf,
}

impl Drop for SessionLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionDefinitionHashes {
    registry_hash: String,
    loop_definition_hash: String,
}

#[derive(Default, Debug, Eq, PartialEq)]
struct SessionLogMetadata {
    registry_hash: Option<String>,
    loop_definition_hash: Option<String>,
}

fn session_definition_hashes(
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
) -> Result<SessionDefinitionHashes, RuntimeError> {
    let registry_json = registry.canonical_json()?;
    let loop_json = proto::canonical_json(&serde_json::to_value(loop_block)?).map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize loop definition hash: {err}"))
    })?;
    Ok(SessionDefinitionHashes {
        registry_hash: stable_hash_text(registry_json.as_bytes()),
        loop_definition_hash: stable_hash_text(loop_json.as_bytes()),
    })
}

fn stable_hash_text(bytes: &[u8]) -> String {
    format!("fnv64:{:016x}", stable_hash64(bytes))
}

fn verify_resume_definition_metadata(
    workspace: &Path,
    session_id: &str,
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
) -> Result<(), RuntimeError> {
    // WHY: resume hashes bind a partial session to the registry definitions that produced
    // it; incomplete metadata cannot prove the prefix matches the current registry.
    let Some(metadata) = read_session_log_metadata(workspace, session_id)? else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing definition metadata"
        )));
    };
    let Some(recorded_registry_hash) = metadata.registry_hash else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing registry_hash metadata"
        )));
    };
    let Some(recorded_loop_definition_hash) = metadata.loop_definition_hash else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing loop_definition_hash metadata"
        )));
    };

    let expected = session_definition_hashes(registry, loop_block)?;
    if recorded_registry_hash != expected.registry_hash
        || recorded_loop_definition_hash != expected.loop_definition_hash
    {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: recorded definition metadata does not match current registry"
        )));
    }
    Ok(())
}

fn read_session_log_metadata(
    workspace: &Path,
    session_id: &str,
) -> Result<Option<SessionLogMetadata>, RuntimeError> {
    let path = session_log_metadata_path(workspace, session_id)?;
    let log_dir = path.parent().ok_or_else(|| {
        RuntimeError::Protocol(format!("{} must have a parent directory", path.display()))
    })?;
    if !ensure_optional_real_directory(log_dir)? {
        return Ok(None);
    }
    match fs::symlink_metadata(&path) {
        Ok(metadata) => validate_real_file(&path, &metadata)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RuntimeError::Io { path, source });
        }
    }
    parse_session_log_metadata(&read_to_string_with_limit(&path, MAX_SESSION_LOG_BYTES)?).map(Some)
}

fn parse_session_log_metadata(text: &str) -> Result<SessionLogMetadata, RuntimeError> {
    let mut metadata = SessionLogMetadata::default();
    for (line_number, line) in text.lines().enumerate() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(RuntimeError::Protocol(format!(
                "session metadata line {} is not key=value",
                line_number + 1
            )));
        };
        match key {
            "registry_hash" => metadata.registry_hash = Some(value.to_owned()),
            "loop_definition_hash" => metadata.loop_definition_hash = Some(value.to_owned()),
            "session_id" | "events" => {}
            _ => {}
        }
    }
    Ok(metadata)
}

fn session_log_metadata_path(workspace: &Path, session_id: &str) -> Result<PathBuf, RuntimeError> {
    if !validate_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    Ok(workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{session_id}.log")))
}

fn reserve_session_log(
    workspace: &Path,
    session_id: &str,
) -> Result<SessionReservation, RuntimeError> {
    let (session_dir, log_dir) = ensure_runtime_dirs(workspace)?;
    let session_path = session_dir.join(format!("{session_id}.jsonl"));
    let log_path = log_dir.join(format!("{session_id}.log"));
    let lock_path = session_lock_path(workspace, session_id)?;
    reserve_session_file(&session_path, session_id)?;
    if let Err(err) = reserve_session_lock_file(&lock_path, session_id) {
        let _ = fs::remove_file(&session_path);
        return Err(err);
    }
    if let Err(err) = reserve_new_file(&log_path) {
        let _ = fs::remove_file(&session_path);
        let _ = fs::remove_file(&lock_path);
        return Err(err);
    }
    Ok(SessionReservation {
        log_path,
        lock_path,
        session_path,
        session_id: session_id.to_owned(),
        cleanup_on_drop: Cell::new(true),
        committed: Cell::new(false),
        side_effects_applied: Cell::new(false),
    })
}

fn reserve_unique_session_log(
    workspace: &Path,
    base_session_id: &str,
) -> Result<SessionReservation, RuntimeError> {
    for ordinal in 1..=10_000 {
        let candidate = if ordinal == 1 {
            base_session_id.to_owned()
        } else {
            suffixed_session_id(base_session_id, ordinal)
        };
        match reserve_session_log(workspace, &candidate) {
            Ok(reservation) => return Ok(reservation),
            Err(RuntimeError::SessionLogExists(_)) => continue,
            Err(err) if is_active_session_error(&err, &candidate) => return Err(err),
            Err(err) => return Err(err),
        }
    }

    Err(RuntimeError::Protocol(format!(
        "could not allocate a unique session_id for {base_session_id}"
    )))
}

fn is_active_session_error(err: &RuntimeError, session_id: &str) -> bool {
    matches!(
        err,
        RuntimeError::ActiveSession {
            session_id: active_session_id,
            ..
        } if active_session_id == session_id
    )
}

fn suffixed_session_id(base_session_id: &str, ordinal: u32) -> String {
    let suffix = format!("-{ordinal}");
    let prefix_len = 128usize.saturating_sub(suffix.len());
    let prefix = if base_session_id.len() > prefix_len {
        &base_session_id[..prefix_len]
    } else {
        base_session_id
    };
    let candidate = format!("{prefix}{suffix}");
    debug_assert!(validate_session_id(&candidate));
    candidate
}

fn reserve_session_file(path: &Path, session_id: &str) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
            path.display()
        ))),
        Ok(metadata) if metadata.is_file() => {
            Err(RuntimeError::SessionLogExists(session_id.to_owned()))
        }
        Ok(_) => Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            reserve_new_file(path).map_err(|err| match err {
                RuntimeError::Io { source, .. }
                    if source.kind() == io::ErrorKind::AlreadyExists =>
                {
                    RuntimeError::SessionLogExists(session_id.to_owned())
                }
                other => other,
            })
        }
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn reserve_new_file(path: &Path) -> Result<(), RuntimeError> {
    ensure_new_leaf_available(path)?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })
}

fn session_lock_path(workspace: &Path, session_id: &str) -> Result<PathBuf, RuntimeError> {
    if !validate_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    Ok(workspace
        .join(LOCAL_SESSION_DIR)
        .join(format!("{session_id}.lock")))
}

fn reserve_session_lock_file(path: &Path, session_id: &str) -> Result<(), RuntimeError> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            Err(RuntimeError::ActiveSession {
                session_id: session_id.to_owned(),
                lock_path: path.to_owned(),
            })
        }
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn active_session_lock_message(path: &Path, session_id: &str) -> String {
    // WHY: M1 cannot safely prove stale lock ownership, so report the exact manual clear
    // path instead of stealing the lock.
    format!(
        "session {session_id} is already active; lock file {} exists. If the previous process crashed, verify no Loop Agent process owns this session, then remove that lock file and retry.",
        path.display()
    )
}

fn acquire_session_lock(
    workspace: &Path,
    session_id: &str,
) -> Result<SessionLockGuard, RuntimeError> {
    let path = session_lock_path(workspace, session_id)?;
    ensure_existing_real_directory(&workspace.join(LOCAL_SESSION_DIR))?;
    reserve_session_lock_file(&path, session_id)?;
    Ok(SessionLockGuard { path })
}

fn write_reserved_session_metadata(
    reservation: &SessionReservation,
    session_id: &str,
    event_count: usize,
    definition_hashes: Option<&SessionDefinitionHashes>,
) -> Result<(), RuntimeError> {
    replace_existing_file_atomically(
        &reservation.log_path,
        session_log_metadata_text(session_id, event_count, definition_hashes).as_bytes(),
    )
}

fn session_log_metadata_text(
    session_id: &str,
    event_count: usize,
    definition_hashes: Option<&SessionDefinitionHashes>,
) -> String {
    let mut metadata = format!("session_id={session_id}\nevents={event_count}\n");
    if let Some(hashes) = definition_hashes {
        metadata.push_str("registry_hash=");
        metadata.push_str(&hashes.registry_hash);
        metadata.push('\n');
        metadata.push_str("loop_definition_hash=");
        metadata.push_str(&hashes.loop_definition_hash);
        metadata.push('\n');
    }
    metadata
}

fn write_initial_session_log_with_clock(
    reservation: &SessionReservation,
    session_id: &str,
    clock: EventClock,
) -> Result<(), RuntimeError> {
    let stream = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        session_id.to_owned(),
        1,
        clock.timestamp(1),
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .map_err(|err| RuntimeError::Protocol(format!("failed to serialize initial event: {err}")))?;
    write_existing_file(&reservation.session_path, stream.as_bytes())
}

fn preflight_session_completion_stream(
    reservation: &SessionReservation,
    expected_session_id: &str,
    events: &[EventEnvelope],
) -> Result<(String, Vec<EventEnvelope>), RuntimeError> {
    preflight_session_completion_stream_from_prefix(reservation, expected_session_id, events, 1)
}

fn preflight_session_completion_stream_from_prefix(
    reservation: &SessionReservation,
    expected_session_id: &str,
    events: &[EventEnvelope],
    persisted_event_count: usize,
) -> Result<(String, Vec<EventEnvelope>), RuntimeError> {
    let stream = canonical_event_stream(events)?;
    let validated_events =
        validate_session_log_text(Path::new("runtime.jsonl"), expected_session_id, &stream)?;
    preflight_complete_reserved_session_log_from_prefix(
        reservation,
        &stream,
        persisted_event_count,
    )?;
    Ok((stream, validated_events))
}

fn preflight_complete_reserved_session_log_from_prefix(
    reservation: &SessionReservation,
    stream: &str,
    persisted_event_count: usize,
) -> Result<(), RuntimeError> {
    let append_bytes = session_stream_suffix_bytes(stream, persisted_event_count)?;
    ensure_session_log_growth_within_limit(&reservation.session_path, append_bytes.len())
}

fn persist_reserved_session_prefix(
    reservation: &SessionReservation,
    session_id: &str,
    events: &[EventEnvelope],
    prefix_event_count: usize,
    definition_hashes: Option<&SessionDefinitionHashes>,
) -> Result<(), RuntimeError> {
    if prefix_event_count <= 1 {
        return Ok(());
    }
    let prefix_stream = canonical_event_stream(&events[..prefix_event_count])?;
    preflight_complete_reserved_session_log_from_prefix(reservation, &prefix_stream, 1)?;
    commit_reserved_session_log_from_prefix(
        reservation,
        session_id,
        &prefix_stream,
        prefix_event_count,
        definition_hashes,
        1,
    )
}

fn durable_run_prefix_event_count(events: &[EventEnvelope]) -> usize {
    events
        .iter()
        .position(|event| {
            event.event_type == EventType::LoopStarted && event.parent_loop_id.is_none()
        })
        .map_or(1, |index| index + 1)
}

fn commit_reserved_session_log_from_prefix(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
    definition_hashes: Option<&SessionDefinitionHashes>,
    persisted_event_count: usize,
) -> Result<(), RuntimeError> {
    let append_bytes = session_stream_suffix_bytes(stream, persisted_event_count)?;
    let append_result = append_session_log_bytes(&reservation.session_path, append_bytes);
    if append_result.is_ok() {
        reservation.mark_committed();
    }
    let metadata_result = if append_result.is_ok() {
        write_reserved_session_metadata(reservation, session_id, event_count, definition_hashes)
    } else {
        Ok(())
    };
    append_result?;
    metadata_result
}

fn session_stream_suffix_bytes(
    stream: &str,
    persisted_event_count: usize,
) -> Result<&[u8], RuntimeError> {
    if persisted_event_count == 0 {
        return Ok(stream.as_bytes());
    }
    let first_line_end = stream.find('\n').ok_or_else(|| {
        RuntimeError::Protocol("validated runtime stream must contain an initial event".to_owned())
    })?;
    let mut line_count = 1usize;
    let mut offset = first_line_end + 1;
    while line_count < persisted_event_count {
        let Some(relative_line_end) = stream[offset..].find('\n') else {
            return Err(RuntimeError::Protocol(format!(
                "validated runtime stream must contain persisted event prefix of {persisted_event_count}"
            )));
        };
        offset += relative_line_end + 1;
        line_count += 1;
    }
    Ok(&stream.as_bytes()[offset..])
}

fn ensure_runtime_dirs(workspace: &Path) -> Result<(PathBuf, PathBuf), RuntimeError> {
    let loop_dir = workspace.join(".loop");
    ensure_created_real_directory(&loop_dir)?;
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    ensure_created_real_directory(&session_dir)?;
    let log_dir = workspace.join(LOCAL_LOG_DIR);
    ensure_created_real_directory(&log_dir)?;
    Ok((session_dir, log_dir))
}

fn ensure_existing_session_log_path(workspace: &Path, path: &Path) -> Result<(), RuntimeError> {
    ensure_existing_real_directory(&workspace.join(".loop"))?;
    ensure_existing_real_directory(&workspace.join(LOCAL_SESSION_DIR))?;
    ensure_real_file(path)
}

fn ensure_existing_real_directory(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_directory_with(path, &metadata, DirectoryErrorMode::Protocol)
}

fn ensure_optional_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_optional_directory_with(path, DirectoryErrorMode::Protocol)
}

fn ensure_created_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_created_directory_with(path, DirectoryErrorMode::Protocol)
}

#[derive(Clone, Copy)]
enum DirectoryErrorMode {
    Protocol,
    ScriptWrite,
}

fn ensure_optional_directory_with(
    path: &Path,
    error_mode: DirectoryErrorMode,
) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_real_directory_with(path, &metadata, error_mode)?;
            Ok(true)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn ensure_created_directory_with(
    path: &Path,
    error_mode: DirectoryErrorMode,
) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_real_directory_with(path, &metadata, error_mode)?;
            Ok(false)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(RuntimeError::Io {
                        path: path.to_owned(),
                        source,
                    });
                }
            }
            let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?;
            validate_real_directory_with(path, &metadata, error_mode)?;
            Ok(true)
        }
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn validate_real_directory_with(
    path: &Path,
    metadata: &fs::Metadata,
    error_mode: DirectoryErrorMode,
) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || has_windows_reparse_point(metadata) {
        let message = format!("{} must not be a symlink or reparse point", path.display());
        return Err(match error_mode {
            DirectoryErrorMode::Protocol => RuntimeError::Protocol(message),
            DirectoryErrorMode::ScriptWrite => {
                runtime_denied(core_policy::DenyReasonCode::SymlinkEscapeDenied, message)
            }
        });
    }
    if !metadata.is_dir() {
        let message = format!("{} must be a directory", path.display());
        return Err(match error_mode {
            DirectoryErrorMode::Protocol => RuntimeError::Protocol(message),
            DirectoryErrorMode::ScriptWrite => {
                runtime_denied(core_policy::DenyReasonCode::WriteDenied, message)
            }
        });
    }
    Ok(())
}

#[cfg(windows)]
fn has_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn has_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn write_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_real_file(path)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    ensure_opened_regular_leaf_matches_path(path, &file)?;
    file.set_len(0).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(contents).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(not(unix))]
fn write_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    replace_existing_file_without_link_count(path, contents)
}

fn replace_existing_file_atomically(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_parent_real_directory(path)?;
    ensure_non_hardlinked_real_file(path)?;
    let (temp_path, mut temp_file) = create_replacement_temp(path, None)?;
    if let Err(err) = temp_file
        .write_all(contents)
        .map_err(|source| RuntimeError::Io {
            path: temp_path.clone(),
            source,
        })
    {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    drop(temp_file);

    ensure_parent_real_directory(path)?;
    ensure_non_hardlinked_real_file(path)?;
    replace_existing_leaf_from_temp(path, &temp_path, SideEffectRecorder::none(), None)
}

#[cfg(unix)]
fn append_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_real_file(path)?;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    ensure_opened_regular_leaf_matches_path(path, &file)?;
    file.write_all(contents).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(not(unix))]
fn append_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    append_existing_file_without_link_count(path, contents)
}

#[cfg(any(not(unix), test))]
fn append_existing_file_without_link_count(
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    let mut appended = read_to_bytes(path)?;
    appended.extend_from_slice(contents);
    replace_existing_file_without_link_count(path, &appended)
}

#[cfg(any(not(unix), test))]
fn replace_existing_file_without_link_count(
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    ensure_parent_real_directory(path)?;
    ensure_real_file(path)?;
    let (temp_path, mut temp_file) = create_replacement_temp(path, None)?;
    if let Err(err) = temp_file
        .write_all(contents)
        .map_err(|source| RuntimeError::Io {
            path: temp_path.clone(),
            source,
        })
    {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    drop(temp_file);

    ensure_parent_real_directory(path)?;
    ensure_real_file(path)?;
    replace_existing_leaf_from_temp(path, &temp_path, SideEffectRecorder::none(), None)
}

fn ensure_new_leaf_available(path: &Path) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
            path.display()
        ))),
        Ok(_) => Err(RuntimeError::Protocol(format!(
            "{} must not already exist",
            path.display()
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn ensure_real_file(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_file(path, &metadata)
}

fn ensure_non_hardlinked_real_file(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_file(path, &metadata)?;
    ensure_not_hardlinked_file(path, &metadata)
}

fn validate_real_file(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || has_windows_reparse_point(metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_not_hardlinked_file(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if hard_link_count(metadata) > 1 {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be hard-linked",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_not_hardlinked_file(_path: &Path, _metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    Ok(())
}

fn ensure_parent_real_directory(path: &Path) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(|| {
        RuntimeError::Protocol(format!("{} must have a parent directory", path.display()))
    })?;
    ensure_existing_real_directory(parent)
}

struct RuntimeExecution {
    events: Vec<EventEnvelope>,
    failed: bool,
    terminal_error: Option<RuntimeError>,
}

#[derive(Clone, Debug)]
struct LoopInvocation {
    loop_id: String,
    parent_loop_id: Option<String>,
}

struct RuntimeFailure {
    reason: String,
    message: &'static str,
    tool_id: Option<String>,
    phase_id: Option<String>,
    emit_tool_failed: bool,
}

#[derive(Clone, Copy)]
struct RuntimeToolPolicy<'a> {
    command: &'a core_policy::CommandPolicy,
    protected_path_match_mode: ProtectedPathMatchMode,
    target: &'a core_policy::PolicyTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSideEffectMode {
    ApplyAll,
    DryRun,
    PreflightResume { prefix_event_count: u64 },
    Resume { prefix_event_count: u64 },
}

impl ToolSideEffectMode {
    fn should_execute_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::ApplyAll => true,
            Self::DryRun => false,
            Self::PreflightResume { .. } => false,
            Self::Resume { prefix_event_count } => completed_sequence > prefix_event_count,
        }
    }

    fn should_preflight_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::PreflightResume { prefix_event_count } => completed_sequence > prefix_event_count,
            Self::ApplyAll | Self::DryRun | Self::Resume { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SideEffectRecorder<'a> {
    reservation: Option<&'a SessionReservation>,
}

impl<'a> SideEffectRecorder<'a> {
    fn none() -> Self {
        Self { reservation: None }
    }

    fn for_reservation(reservation: &'a SessionReservation) -> Self {
        Self {
            reservation: Some(reservation),
        }
    }

    fn mark_applied(self) {
        if let Some(reservation) = self.reservation {
            reservation.mark_side_effects_applied();
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LoopExecutionOptions<'a> {
    clock: EventClock,
    side_effect_mode: ToolSideEffectMode,
    side_effect_recorder: SideEffectRecorder<'a>,
}

impl<'a> LoopExecutionOptions<'a> {
    fn new(
        clock: EventClock,
        side_effect_mode: ToolSideEffectMode,
        side_effect_recorder: SideEffectRecorder<'a>,
    ) -> Self {
        Self {
            clock,
            side_effect_mode,
            side_effect_recorder,
        }
    }
}

fn runtime_policy_artifact(
    artifacts: &[core_policy::PolicyArtifact],
) -> Result<&core_policy::PolicyArtifact, RuntimeError> {
    let target = runtime_policy_target();
    runtime_policy_artifact_for_target(artifacts, &target)
}

#[cfg(target_os = "macos")]
fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::MacosSeatbelt
}

#[cfg(not(target_os = "macos"))]
fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::LinuxLandlockSeccomp
}

#[cfg(windows)]
fn runtime_protected_path_match_mode(target: &core_policy::PolicyTarget) -> ProtectedPathMatchMode {
    let _policy_mode = protected_path_match_mode_for_policy_target(target);
    ProtectedPathMatchMode::CaseInsensitive
}

#[cfg(not(windows))]
fn runtime_protected_path_match_mode(target: &core_policy::PolicyTarget) -> ProtectedPathMatchMode {
    protected_path_match_mode_for_policy_target(target)
}

fn runtime_policy_artifact_for_target<'a>(
    artifacts: &'a [core_policy::PolicyArtifact],
    target: &core_policy::PolicyTarget,
) -> Result<&'a core_policy::PolicyArtifact, RuntimeError> {
    artifacts
        .iter()
        .find(|artifact| &artifact.target == target)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "missing {} runtime policy artifact",
                policy_target_name(target)
            ))
        })
}

fn policy_target_name(target: &core_policy::PolicyTarget) -> &'static str {
    match target {
        core_policy::PolicyTarget::LinuxLandlockSeccomp => "linux",
        core_policy::PolicyTarget::MacosSeatbelt => "macos",
    }
}

struct RuntimeEventBuilder {
    clock: EventClock,
    events: Vec<EventEnvelope>,
    loop_counter: u64,
    message_counter: u64,
    sequence: u64,
    session_id: String,
    stream_bytes: usize,
}

impl RuntimeEventBuilder {
    fn with_clock(session_id: String, clock: EventClock) -> Self {
        Self {
            clock,
            events: Vec::new(),
            loop_counter: 0,
            message_counter: 0,
            sequence: 0,
            session_id,
            stream_bytes: 0,
        }
    }

    fn next_loop_invocation(
        &mut self,
        parent_loop_id: Option<String>,
    ) -> Result<LoopInvocation, RuntimeError> {
        let next_loop_counter = self.loop_counter + 1;
        // WHY: loop invocation budgets preserve duplicate subloop execution semantics while
        // bounding the total runtime work one session can request.
        if next_loop_counter > MAX_LOOP_INVOCATIONS {
            return Err(RuntimeError::Protocol(format!(
                "loop invocation budget exceeded: next invocation {next_loop_counter} exceeds max {MAX_LOOP_INVOCATIONS}"
            )));
        }
        self.loop_counter = next_loop_counter;
        Ok(LoopInvocation {
            loop_id: format!("loop-{:03}", self.loop_counter),
            parent_loop_id,
        })
    }

    fn next_message_id(&mut self) -> String {
        self.message_counter += 1;
        format!("msg-{:03}", self.message_counter)
    }

    fn emit(
        &mut self,
        invocation: Option<&LoopInvocation>,
        event_type: EventType,
        payload: serde_json::Value,
    ) -> Result<(), RuntimeError> {
        let sequence = self.sequence + 1;
        // WHY: enforce event budgets before storing the event so oversized in-cap loops
        // cannot accumulate unbounded memory.
        if sequence > MAX_LOOP_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "runtime event budget exceeded: next event {sequence} exceeds max {MAX_LOOP_EVENTS}"
            )));
        }
        let mut event = EventEnvelope::new(
            format!("evt-{:03}", sequence),
            event_type,
            self.session_id.clone(),
            sequence,
            self.clock.timestamp(sequence),
            "loop-agent-cli",
            payload,
        );
        if let Some(invocation) = invocation {
            event.loop_id = Some(invocation.loop_id.clone());
            event.parent_loop_id = invocation.parent_loop_id.clone();
        }
        event.normalize_strings_to_nfc();
        let event_bytes = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize runtime event: {err}"))
        })?;
        let next_stream_bytes = self
            .stream_bytes
            .checked_add(event_bytes.len())
            .unwrap_or(usize::MAX);
        if next_stream_bytes > MAX_LOOP_EVENT_STREAM_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "event stream budget exceeded: next event would use {next_stream_bytes} bytes, max {MAX_LOOP_EVENT_STREAM_BYTES}"
            )));
        }
        self.sequence = sequence;
        self.stream_bytes = next_stream_bytes;
        self.events.push(event);
        Ok(())
    }
}

fn execute_loop(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_loop: &core_script::LoopBlock,
    session_id: &str,
    options: LoopExecutionOptions<'_>,
) -> Result<RuntimeExecution, RuntimeError> {
    let mut builder = RuntimeEventBuilder::with_clock(session_id.to_owned(), options.clock);
    builder.emit(
        None,
        EventType::SessionStarted,
        serde_json::json!({"reason":"fixture-start"}),
    )?;

    let context = LoopEmitContext {
        workspace,
        registry,
        policy,
        side_effect_mode: options.side_effect_mode,
        side_effect_recorder: options.side_effect_recorder,
    };
    let failed = match emit_loop_block(&context, root_loop, None, &mut builder) {
        Ok(failed) => failed,
        Err(err) if should_terminalize_runtime_error(options.side_effect_mode) => {
            builder.emit(
                None,
                EventType::SessionFailed,
                serde_json::json!({"reason":RUNTIME_ERROR_REASON}),
            )?;
            return Ok(RuntimeExecution {
                events: builder.events,
                failed: true,
                terminal_error: Some(err),
            });
        }
        Err(err) => return Err(err),
    };
    if let Some(failure) = failed {
        builder.emit(
            None,
            EventType::SessionFailed,
            serde_json::json!({"reason":failure.reason}),
        )?;
        Ok(RuntimeExecution {
            events: builder.events,
            failed: true,
            terminal_error: None,
        })
    } else {
        builder.emit(None, EventType::SessionCompleted, serde_json::json!({}))?;
        Ok(RuntimeExecution {
            events: builder.events,
            failed: false,
            terminal_error: None,
        })
    }
}

fn should_terminalize_runtime_error(side_effect_mode: ToolSideEffectMode) -> bool {
    matches!(
        side_effect_mode,
        ToolSideEffectMode::ApplyAll | ToolSideEffectMode::Resume { .. }
    )
}

fn preflight_loop_tools(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
) -> Result<(), RuntimeError> {
    preflight_loop_tools_at_depth(workspace, registry, policy, loop_block, 1)
}

fn preflight_loop_tools_at_depth(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    depth: usize,
) -> Result<(), RuntimeError> {
    if depth > core_script::MAX_LOOP_NESTING_DEPTH {
        return Err(RuntimeError::Protocol(format!(
            "loop nesting depth {depth} for {} exceeds max {}",
            loop_block.identity.id,
            core_script::MAX_LOOP_NESTING_DEPTH
        )));
    }

    for phase_ref in &loop_block.phase_refs {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        preflight_phase_tools(workspace, registry, policy, phase)?;
    }

    for subloop_ref in &loop_block.subloop_refs {
        let subloop = registry.loop_block(subloop_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
        })?;
        preflight_loop_tools_at_depth(workspace, registry, policy, subloop, depth + 1)?;
    }

    Ok(())
}

fn preflight_phase_tools(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    phase: &core_script::PhaseBlock,
) -> Result<(), RuntimeError> {
    for tool_ref in &phase.tool_refs {
        let tool = registry.tool_block(tool_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
        })?;
        let command_policy = command_policy_for_phase(policy, &phase.identity.id, tool)?;
        ensure_tool_matches_policy(tool, &policy.target, command_policy)?;
        preflight_tool_progress(
            workspace,
            tool,
            runtime_protected_path_match_mode(&policy.target),
            command_policy,
        )?;
    }
    Ok(())
}

fn emit_loop_block(
    context: &LoopEmitContext<'_>,
    loop_block: &core_script::LoopBlock,
    parent_loop_id: Option<String>,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    emit_loop_block_at_depth(context, loop_block, parent_loop_id, builder, 1)
}

struct LoopEmitContext<'a> {
    workspace: &'a Path,
    registry: &'a core_script::ResolvedRegistry,
    policy: &'a core_policy::PolicyArtifact,
    side_effect_mode: ToolSideEffectMode,
    side_effect_recorder: SideEffectRecorder<'a>,
}

fn emit_loop_block_at_depth(
    context: &LoopEmitContext<'_>,
    loop_block: &core_script::LoopBlock,
    parent_loop_id: Option<String>,
    builder: &mut RuntimeEventBuilder,
    depth: usize,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    if depth > core_script::MAX_LOOP_NESTING_DEPTH {
        return Err(RuntimeError::Protocol(format!(
            "loop nesting depth {depth} for {} exceeds max {}",
            loop_block.identity.id,
            core_script::MAX_LOOP_NESTING_DEPTH
        )));
    }

    let invocation = builder.next_loop_invocation(parent_loop_id)?;
    builder.emit(
        Some(&invocation),
        EventType::LoopStarted,
        serde_json::json!({
            "loop_definition_id": loop_block.identity.id,
            "loop_name": loop_block.identity.name,
        }),
    )?;

    for phase_ref in &loop_block.phase_refs {
        let phase = context.registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        match emit_phase(context, phase, &invocation, builder) {
            Ok(Some(failure)) => {
                emit_runtime_failure(loop_block, &invocation, &failure, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                emit_runtime_error_failure(loop_block, &invocation, &err, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    for subloop_ref in &loop_block.subloop_refs {
        let subloop = context.registry.loop_block(subloop_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
        })?;
        match emit_loop_block_at_depth(
            context,
            subloop,
            Some(invocation.loop_id.clone()),
            builder,
            depth + 1,
        ) {
            Ok(Some(failure)) => {
                emit_propagated_runtime_failure(loop_block, &invocation, &failure, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                emit_propagated_runtime_error_failure(loop_block, &invocation, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    builder.emit(
        Some(&invocation),
        EventType::LoopCompleted,
        serde_json::json!({
            "loop_definition_id": loop_block.identity.id,
            "loop_name": loop_block.identity.name,
        }),
    )?;
    Ok(None)
}

fn emit_phase(
    context: &LoopEmitContext<'_>,
    phase: &core_script::PhaseBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let instruction_ids = phase
        .instruction_refs
        .iter()
        .map(|instruction_ref| {
            context
                .registry
                .instruction_block(instruction_ref)
                .map(|instruction| instruction.identity.id.clone())
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "resolved registry missing instruction {instruction_ref}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let tool_ids = phase
        .tool_refs
        .iter()
        .map(|tool_ref| {
            context
                .registry
                .tool_block(tool_ref)
                .map(|tool| tool.identity.id.clone())
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    builder.emit(
        Some(invocation),
        EventType::PhaseEntered,
        serde_json::json!({
            "instruction_ids": instruction_ids,
            "phase_id": phase.identity.id,
            "phase_name": phase.identity.name,
            "tool_ids": tool_ids,
        }),
    )?;

    for (step_index, step) in phase.steps.iter().enumerate() {
        let step_payload = step_payload(context.registry, phase, step)?;
        builder.emit(
            Some(invocation),
            EventType::StepStarted,
            step_payload.clone(),
        )?;

        if let Some(content) = stub_message_content(context.registry, phase)? {
            let message_id = builder.next_message_id();
            builder.emit(
                Some(invocation),
                EventType::MessageDelta,
                serde_json::json!({
                    "content_delta": content,
                    "message_id": message_id,
                    "role": "assistant",
                }),
            )?;
            builder.emit(
                Some(invocation),
                EventType::MessageCompleted,
                serde_json::json!({
                    "message_id": message_id,
                    "role": "assistant",
                }),
            )?;
        }

        if step_index == 0 {
            if let Some(failure) =
                sandbox_out_of_phase_failure(context.registry, context.policy, phase)
            {
                builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                return Ok(Some(failure));
            }

            for tool_ref in &phase.tool_refs {
                let tool = context.registry.tool_block(tool_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })?;
                let command_policy =
                    command_policy_for_phase(context.policy, &phase.identity.id, tool)?;
                let tool_policy = RuntimeToolPolicy {
                    command: command_policy,
                    protected_path_match_mode: runtime_protected_path_match_mode(
                        &context.policy.target,
                    ),
                    target: &context.policy.target,
                };
                match emit_tool(
                    context.workspace,
                    tool,
                    tool_policy,
                    invocation,
                    context.side_effect_mode,
                    context.side_effect_recorder,
                    builder,
                ) {
                    Ok(Some(mut failure)) => {
                        emit_runtime_tool_failure(invocation, &failure, builder)?;
                        failure.emit_tool_failed = false;
                        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                        return Ok(Some(failure));
                    }
                    Ok(None) => {}
                    Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                        let mut failure = runtime_failure_for_unhandled_error(&err);
                        failure.tool_id = Some(tool.identity.id.clone());
                        emit_runtime_tool_failure(invocation, &failure, builder)?;
                        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                        return Err(err);
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
    }

    Ok(None)
}

fn step_payload(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    step: &core_script::StepBlock,
) -> Result<serde_json::Value, RuntimeError> {
    let mut payload = serde_json::json!({
        "phase_id": phase.identity.id,
        "step_id": step.id,
        "step_name": step.name,
    });
    if !step.connection_refs.is_empty() {
        let connection_kinds = step
            .connection_refs
            .iter()
            .map(|connection_ref| {
                let connection = registry.connection_block(connection_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "resolved registry missing connection {connection_ref}"
                    ))
                })?;
                Ok(connection_kind_name(&connection.connection_kind))
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let connection_ids = step
            .connection_refs
            .iter()
            .map(|connection_ref| {
                registry
                    .connection_block(connection_ref)
                    .map(|connection| connection.identity.id.clone())
                    .ok_or_else(|| {
                        RuntimeError::Protocol(format!(
                            "resolved registry missing connection {connection_ref}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let object = payload
            .as_object_mut()
            .expect("step payload is constructed as an object");
        object.insert(
            "connection_ids".to_owned(),
            serde_json::json!(connection_ids),
        );
        object.insert(
            "connection_kinds".to_owned(),
            serde_json::json!(connection_kinds),
        );
    }
    Ok(payload)
}

fn stub_message_content(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
) -> Result<Option<&'static str>, RuntimeError> {
    let has_predefined_tool = phase.tool_refs.iter().any(|tool_ref| {
        registry
            .tool_block(tool_ref)
            .is_some_and(|tool| tool.tool_kind == core_script::ToolKind::PredefinedCommand)
    });
    if !has_predefined_tool {
        return Ok(None);
    }

    for instruction_ref in &phase.instruction_refs {
        let instruction = registry.instruction_block(instruction_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "resolved registry missing instruction {instruction_ref}"
            ))
        })?;
        if instruction.prompt.to_ascii_lowercase().contains("smoke") {
            return Ok(Some("smoke"));
        }
    }

    Ok(Some("hello"))
}

fn command_policy_for_phase<'a>(
    policy: &'a core_policy::PolicyArtifact,
    phase_id: &str,
    tool: &core_script::ToolBlock,
) -> Result<&'a core_policy::CommandPolicy, RuntimeError> {
    let scoped = policy
        .phase_scope
        .iter()
        .find(|phase| phase.phase_id == phase_id)
        .is_some_and(|phase| {
            phase
                .tool_ids
                .iter()
                .any(|tool_id| tool_id == &tool.identity.id)
        });
    if !scoped {
        return Err(RuntimeError::Protocol(format!(
            "tool {} is not available in phase {phase_id}",
            tool.identity.id
        )));
    }
    policy
        .commands
        .iter()
        .find(|command| command.tool_id == tool.identity.id)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "runtime policy missing command for tool {}",
                tool.identity.id
            ))
        })
}

fn ensure_tool_matches_policy(
    tool: &core_script::ToolBlock,
    target: &core_policy::PolicyTarget,
    policy: &core_policy::CommandPolicy,
) -> Result<(), RuntimeError> {
    if policy.tool_id != tool.identity.id {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy tool_id {} does not match tool {}",
            policy.tool_id, tool.identity.id
        )));
    }
    if policy_tool_kind_name(&policy.tool_kind) != tool_kind_name(&tool.tool_kind) {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy kind does not match tool {}",
            tool.identity.id
        )));
    }
    if policy.network.default != core_policy::NetworkDefault::Deny {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use deny-all network policy",
            tool.identity.id
        )));
    }
    if matches!(target, core_policy::PolicyTarget::LinuxLandlockSeccomp)
        && !policy.network.allow.is_empty()
    {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use deny-all network policy",
            tool.identity.id
        )));
    }

    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => {
            if policy.command_id != *command_id
                || policy.executable != format!("registry:{command_id}")
                || policy.argv != *argv
                || policy.script_runtime.is_some()
            {
                return Err(RuntimeError::Protocol(format!(
                    "runtime policy command does not match tool {}",
                    tool.identity.id
                )));
            }
        }
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(command_id)) => {
            if policy.command_id != *command_id
                || policy.executable != "runner:posix-sh"
                || policy.script_runtime.as_deref() != Some("posix-sh")
                || !policy.argv.is_empty()
            {
                return Err(RuntimeError::Protocol(format!(
                    "runtime policy script command does not match tool {}",
                    tool.identity.id
                )));
            }
        }
        _ => {
            return Err(RuntimeError::Protocol(format!(
                "tool command shape does not match {}",
                tool.identity.id
            )));
        }
    }

    Ok(())
}

fn emit_tool(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    policy: RuntimeToolPolicy<'_>,
    invocation: &LoopInvocation,
    side_effect_mode: ToolSideEffectMode,
    side_effect_recorder: SideEffectRecorder<'_>,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    ensure_tool_matches_policy(tool, policy.target, policy.command)?;
    let planned_progress =
        planned_tool_progress(tool, policy.protected_path_match_mode, policy.command)?;
    builder.emit(
        Some(invocation),
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": policy.command.allowed_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
            "network_access": tool_network_access_name(&tool.network),
            "read_scope": policy.command.filesystem.read_roots,
            "tool_id": tool.identity.id,
            "tool_kind": policy_tool_kind_name(&policy.command.tool_kind),
            "tool_name": tool.identity.name,
            "write_scope": policy.command.filesystem.write_roots,
        }),
    )?;

    if let Some(failure) = sandbox_tool_dispatch_failure(tool, policy.target, policy.command)? {
        return Ok(Some(failure));
    }

    let side_effect_sequence = builder.sequence + 1;
    let completed_sequence = side_effect_sequence + u64::from(planned_progress.is_some());
    let replay_guard_sequence = if planned_progress.is_some() {
        side_effect_sequence
    } else {
        completed_sequence
    };
    let progress = if side_effect_mode.should_execute_tool(replay_guard_sequence) {
        match execute_tool(
            workspace,
            tool,
            policy.protected_path_match_mode,
            policy.command,
            side_effect_recorder,
        ) {
            Ok(progress) => progress,
            Err(err) => {
                if matches!(side_effect_mode, ToolSideEffectMode::ApplyAll) {
                    if let Some(failure) = runtime_failure_for_tool_error(&err, &tool.identity.id) {
                        return Ok(Some(failure));
                    }
                }
                return Err(err);
            }
        }
    } else if side_effect_mode.should_preflight_tool(replay_guard_sequence) {
        preflight_tool_progress(
            workspace,
            tool,
            policy.protected_path_match_mode,
            policy.command,
        )?
    } else {
        planned_progress
    };

    if let Some(message) = progress {
        emit_tool_progress(message, tool, invocation, builder)?;
    }

    builder.emit(
        Some(invocation),
        EventType::ToolCompleted,
        serde_json::json!({
            "exit_code": 0,
            "tool_id": tool.identity.id,
        }),
    )?;
    Ok(None)
}

fn execute_tool(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    side_effect_recorder: SideEffectRecorder<'_>,
) -> Result<Option<&'static str>, RuntimeError> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => execute_predefined_command(policy, command_id, argv),
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(_)) => {
            execute_own_script(
                workspace,
                tool,
                protected_path_match_mode,
                policy,
                side_effect_recorder,
            )?;
            Ok(Some("stub write completed"))
        }
        _ => Err(RuntimeError::Protocol(format!(
            "tool command shape does not match {}",
            tool.identity.id
        ))),
    }
}

fn planned_tool_progress(
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<Option<&'static str>, RuntimeError> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => execute_predefined_command(policy, command_id, argv),
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(_)) => {
            plan_own_script(tool, protected_path_match_mode, policy)?;
            Ok(Some("stub write completed"))
        }
        _ => Err(RuntimeError::Protocol(format!(
            "tool command shape does not match {}",
            tool.identity.id
        ))),
    }
}

fn preflight_tool_progress(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<Option<&'static str>, RuntimeError> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => execute_predefined_command(policy, command_id, argv),
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(_)) => {
            let operations = plan_own_script(tool, protected_path_match_mode, policy)?;
            preflight_own_script_outputs(workspace, &operations)?;
            Ok(Some("stub write completed"))
        }
        _ => Err(RuntimeError::Protocol(format!(
            "tool command shape does not match {}",
            tool.identity.id
        ))),
    }
}

fn execute_predefined_command(
    policy: &core_policy::CommandPolicy,
    command_id: &str,
    argv: &[String],
) -> Result<Option<&'static str>, RuntimeError> {
    let command = trusted_predefined_command(command_id).ok_or_else(|| {
        RuntimeError::Protocol(format!("unsupported predefined command {command_id:?}"))
    })?;
    let executable = format!("registry:{command_id}");
    if policy.executable != executable || policy.argv != argv {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy executable does not match trusted command {command_id:?}"
        )));
    }
    Ok(command.progress)
}

fn trusted_predefined_command(command_id: &str) -> Option<TrustedPredefinedCommand> {
    TRUSTED_PREDEFINED_COMMANDS
        .iter()
        .copied()
        .find(|command| command.command_id == command_id)
}

fn execute_own_script(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    side_effect_recorder: SideEffectRecorder<'_>,
) -> Result<(), RuntimeError> {
    if tool.script_runtime.as_ref() != Some(&core_script::ScriptRuntime::PosixSh) {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use script_runtime posix-sh",
            tool.identity.id
        )));
    }
    let operations = plan_own_script(tool, protected_path_match_mode, policy)?;
    for operation in operations {
        match operation {
            ScriptOperation::Noop => {}
            ScriptOperation::Write { contents, target } => {
                write_script_output(workspace, &target, &contents, side_effect_recorder)?;
            }
        }
    }
    Ok(())
}

fn plan_own_script(
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<Vec<ScriptOperation>, RuntimeError> {
    if tool.script_runtime.as_ref() != Some(&core_script::ScriptRuntime::PosixSh) {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use script_runtime posix-sh",
            tool.identity.id
        )));
    }
    let script_body = tool.script_body.as_deref().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "tool {} must include script_body",
            tool.identity.id
        ))
    })?;
    compile_own_script_operations(protected_path_match_mode, policy, script_body)
}

enum ScriptOperation {
    Noop,
    Write { contents: Vec<u8>, target: String },
}

fn compile_own_script_operations(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    script_body: &str,
) -> Result<Vec<ScriptOperation>, RuntimeError> {
    let mut operations = Vec::new();
    let mut write_count = 0usize;
    for line in script_body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line == "---" {
            operations.push(ScriptOperation::Noop);
            continue;
        }
        if let Some((command, target)) = script_redirection(line)? {
            write_count += 1;
            if write_count > 1 {
                return Err(RuntimeError::Protocol(
                    "own-script multiple write operations are not supported in M1".to_owned(),
                ));
            }
            let target = validate_script_write_target(protected_path_match_mode, policy, &target)?;
            let contents = evaluate_script_command(&command)?;
            operations.push(ScriptOperation::Write { contents, target });
        } else {
            evaluate_script_command(line)?;
            operations.push(ScriptOperation::Noop);
        }
    }
    Ok(operations)
}

fn script_redirection(line: &str) -> Result<Option<(String, String)>, RuntimeError> {
    let positions = redirection_positions(line)?;
    let Some(&redirection_index) = positions.first() else {
        return Ok(None);
    };
    if positions.len() > 1 {
        return Err(RuntimeError::Protocol(
            "own-script multiple redirections are not supported in M1".to_owned(),
        ));
    }
    let command = line[..redirection_index].trim();
    if command.is_empty() {
        return Err(RuntimeError::Protocol(
            "own-script redirection must include a command".to_owned(),
        ));
    }
    let target = unquote_script_path(line[redirection_index + 1..].trim())?;
    Ok(Some((command.to_owned(), target)))
}

fn redirection_positions(line: &str) -> Result<Vec<usize>, RuntimeError> {
    let mut positions = Vec::new();
    let mut quote = None;
    let mut chars = line.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if matches!(ch, '\'' | '"') => quote = Some(ch),
            None if ch == '>' => {
                if matches!(chars.peek(), Some((_, '>'))) {
                    return Err(RuntimeError::Protocol(
                        "own-script append redirection is not supported in M1".to_owned(),
                    ));
                }
                positions.push(index);
            }
            None => {}
        }
    }

    if quote.is_some() {
        return Err(RuntimeError::Protocol(
            "own-script command contains an unterminated quote".to_owned(),
        ));
    }

    Ok(positions)
}

fn unquote_script_path(value: &str) -> Result<String, RuntimeError> {
    if value.is_empty() {
        return Err(RuntimeError::Protocol(
            "own-script redirection target must be one literal path".to_owned(),
        ));
    }
    if let Some(quote) = value.chars().next().filter(|ch| matches!(ch, '"' | '\'')) {
        if value.len() < 2 || !value.ends_with(quote) {
            return Err(RuntimeError::Protocol(
                "own-script redirection target must be one literal path".to_owned(),
            ));
        }
        return Ok(value[1..value.len() - 1].to_owned());
    }
    if value.contains('"') || value.contains('\'') || value.split_whitespace().count() != 1 {
        return Err(RuntimeError::Protocol(
            "own-script redirection target must be one literal path".to_owned(),
        ));
    }
    Ok(value.to_owned())
}

fn validate_script_write_target(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    target: &str,
) -> Result<String, RuntimeError> {
    let relative = normalize_script_write_target(target)?;
    let scoped = format!("workspace/{relative}");
    if !policy
        .filesystem
        .write_roots
        .iter()
        .any(|root| core_script::relative_path_is_inside_scope(&scoped, root))
    {
        return Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("tool {} lacks write scope {scoped}", policy.tool_id),
        ));
    }
    let temp_parent_scoped = script_replacement_temp_parent_scope(&relative);
    if !policy
        .filesystem
        .write_roots
        .iter()
        .any(|root| core_script::relative_path_is_inside_scope(&temp_parent_scoped, root))
    {
        return Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!(
                "tool {} lacks write scope for replacement temp under {temp_parent_scoped}",
                policy.tool_id
            ),
        ));
    }
    ensure_script_target_not_protected(protected_path_match_mode, policy, &scoped)?;
    Ok(relative)
}

fn script_replacement_temp_parent_scope(relative: &str) -> String {
    relative.rsplit_once('/').map_or_else(
        || "workspace".to_owned(),
        |(parent, _)| format!("workspace/{parent}"),
    )
}

fn write_script_output(
    workspace: &Path,
    target: &str,
    contents: &[u8],
    side_effect_recorder: SideEffectRecorder<'_>,
) -> Result<(), RuntimeError> {
    let path = ensure_real_workspace_write_path(workspace, target, side_effect_recorder)?;
    replace_script_output_atomically(workspace, target, &path, contents, side_effect_recorder)
}

fn preflight_own_script_outputs(
    workspace: &Path,
    operations: &[ScriptOperation],
) -> Result<(), RuntimeError> {
    for operation in operations {
        if let ScriptOperation::Write { target, .. } = operation {
            let path = preflight_real_workspace_write_path(workspace, target)?;
            ensure_writable_regular_leaf(&path)?;
        }
    }
    Ok(())
}

fn replace_script_output_atomically(
    workspace: &Path,
    target: &str,
    path: &Path,
    contents: &[u8],
    side_effect_recorder: SideEffectRecorder<'_>,
) -> Result<(), RuntimeError> {
    ensure_real_workspace_write_path(workspace, target, side_effect_recorder)?;
    let initial_leaf_existed = ensure_writable_regular_leaf(path)?;
    let (temp_path, mut temp_file) =
        create_replacement_temp(path, Some(core_policy::DenyReasonCode::WriteDenied))?;
    if let Err(err) = temp_file
        .write_all(contents)
        .map_err(|source| RuntimeError::Io {
            path: temp_path.clone(),
            source,
        })
    {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    drop(temp_file);

    ensure_real_workspace_write_path(workspace, target, side_effect_recorder)?;
    if initial_leaf_existed {
        if ensure_writable_regular_leaf(path)? {
            return replace_existing_leaf_from_temp(
                path,
                &temp_path,
                side_effect_recorder,
                Some(core_policy::DenyReasonCode::WriteDenied),
            );
        }
    } else {
        ensure_new_leaf_available(path)?;
    }
    ensure_real_workspace_write_path(workspace, target, side_effect_recorder)?;
    if let Err(source) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        });
    }
    side_effect_recorder.mark_applied();
    Ok(())
}

#[cfg(unix)]
fn replace_existing_leaf_from_temp(
    path: &Path,
    temp_path: &Path,
    side_effect_recorder: SideEffectRecorder<'_>,
    _denied_reason: Option<core_policy::DenyReasonCode>,
) -> Result<(), RuntimeError> {
    if let Err(source) = fs::rename(temp_path, path) {
        let _ = fs::remove_file(temp_path);
        return Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        });
    }
    side_effect_recorder.mark_applied();
    Ok(())
}

#[cfg(not(unix))]
fn replace_existing_leaf_from_temp(
    path: &Path,
    temp_path: &Path,
    side_effect_recorder: SideEffectRecorder<'_>,
    denied_reason: Option<core_policy::DenyReasonCode>,
) -> Result<(), RuntimeError> {
    let backup_path = create_replacement_backup_path(path, denied_reason)?;
    if let Err(source) = fs::rename(path, &backup_path) {
        let _ = fs::remove_file(temp_path);
        return Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        });
    }
    // WHY: once the original target is moved aside, a later failure must not
    // erase the session attempt that explains the workspace change.
    side_effect_recorder.mark_applied();
    if let Err(source) = fs::rename(temp_path, path) {
        if fs::rename(&backup_path, path).is_ok() {
            let _ = fs::remove_file(temp_path);
        }
        return Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        });
    }
    fs::remove_file(&backup_path).map_err(|source| RuntimeError::Io {
        path: backup_path,
        source,
    })
}

fn create_replacement_temp(
    path: &Path,
    denied_reason: Option<core_policy::DenyReasonCode>,
) -> Result<(PathBuf, fs::File), RuntimeError> {
    for attempt in 0..100 {
        let temp_path = replacement_temp_path(path, attempt)?;
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(RuntimeError::Io {
                    path: temp_path,
                    source,
                });
            }
        }
    }
    Err(runtime_protocol_or_denied(
        denied_reason,
        format!(
            "could not allocate temporary replacement path for {}",
            path.display()
        ),
    ))
}

#[cfg(not(unix))]
fn create_replacement_backup_path(
    path: &Path,
    denied_reason: Option<core_policy::DenyReasonCode>,
) -> Result<PathBuf, RuntimeError> {
    for attempt in 0..100 {
        let backup_path = replacement_backup_path(path, attempt)?;
        match fs::symlink_metadata(&backup_path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(backup_path),
            Ok(_) => {}
            Err(source) => {
                return Err(RuntimeError::Io {
                    path: backup_path,
                    source,
                });
            }
        }
    }
    Err(runtime_protocol_or_denied(
        denied_reason,
        format!(
            "could not allocate backup replacement path for {}",
            path.display()
        ),
    ))
}

fn replacement_temp_path(path: &Path, attempt: u32) -> Result<PathBuf, RuntimeError> {
    let mut file_name = path
        .file_name()
        .ok_or_else(|| RuntimeError::Protocol("replacement path must have a file name".to_owned()))?
        .to_os_string();
    file_name.push(format!(".watershed-{}-{attempt}.tmp", std::process::id()));
    Ok(path.with_file_name(file_name))
}

#[cfg(not(unix))]
fn replacement_backup_path(path: &Path, attempt: u32) -> Result<PathBuf, RuntimeError> {
    let mut file_name = path
        .file_name()
        .ok_or_else(|| RuntimeError::Protocol("replacement path must have a file name".to_owned()))?
        .to_os_string();
    file_name.push(format!(".watershed-{}-{attempt}.bak", std::process::id()));
    Ok(path.with_file_name(file_name))
}

fn ensure_script_target_not_protected(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    scoped_target: &str,
) -> Result<(), RuntimeError> {
    if !policy.filesystem.protected_paths.iter().any(|pattern| {
        protected_path_pattern_matches(protected_path_match_mode, pattern, scoped_target)
    }) {
        return Ok(());
    }

    if policy
        .filesystem
        .protected_path_grants
        .iter()
        .any(|pattern| {
            protected_path_pattern_matches(protected_path_match_mode, pattern, scoped_target)
        })
    {
        return Ok(());
    }

    Err(runtime_denied(
        core_policy::DenyReasonCode::ProtectedPathDenied,
        format!(
            "tool {} cannot write protected path {scoped_target}",
            policy.tool_id
        ),
    ))
}

fn ensure_real_workspace_write_path(
    workspace: &Path,
    target: &str,
    side_effect_recorder: SideEffectRecorder<'_>,
) -> Result<PathBuf, RuntimeError> {
    let mut parts = target.split('/').peekable();
    let mut path = workspace.to_path_buf();
    while let Some(part) = parts.next() {
        path.push(part);
        if parts.peek().is_some() && ensure_created_script_real_directory(&path)? {
            // WHY: a newly created parent directory is already a durable
            // workspace mutation even if the later leaf write fails.
            side_effect_recorder.mark_applied();
        }
    }
    Ok(path)
}

fn preflight_real_workspace_write_path(
    workspace: &Path,
    target: &str,
) -> Result<PathBuf, RuntimeError> {
    let mut parts = target.split('/').peekable();
    let mut path = workspace.to_path_buf();
    while let Some(part) = parts.next() {
        path.push(part);
        if parts.peek().is_some() {
            ensure_optional_script_real_directory(&path)?;
        }
    }
    Ok(path)
}

fn ensure_optional_script_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_optional_directory_with(path, DirectoryErrorMode::ScriptWrite)
}

fn ensure_created_script_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_created_directory_with(path, DirectoryErrorMode::ScriptWrite)
}

#[cfg(unix)]
fn ensure_opened_regular_leaf_matches_path(
    path: &Path,
    file: &fs::File,
) -> Result<(), RuntimeError> {
    let path_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    if path_metadata.file_type().is_symlink() {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
            path.display()
        )));
    }
    if !path_metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        )));
    }

    let file_metadata = file.metadata().map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    if !file_metadata.is_file() || !same_file_metadata(&path_metadata, &file_metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} changed before write",
            path.display()
        )));
    }
    ensure_not_hardlinked_file(path, &file_metadata)?;

    Ok(())
}

#[cfg(unix)]
fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(unix)]
fn hard_link_count(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    metadata.nlink()
}

fn normalize_script_write_target(target: &str) -> Result<String, RuntimeError> {
    // WHY: script write targets use one shared slash-only path policy across parser,
    // policy and runtime checks.
    if target.is_empty()
        || target.starts_with('/')
        || target.contains(':')
        || target.contains('\\')
        || target.contains('$')
        || target.contains('*')
        || target.contains('?')
    {
        return Err(RuntimeError::Protocol(format!(
            "own-script write target {target:?} must be a literal workspace-relative path"
        )));
    }
    if core_script::relative_path_has_windows_alias(target) {
        return Err(RuntimeError::Protocol(format!(
            "own-script write target {target:?} must not use a Windows path alias"
        )));
    }
    core_script::normalize_safe_relative_path(target).ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "own-script write target {target:?} must stay inside the workspace"
        ))
    })
}

fn evaluate_script_command(command: &str) -> Result<Vec<u8>, RuntimeError> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix("printf ") {
        evaluate_printf_command(rest)
    } else if let Some(rest) = command.strip_prefix("echo ") {
        let mut out = unquote_script_argument(rest.trim())?;
        out.push('\n');
        Ok(out.into_bytes())
    } else {
        Err(RuntimeError::Protocol(format!(
            "unsupported own-script command {command:?}"
        )))
    }
}

fn evaluate_printf_command(rest: &str) -> Result<Vec<u8>, RuntimeError> {
    let (format, rest) = parse_single_quoted_argument(rest.trim())?;
    let rest = rest.trim();
    let formatted = if rest.is_empty() {
        decode_printf_escapes(&format)?
    } else if matches!(rest, "\"$SUMMARY\"" | "$SUMMARY") {
        decode_printf_escapes(&format)?.replacen("%s", "hello", 1)
    } else {
        return Err(RuntimeError::Protocol(format!(
            "unsupported own-script printf argument {rest:?}"
        )));
    };
    Ok(formatted.into_bytes())
}

fn parse_single_quoted_argument(value: &str) -> Result<(String, &str), RuntimeError> {
    let Some(rest) = value.strip_prefix('\'') else {
        return Err(RuntimeError::Protocol(
            "own-script printf format must be single-quoted".to_owned(),
        ));
    };
    let Some(end) = rest.find('\'') else {
        return Err(RuntimeError::Protocol(
            "own-script printf format is unterminated".to_owned(),
        ));
    };
    Ok((rest[..end].to_owned(), &rest[end + 1..]))
}

fn decode_printf_escapes(value: &str) -> Result<String, RuntimeError> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                return Err(RuntimeError::Protocol(format!(
                    "unsupported own-script printf escape \\{other}"
                )));
            }
            None => {
                return Err(RuntimeError::Protocol(
                    "own-script printf format contains a dangling escape".to_owned(),
                ));
            }
        }
    }
    Ok(out)
}

fn unquote_script_argument(value: &str) -> Result<String, RuntimeError> {
    let unquoted = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    if unquoted.chars().any(|ch| matches!(ch, '$' | '`' | '\\')) {
        Err(RuntimeError::Protocol(format!(
            "unsupported own-script argument {value:?}"
        )))
    } else {
        Ok(unquoted.to_owned())
    }
}

fn emit_tool_progress(
    message: &'static str,
    tool: &core_script::ToolBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::ToolProgress,
        serde_json::json!({
            "message": message,
            "tool_id": tool.identity.id,
        }),
    )
}

fn ensure_writable_regular_leaf(path: &Path) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(runtime_denied(
            core_policy::DenyReasonCode::SymlinkEscapeDenied,
            format!("{} must not be a symlink", path.display()),
        )),
        Ok(metadata) if metadata.is_file() => {
            ensure_script_leaf_not_hardlinked(path, &metadata)?;
            Ok(true)
        }
        Ok(_) => Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("{} must be a file", path.display()),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

#[cfg(unix)]
fn ensure_script_leaf_not_hardlinked(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    if hard_link_count(metadata) > 1 {
        return Err(runtime_denied(
            core_policy::DenyReasonCode::WriteDenied,
            format!("{} must not be hard-linked", path.display()),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_script_leaf_not_hardlinked(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    Ok(())
}

fn emit_runtime_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    if failure.emit_tool_failed {
        emit_runtime_tool_failure(invocation, failure, builder)?;
    }
    emit_runtime_error(invocation, failure, builder)?;
    emit_runtime_loop_failure(loop_block, invocation, &failure.reason, builder)
}

fn emit_runtime_error(
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    let mut error_payload = serde_json::json!({
        "code": failure.reason,
        "message": failure.message,
    });
    if let Some(phase_id) = &failure.phase_id {
        let mut error_data = serde_json::Map::new();
        error_data.insert("phase_id".to_owned(), serde_json::json!(phase_id));
        if let Some(tool_id) = &failure.tool_id {
            error_data.insert("tool_id".to_owned(), serde_json::json!(tool_id));
        }
        let object = error_payload
            .as_object_mut()
            .expect("error payload is constructed as an object");
        object.insert("data".to_owned(), serde_json::Value::Object(error_data));
    }
    builder.emit(Some(invocation), EventType::Error, error_payload)
}

fn emit_runtime_loop_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    reason: &str,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::LoopFailed,
        serde_json::json!({
            "error": reason,
            "loop_definition_id": loop_block.identity.id,
        }),
    )
}

fn emit_runtime_tool_failure(
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    if let Some(tool_id) = &failure.tool_id {
        builder.emit(
            Some(invocation),
            EventType::ToolFailed,
            serde_json::json!({
                "error": failure.reason,
                "tool_id": tool_id,
            }),
        )?;
    }
    Ok(())
}

fn emit_propagated_runtime_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    emit_runtime_loop_failure(loop_block, invocation, &failure.reason, builder)
}

fn emit_runtime_error_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    err: &RuntimeError,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    let failure = runtime_failure_for_unhandled_error(err);
    emit_runtime_error(invocation, &failure, builder)?;
    emit_runtime_loop_failure(loop_block, invocation, &failure.reason, builder)
}

fn emit_propagated_runtime_error_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    emit_runtime_loop_failure(loop_block, invocation, RUNTIME_ERROR_REASON, builder)
}

fn sandbox_tool_dispatch_failure(
    tool: &core_script::ToolBlock,
    target: &core_policy::PolicyTarget,
    command_policy: &core_policy::CommandPolicy,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    ensure_tool_matches_policy(tool, target, command_policy)?;
    let Some(reason_code) = sandbox_negative_reason_for_tool(tool)? else {
        return Ok(None);
    };
    Ok(Some(runtime_failure_for_reason(
        reason_code,
        Some(tool.identity.id.clone()),
    )))
}

fn sandbox_out_of_phase_failure(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    phase: &core_script::PhaseBlock,
) -> Option<RuntimeFailure> {
    if !phase.tool_refs.is_empty()
        || !phase.identity.id.starts_with("negative-")
        || !phase.identity.id.contains("no-tools")
    {
        return None;
    }
    let unavailable_sentinel = registry
        .tools
        .values()
        .filter(|tool| {
            is_sandbox_negative_sentinel_tool(tool)
                && !policy_phase_contains_tool(policy, &phase.identity.id, &tool.identity.id)
        })
        .min_by_key(|tool| {
            if sandbox_negative_operation_for_tool(tool) == Some("write") {
                0
            } else {
                1
            }
        })?;
    Some(runtime_out_of_phase_failure(
        phase.identity.id.clone(),
        unavailable_sentinel.identity.id.clone(),
    ))
}

fn is_sandbox_negative_sentinel_tool(tool: &core_script::ToolBlock) -> bool {
    sandbox_negative_operation_for_tool(tool).is_some()
}

fn sandbox_negative_operation_for_tool(tool: &core_script::ToolBlock) -> Option<&str> {
    let (
        core_script::ToolKind::PredefinedCommand,
        core_script::ToolCommand::Predefined { command_id, argv },
    ) = (&tool.tool_kind, &tool.command)
    else {
        return None;
    };
    if command_id != "agent-negative" {
        return None;
    }
    let [operation] = argv.as_slice() else {
        return None;
    };
    sandbox_negative_reason_for_operation(operation).map(|_| operation.as_str())
}

fn sandbox_negative_reason_for_tool(
    tool: &core_script::ToolBlock,
) -> Result<Option<core_policy::DenyReasonCode>, RuntimeError> {
    let (
        core_script::ToolKind::PredefinedCommand,
        core_script::ToolCommand::Predefined { command_id, argv },
    ) = (&tool.tool_kind, &tool.command)
    else {
        return Ok(None);
    };
    if command_id != "agent-negative" {
        return Ok(None);
    }
    let [operation] = argv.as_slice() else {
        return Err(RuntimeError::Protocol(format!(
            "tool {} agent-negative command must declare one denied operation",
            tool.identity.id
        )));
    };
    sandbox_negative_reason_for_operation(operation)
        .map(Some)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "tool {} declares unsupported sandbox-negative operation {operation:?}",
                tool.identity.id
            ))
        })
}

fn sandbox_negative_reason_for_operation(operation: &str) -> Option<core_policy::DenyReasonCode> {
    match operation {
        "environment" => Some(core_policy::DenyReasonCode::EnvironmentDenied),
        "interpreter" => Some(core_policy::DenyReasonCode::InterpreterEscapeDenied),
        "network" => Some(core_policy::DenyReasonCode::NetworkDenied),
        "protected-path" => Some(core_policy::DenyReasonCode::ProtectedPathDenied),
        "symlink" => Some(core_policy::DenyReasonCode::SymlinkEscapeDenied),
        "write" => Some(core_policy::DenyReasonCode::WriteDenied),
        _ => None,
    }
}

fn runtime_denied(reason: core_policy::DenyReasonCode, message: String) -> RuntimeError {
    RuntimeError::Denied { reason, message }
}

fn runtime_protocol_or_denied(
    denied_reason: Option<core_policy::DenyReasonCode>,
    message: String,
) -> RuntimeError {
    match denied_reason {
        Some(reason) => runtime_denied(reason, message),
        None => RuntimeError::Protocol(message),
    }
}

fn runtime_failure_for_reason(
    reason_code: core_policy::DenyReasonCode,
    tool_id: Option<String>,
) -> RuntimeFailure {
    let emit_tool_failed = tool_id.is_some();
    RuntimeFailure {
        reason: reason_code.as_str().to_owned(),
        message: denial_message(reason_code),
        tool_id,
        phase_id: None,
        emit_tool_failed,
    }
}

fn runtime_failure_for_unhandled_error(err: &RuntimeError) -> RuntimeFailure {
    RuntimeFailure {
        reason: RUNTIME_ERROR_REASON.to_owned(),
        message: runtime_error_message(err),
        tool_id: None,
        phase_id: None,
        emit_tool_failed: false,
    }
}

fn runtime_error_message(_err: &RuntimeError) -> &'static str {
    "runtime execution failed"
}

fn runtime_failure_for_tool_error(err: &RuntimeError, tool_id: &str) -> Option<RuntimeFailure> {
    let reason = match err {
        RuntimeError::Denied { reason, .. } => reason.clone(),
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied => {
            core_policy::DenyReasonCode::WriteDenied
        }
        RuntimeError::Io { .. } => return None,
        RuntimeError::Json(_)
        | RuntimeError::Policy(_)
        | RuntimeError::Registry(_)
        | RuntimeError::Protocol(_)
        | RuntimeError::ActiveSession { .. }
        | RuntimeError::SessionLogExists(_)
        | RuntimeError::TerminalSession(_)
        | RuntimeError::Usage(_) => {
            return None;
        }
    };
    Some(runtime_failure_for_reason(reason, Some(tool_id.to_owned())))
}

fn runtime_out_of_phase_failure(phase_id: String, tool_id: String) -> RuntimeFailure {
    RuntimeFailure {
        reason: core_policy::DenyReasonCode::ToolOutOfPhase
            .as_str()
            .to_owned(),
        message: denial_message(core_policy::DenyReasonCode::ToolOutOfPhase),
        tool_id: Some(tool_id),
        phase_id: Some(phase_id),
        emit_tool_failed: false,
    }
}

fn policy_phase_contains_tool(
    policy: &core_policy::PolicyArtifact,
    phase_id: &str,
    tool_id: &str,
) -> bool {
    policy
        .phase_scope
        .iter()
        .any(|phase| phase.phase_id == phase_id && phase.tool_ids.iter().any(|id| id == tool_id))
}

fn denial_message(reason: core_policy::DenyReasonCode) -> &'static str {
    match reason {
        core_policy::DenyReasonCode::WriteDenied => "write outside declared roots denied",
        core_policy::DenyReasonCode::NetworkDenied => "network egress denied by default",
        core_policy::DenyReasonCode::EnvironmentDenied => "secret environment read denied",
        core_policy::DenyReasonCode::ToolOutOfPhase => "tool is not available in the active phase",
        core_policy::DenyReasonCode::ProtectedPathDenied => "protected path access denied",
        core_policy::DenyReasonCode::SymlinkEscapeDenied => "symlink escape denied",
        core_policy::DenyReasonCode::InterpreterEscapeDenied => "interpreter escape denied",
    }
}

fn canonical_event_stream(events: &[EventEnvelope]) -> Result<String, RuntimeError> {
    let mut stream = String::new();
    for event in events {
        stream.push_str(&event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize runtime event: {err}"))
        })?);
    }
    Ok(stream)
}

fn session_id_for_loop(loop_id: &str) -> String {
    if matches!(loop_id, "smoke-loop" | "hello-loop") {
        let base = loop_id
            .strip_suffix("-loop")
            .expect("fixture loop id ends with -loop");
        return session_id_from_token(base, loop_id);
    }
    if let Some(operation) = loop_id.strip_prefix("sandbox-negative-") {
        return session_id_from_token(&sandbox_negative_session_token(operation), loop_id);
    }
    session_id_from_token(loop_id, loop_id)
}

fn sandbox_negative_session_token(operation: &str) -> String {
    let mut token = String::from("neg");
    for word in operation.split('-') {
        match word {
            "environment" => token.push_str("env"),
            "interpreter" => token.push_str("interp"),
            "network" => token.push_str("net"),
            "path" | "symlink" | "write" => token.push_str(word),
            "phase" => token.push_str("phase"),
            "of" | "out" | "protected" | "tool" => {}
            other => token.push_str(other),
        }
    }
    token
}

fn session_id_from_token(token: &str, stable_source: &str) -> String {
    let mut token = token.to_ascii_lowercase();
    token.retain(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-');
    if token.is_empty() {
        token.push_str("session");
    }
    let suffix = if token.len() <= 125 {
        "001".to_owned()
    } else {
        format!("-{:016x}001", stable_hash64(stable_source.as_bytes()))
    };
    token.truncate(128 - suffix.len());
    token.push_str(&suffix);
    token
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn tool_kind_name(kind: &core_script::ToolKind) -> &'static str {
    match kind {
        core_script::ToolKind::PredefinedCommand => "predefined-command",
        core_script::ToolKind::OwnScript => "own-script",
    }
}

fn policy_tool_kind_name(kind: &core_policy::ToolKind) -> &'static str {
    match kind {
        core_policy::ToolKind::PredefinedCommand => "predefined-command",
        core_policy::ToolKind::OwnScript => "own-script",
    }
}

fn tool_network_access_name(policy: &core_script::NetworkPolicy) -> &'static str {
    match policy {
        core_script::NetworkPolicy::Deny(_) => "deny",
        core_script::NetworkPolicy::Declared { .. } => "declared",
    }
}

fn connection_kind_name(kind: &core_script::ConnectionKind) -> &'static str {
    match kind {
        core_script::ConnectionKind::Data => "data",
        core_script::ConnectionKind::Trigger => "trigger",
        core_script::ConnectionKind::Refresh => "refresh",
    }
}

fn load_workspace_config(workspace: &Path) -> Result<WorkspaceConfig, RuntimeError> {
    let path = workspace.join(".loop/config.yaml");
    let text = read_workspace_config_to_string(&path)?;
    let registry_root = config_value(&text, "registry_root")
        .ok_or_else(|| RuntimeError::Usage("missing .loop/config.yaml registry_root".to_owned()))?;
    let registry_root = PathBuf::from(registry_root);
    if registry_root.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
                | std::path::Component::RootDir
        )
    }) {
        return Err(RuntimeError::Usage(
            ".loop/config.yaml registry_root must stay within the workspace".to_owned(),
        ));
    }
    let event_clock = workspace_event_clock(&text)?;
    Ok(WorkspaceConfig {
        event_clock,
        registry_root,
    })
}

fn registry_root_path(workspace: &Path, registry_root: &Path) -> Result<PathBuf, RuntimeError> {
    let mut path = workspace.to_path_buf();
    for component in registry_root.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(segment) => {
                path.push(segment);
                let metadata = fs::symlink_metadata(&path).map_err(|source| RuntimeError::Io {
                    path: path.clone(),
                    source,
                })?;
                if metadata.file_type().is_symlink() || has_windows_reparse_point(&metadata) {
                    return Err(RuntimeError::Usage(
                        ".loop/config.yaml registry_root must not contain symlinks or reparse points"
                            .to_owned(),
                    ));
                }
                if !metadata.is_dir() {
                    return Err(RuntimeError::Usage(
                        ".loop/config.yaml registry_root must resolve through directories"
                            .to_owned(),
                    ));
                }
            }
            std::path::Component::ParentDir
            | std::path::Component::Prefix(_)
            | std::path::Component::RootDir => {
                return Err(RuntimeError::Usage(
                    ".loop/config.yaml registry_root must stay within the workspace".to_owned(),
                ));
            }
        }
    }
    Ok(path)
}

fn config_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for raw_line in text.lines() {
        let line = strip_config_comment(raw_line);
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = unquote_config_scalar(value.trim());
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn strip_config_comment(line: &str) -> &str {
    let mut in_double_quotes = false;
    let mut in_single_quotes = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' if !in_single_quotes => in_double_quotes = !in_double_quotes,
            '\'' if !in_double_quotes => in_single_quotes = !in_single_quotes,
            '#' if !in_double_quotes && !in_single_quotes => return &line[..index],
            _ => {}
        }
    }
    line
}

fn unquote_config_scalar(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1].replace("\\\"", "\"");
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    value.to_owned()
}

#[derive(Debug)]
struct WorkspaceConfig {
    event_clock: EventClock,
    registry_root: PathBuf,
}

fn workspace_event_clock(text: &str) -> Result<EventClock, RuntimeError> {
    match (
        config_value(text, "fixture_profile"),
        config_value(text, "stub_model"),
    ) {
        (Some(profile), Some(model)) if profile == "stub-model" && model == "deterministic" => {
            Ok(EventClock::fixed_fixture())
        }
        (Some(profile), None) if profile == "stub-model" => Err(RuntimeError::Usage(
            ".loop/config.yaml fixture_profile stub-model requires stub_model: deterministic"
                .to_owned(),
        )),
        (Some(profile), _) if profile != "stub-model" => Err(RuntimeError::Usage(format!(
            "unsupported .loop/config.yaml fixture_profile {profile:?}"
        ))),
        (None, Some(model)) if model == "deterministic" => Err(RuntimeError::Usage(
            ".loop/config.yaml stub_model deterministic requires fixture_profile: stub-model"
                .to_owned(),
        )),
        (_, Some(model)) if model != "deterministic" => Err(RuntimeError::Usage(format!(
            "unsupported .loop/config.yaml stub_model {model:?}"
        ))),
        _ => Ok(EventClock::wall_clock()),
    }
}

fn resume_event_clock(
    config: &WorkspaceConfig,
    events: &[EventEnvelope],
) -> Result<EventClock, RuntimeError> {
    if config.event_clock == EventClock::fixed_fixture() {
        return Ok(config.event_clock);
    }
    let first_event = events
        .first()
        .expect("validated streams contain at least one event");
    EventClock::from_first_event(first_event).ok_or_else(|| {
        RuntimeError::Protocol("session first event timestamp cannot anchor resume".to_owned())
    })
}

fn read_workspace_config_to_string(path: &Path) -> Result<String, RuntimeError> {
    ensure_real_file(path)?;
    read_to_string_with_limit(path, MAX_WORKSPACE_CONFIG_BYTES)
}

fn read_session_log_to_string(path: &Path) -> Result<String, RuntimeError> {
    read_to_string_with_limit(path, MAX_SESSION_LOG_BYTES)
}

fn session_log_len(path: &Path) -> Result<usize, RuntimeError> {
    let (_file, metadata) = open_real_file_for_read(path)?;
    let len = metadata.len();
    if len > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {len} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    usize::try_from(len).map_err(|_| {
        RuntimeError::Protocol(format!(
            "{} read size {len} bytes exceeds addressable memory",
            path.display()
        ))
    })
}

fn tail_session_log_len(path: &Path) -> Result<usize, RuntimeError> {
    retry_tail_transient_read_error(|| session_log_len(path))
}

fn read_to_string_with_limit(path: &Path, max_bytes: u64) -> Result<String, RuntimeError> {
    let bytes = read_file_range(path, 0, max_bytes)?;
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

fn open_real_file_for_read(path: &Path) -> Result<(fs::File, fs::Metadata), RuntimeError> {
    let expected_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    validate_real_file(path, &expected_metadata)?;
    let file =
        open_file_for_read_without_following_reparse(path).map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let file_metadata =
        ensure_opened_real_file_for_read_matches_path(path, &expected_metadata, &file)?;
    Ok((file, file_metadata))
}

#[cfg(windows)]
fn open_file_for_read_without_following_reparse(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(windows))]
fn open_file_for_read_without_following_reparse(path: &Path) -> io::Result<fs::File> {
    fs::File::open(path)
}

fn ensure_opened_real_file_for_read_matches_path(
    path: &Path,
    expected_metadata: &fs::Metadata,
    file: &fs::File,
) -> Result<fs::Metadata, RuntimeError> {
    #[cfg(not(unix))]
    let _ = expected_metadata;

    let current_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    validate_real_file(path, &current_metadata)?;

    let file_metadata = file.metadata().map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    validate_real_file(path, &file_metadata)?;

    #[cfg(unix)]
    if !same_file_metadata(expected_metadata, &current_metadata)
        || !same_file_metadata(&current_metadata, &file_metadata)
    {
        return Err(RuntimeError::Protocol(format!(
            "{} changed before read",
            path.display()
        )));
    }

    Ok(file_metadata)
}

fn read_file_suffix_to_string(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<String, RuntimeError> {
    if expected_len < offset {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    let suffix_len = expected_len - offset;
    let (mut file, metadata) = open_real_file_for_read(path)?;
    let total_len = metadata.len();
    if total_len > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    let expected_len = u64::try_from(expected_len).map_err(|_| {
        RuntimeError::Protocol(format!(
            "{} read size {expected_len} bytes exceeds addressable memory",
            path.display()
        ))
    })?;
    if total_len < expected_len {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    let offset = u64::try_from(offset).unwrap_or(u64::MAX);
    let suffix_len = u64::try_from(suffix_len).unwrap_or(u64::MAX);
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = Vec::new();
    file.take(suffix_len)
        .read_to_end(&mut bytes)
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != suffix_len {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

fn read_tail_file_suffix_to_string(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<String, RuntimeError> {
    retry_tail_transient_read_error(|| read_file_suffix_to_string(path, offset, expected_len))
}

fn retry_tail_transient_read_error<T>(
    mut operation: impl FnMut() -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    for attempt in 0..=TAIL_TRANSIENT_READ_RETRY_ATTEMPTS {
        match operation() {
            Err(err)
                if runtime_error_is_transient_tail_read(&err)
                    && attempt < TAIL_TRANSIENT_READ_RETRY_ATTEMPTS =>
            {
                thread::sleep(Duration::from_millis(TAIL_TRANSIENT_READ_RETRY_MS));
            }
            result => return result,
        }
    }
    unreachable!("tail transient retry loop always returns")
}

fn runtime_error_is_transient_tail_read(err: &RuntimeError) -> bool {
    matches!(
        err,
        RuntimeError::Io { source, .. }
            if matches!(
                source.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            )
    )
}

fn read_file_range(path: &Path, offset: u64, max_bytes: u64) -> Result<Vec<u8>, RuntimeError> {
    let (mut file, metadata) = open_real_file_for_read(path)?;
    let total_len = metadata.len();
    if total_len > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    if offset > total_len {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    let available = total_len - offset;
    if available > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {available} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let bytes_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes_len > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {bytes_len} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(any(not(unix), test))]
fn read_to_bytes(path: &Path) -> Result<Vec<u8>, RuntimeError> {
    fs::read(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Validates public v0 event JSONL canonical bytes, envelope fields, payload
/// contracts and session lifecycle ordering.
pub fn validate_protocol_jsonl_text(
    path: &Path,
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    if !text.ends_with('\n') {
        return Err(RuntimeError::Protocol(format!(
            "{} must end with LF",
            path.display()
        )));
    }

    let mut previous_sequence = 0;
    let mut session_id = None::<String>;
    let mut event_ids = BTreeSet::new();
    let mut loop_started_ids = BTreeSet::new();
    let mut terminal_line = None::<usize>;
    let mut events = Vec::new();
    let mut stream_bytes = 0usize;
    for (index, line) in text.split_terminator('\n').enumerate() {
        let line_number = index + 1;
        // WHY: count JSONL bytes and events before parsing payloads so oversized streams
        // fail cheaply and deterministically.
        if u64::try_from(line_number).unwrap_or(u64::MAX) > MAX_LOOP_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "{} runtime event budget exceeded at line {line_number}: max {MAX_LOOP_EVENTS}",
                path.display()
            )));
        }
        stream_bytes = stream_bytes
            .checked_add(line.len().saturating_add(1))
            .unwrap_or(usize::MAX);
        if stream_bytes > MAX_LOOP_EVENT_STREAM_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} event stream budget exceeded at line {line_number}: {stream_bytes} bytes exceeds max {MAX_LOOP_EVENT_STREAM_BYTES}",
                path.display()
            )));
        }
        if line.ends_with('\r') {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use LF-only line endings",
                path.display()
            )));
        }
        let event: EventEnvelope = serde_json::from_str(line)?;
        let canonical = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("{} line {line_number}: {err}", path.display()))
        })?;
        if canonical != format!("{line}\n") {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use canonical JSONL bytes",
                path.display()
            )));
        }
        if !validate_session_id(&event.session_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a valid session_id",
                path.display()
            )));
        }
        if event.event_id.is_empty() {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a non-empty event_id",
                path.display()
            )));
        }
        if event.source.is_empty() {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a non-empty source",
                path.display()
            )));
        }
        if !is_rfc3339_utc_timestamp(&event.timestamp) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use an RFC3339 UTC timestamp",
                path.display()
            )));
        }
        if event
            .correlation_id
            .as_ref()
            .is_some_and(|correlation_id| correlation_id.is_empty())
        {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a non-empty correlation_id",
                path.display()
            )));
        }
        if event
            .loop_id
            .as_ref()
            .is_some_and(|loop_id| loop_id.is_empty())
        {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a non-empty loop_id",
                path.display()
            )));
        }
        if event
            .parent_loop_id
            .as_ref()
            .is_some_and(|parent_loop_id| parent_loop_id.is_empty())
        {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a non-empty parent_loop_id",
                path.display()
            )));
        }
        if line_number == 1 && event.sequence != 1 {
            return Err(RuntimeError::Protocol(format!(
                "{} first sequence must be 1",
                path.display()
            )));
        }
        if event.sequence <= previous_sequence {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} sequence must increase",
                path.display()
            )));
        }
        previous_sequence = event.sequence;
        if !event_ids.insert(event.event_id.clone()) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a unique event_id",
                path.display()
            )));
        }
        if let Some(terminal_line) = terminal_line {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} appears after terminal session event on line {terminal_line}",
                path.display()
            )));
        }
        validate_event_payload(path, line_number, &event)?;
        if event.event_type == EventType::LoopStarted {
            let loop_id = event.loop_id.as_deref().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "{} line {line_number} loop.started must include loop_id",
                    path.display()
                ))
            })?;
            if !loop_started_ids.insert(loop_id.to_owned()) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a unique loop_id for loop.started",
                    path.display()
                )));
            }
        }
        match &session_id {
            Some(existing) if existing != &event.session_id => {
                return Err(RuntimeError::Protocol(format!(
                    "{} must use one session_id",
                    path.display()
                )));
            }
            None => session_id = Some(event.session_id.clone()),
            Some(_) => {}
        }
        if matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        ) {
            terminal_line = Some(line_number);
        }
        events.push(event);
    }
    if events.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "{} must contain at least one event",
            path.display()
        )));
    }
    validate_session_lifecycle(path, &events)?;
    Ok(events)
}

fn validate_event_payload(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<(), RuntimeError> {
    let payload = event.payload.as_object().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} payload must be an object",
            path.display(),
            event.event_type.as_str()
        ))
    })?;

    match event.event_type {
        EventType::SessionStarted
        | EventType::SessionPaused
        | EventType::SessionResumed
        | EventType::SessionCompleted => {
            optional_payload_string(path, line_number, event.event_type, payload, "reason")?;
        }
        EventType::SessionFailed => {
            require_payload_string(path, line_number, event.event_type, payload, "reason")?;
        }
        EventType::LoopStarted | EventType::LoopCompleted => {
            require_payload_string(
                path,
                line_number,
                event.event_type,
                payload,
                "loop_definition_id",
            )?;
            optional_payload_string(path, line_number, event.event_type, payload, "loop_name")?;
        }
        EventType::LoopFailed => {
            require_payload_string(
                path,
                line_number,
                event.event_type,
                payload,
                "loop_definition_id",
            )?;
            optional_payload_string(path, line_number, event.event_type, payload, "loop_name")?;
            require_payload_string(path, line_number, event.event_type, payload, "error")?;
        }
        EventType::PhaseEntered => {
            require_payload_string(path, line_number, event.event_type, payload, "phase_id")?;
            require_payload_string(path, line_number, event.event_type, payload, "phase_name")?;
            require_payload_string_array(
                path,
                line_number,
                event.event_type,
                payload,
                "instruction_ids",
            )?;
            require_payload_string_array(path, line_number, event.event_type, payload, "tool_ids")?;
        }
        EventType::StepStarted | EventType::StepCompleted => {
            require_payload_string(path, line_number, event.event_type, payload, "step_id")?;
            require_payload_string(path, line_number, event.event_type, payload, "step_name")?;
            optional_payload_string(path, line_number, event.event_type, payload, "phase_id")?;
            optional_payload_string(
                path,
                line_number,
                event.event_type,
                payload,
                "instruction_id",
            )?;
            let connection_ids = optional_payload_string_array(
                path,
                line_number,
                event.event_type,
                payload,
                "connection_ids",
            )?;
            let connection_kinds = optional_payload_string_array(
                path,
                line_number,
                event.event_type,
                payload,
                "connection_kinds",
            )?;
            match (connection_ids, connection_kinds) {
                (Some(ids), Some(kinds)) => {
                    if ids.len() != kinds.len() {
                        return Err(payload_contract_error(
                            path,
                            line_number,
                            event.event_type,
                            "payload connection arrays must have the same length",
                        ));
                    }
                    for kind in kinds {
                        if !matches!(kind, "data" | "trigger" | "refresh") {
                            return Err(payload_contract_error(
                                path,
                                line_number,
                                event.event_type,
                                "payload.connection_kinds values must be data, trigger, or refresh",
                            ));
                        }
                    }
                }
                (None, None) => {}
                _ => {
                    return Err(payload_contract_error(
                        path,
                        line_number,
                        event.event_type,
                        "payload connection arrays must be present together",
                    ));
                }
            }
        }
        EventType::MessageDelta => {
            require_payload_string(path, line_number, event.event_type, payload, "message_id")?;
            require_payload_role(path, line_number, event.event_type, payload)?;
            require_payload_string(
                path,
                line_number,
                event.event_type,
                payload,
                "content_delta",
            )?;
        }
        EventType::MessageCompleted => {
            require_payload_string(path, line_number, event.event_type, payload, "message_id")?;
            require_payload_role(path, line_number, event.event_type, payload)?;
        }
        EventType::ToolStarted => {
            require_payload_string(path, line_number, event.event_type, payload, "tool_id")?;
            require_payload_string(path, line_number, event.event_type, payload, "tool_name")?;
            let tool_kind =
                require_payload_string(path, line_number, event.event_type, payload, "tool_kind")?;
            if !matches!(tool_kind, "predefined-command" | "own-script") {
                return Err(payload_contract_error(
                    path,
                    line_number,
                    event.event_type,
                    "payload.tool_kind must be predefined-command or own-script",
                ));
            }
            require_payload_string_array(
                path,
                line_number,
                event.event_type,
                payload,
                "read_scope",
            )?;
            require_payload_string_array(
                path,
                line_number,
                event.event_type,
                payload,
                "write_scope",
            )?;
            require_payload_string_array(
                path,
                line_number,
                event.event_type,
                payload,
                "allowed_parameters",
            )?;
            let network_access = require_payload_string(
                path,
                line_number,
                event.event_type,
                payload,
                "network_access",
            )?;
            if !matches!(network_access, "deny" | "declared") {
                return Err(payload_contract_error(
                    path,
                    line_number,
                    event.event_type,
                    "payload.network_access must be deny or declared",
                ));
            }
        }
        EventType::ToolProgress => {
            require_payload_string(path, line_number, event.event_type, payload, "tool_id")?;
            require_payload_string(path, line_number, event.event_type, payload, "message")?;
        }
        EventType::ToolCompleted => {
            require_payload_string(path, line_number, event.event_type, payload, "tool_id")?;
            optional_payload_integer(path, line_number, event.event_type, payload, "exit_code")?;
        }
        EventType::ToolFailed | EventType::ToolTimedOut => {
            require_payload_string(path, line_number, event.event_type, payload, "tool_id")?;
            require_payload_string(path, line_number, event.event_type, payload, "error")?;
        }
        EventType::ArtifactLogged => {
            require_payload_string(path, line_number, event.event_type, payload, "artifact_id")?;
            require_payload_string(
                path,
                line_number,
                event.event_type,
                payload,
                "artifact_type",
            )?;
            require_payload_string(path, line_number, event.event_type, payload, "uri")?;
        }
        EventType::AttentionRequested => {
            require_payload_string(path, line_number, event.event_type, payload, "request_id")?;
            require_payload_string(path, line_number, event.event_type, payload, "reason")?;
        }
        EventType::MetricSample => {
            require_payload_string(path, line_number, event.event_type, payload, "metric_name")?;
            require_payload_number(path, line_number, event.event_type, payload, "value")?;
        }
        EventType::Error => {
            require_payload_string(path, line_number, event.event_type, payload, "code")?;
            require_payload_string(path, line_number, event.event_type, payload, "message")?;
            optional_payload_object(path, line_number, event.event_type, payload, "data")?;
        }
    }

    Ok(())
}

fn require_payload_string<'a>(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<&'a str, RuntimeError> {
    match payload.get(field).and_then(serde_json::Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(payload_contract_error(
            path,
            line_number,
            event_type,
            &format!("payload.{field} must be a non-empty string"),
        )),
    }
}

fn optional_payload_string(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<(), RuntimeError> {
    if payload.contains_key(field) {
        require_payload_string(path, line_number, event_type, payload, field)?;
    }
    Ok(())
}

fn require_payload_role(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), RuntimeError> {
    let role = require_payload_string(path, line_number, event_type, payload, "role")?;
    if matches!(role, "system" | "user" | "assistant" | "tool") {
        Ok(())
    } else {
        Err(payload_contract_error(
            path,
            line_number,
            event_type,
            "payload.role must be system, user, assistant, or tool",
        ))
    }
}

fn require_payload_string_array<'a>(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Vec<&'a str>, RuntimeError> {
    let Some(value) = payload.get(field) else {
        return Err(payload_contract_error(
            path,
            line_number,
            event_type,
            &format!("payload.{field} must be a string array"),
        ));
    };
    payload_string_array(path, line_number, event_type, field, value)
}

fn optional_payload_string_array<'a>(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<Vec<&'a str>>, RuntimeError> {
    payload
        .get(field)
        .map(|value| payload_string_array(path, line_number, event_type, field, value))
        .transpose()
}

fn payload_string_array<'a>(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    field: &str,
    value: &'a serde_json::Value,
) -> Result<Vec<&'a str>, RuntimeError> {
    let Some(values) = value.as_array() else {
        return Err(payload_contract_error(
            path,
            line_number,
            event_type,
            &format!("payload.{field} must be a string array"),
        ));
    };
    values
        .iter()
        .map(|value| {
            value.as_str().ok_or_else(|| {
                payload_contract_error(
                    path,
                    line_number,
                    event_type,
                    &format!("payload.{field} must contain only strings"),
                )
            })
        })
        .collect()
}

fn optional_payload_integer(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<(), RuntimeError> {
    if let Some(value) = payload.get(field) {
        let Some(number) = value.as_number() else {
            return Err(payload_contract_error(
                path,
                line_number,
                event_type,
                &format!("payload.{field} must be an integer"),
            ));
        };
        if number.as_i64().is_none() && number.as_u64().is_none() {
            return Err(payload_contract_error(
                path,
                line_number,
                event_type,
                &format!("payload.{field} must be an integer"),
            ));
        }
    }
    Ok(())
}

fn require_payload_number(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<(), RuntimeError> {
    if payload.get(field).is_some_and(serde_json::Value::is_number) {
        Ok(())
    } else {
        Err(payload_contract_error(
            path,
            line_number,
            event_type,
            &format!("payload.{field} must be a number"),
        ))
    }
}

fn optional_payload_object(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    payload: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<(), RuntimeError> {
    if payload.get(field).is_some_and(|value| !value.is_object()) {
        Err(payload_contract_error(
            path,
            line_number,
            event_type,
            &format!("payload.{field} must be an object"),
        ))
    } else {
        Ok(())
    }
}

fn payload_contract_error(
    path: &Path,
    line_number: usize,
    event_type: EventType,
    message: &str,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} {message}",
        path.display(),
        event_type.as_str()
    ))
}

#[cfg(any(test, doctest))]
fn validate_appended_session_log_text(
    path: &Path,
    expected_session_id: &str,
    prior_events: &[EventEnvelope],
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    if prior_events.is_empty() {
        return validate_session_log_text(path, expected_session_id, text);
    }
    let mut stream_bytes = 0usize;
    for event in prior_events {
        let canonical = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("{} prior event stream: {err}", path.display()))
        })?;
        stream_bytes = stream_bytes
            .checked_add(canonical.len())
            .unwrap_or(usize::MAX);
    }
    let mut state = SessionAppendValidationState::from_prior_events(
        path,
        expected_session_id,
        prior_events,
        stream_bytes,
    )?;
    state.validate_appended(path, text)
}

struct SessionAppendValidationState {
    expected_session_id: String,
    previous_sequence: u64,
    event_ids: BTreeSet<String>,
    loop_started_ids: BTreeSet<String>,
    terminal_line: Option<usize>,
    stream_bytes: usize,
    line_count: usize,
    lifecycle: SessionLifecycleState,
}

impl SessionAppendValidationState {
    fn from_prior_events(
        path: &Path,
        expected_session_id: &str,
        prior_events: &[EventEnvelope],
        stream_bytes: usize,
    ) -> Result<Self, RuntimeError> {
        let prior_session_id = &prior_events
            .first()
            .expect("prior events are non-empty")
            .session_id;
        if prior_events
            .first()
            .expect("prior events are non-empty")
            .event_type
            != EventType::SessionStarted
        {
            return Err(RuntimeError::Protocol(format!(
                "{} line 1 must start with session.started",
                path.display()
            )));
        }
        if prior_session_id != expected_session_id {
            return Err(RuntimeError::Protocol(format!(
                "{} contains session_id {prior_session_id:?}, expected {expected_session_id:?}",
                path.display()
            )));
        }

        let mut lifecycle = SessionLifecycleState::default();
        for (index, event) in prior_events.iter().enumerate() {
            lifecycle.validate_event(path, index + 1, event)?;
        }
        lifecycle.validate_terminal_session(path, prior_events.last())?;

        let terminal_line = prior_events
            .iter()
            .position(|event| {
                matches!(
                    event.event_type,
                    EventType::SessionCompleted | EventType::SessionFailed
                )
            })
            .map(|index| index + 1);

        Ok(Self {
            expected_session_id: expected_session_id.to_owned(),
            previous_sequence: prior_events
                .last()
                .expect("prior events are non-empty")
                .sequence,
            event_ids: prior_events
                .iter()
                .map(|event| event.event_id.clone())
                .collect(),
            loop_started_ids: prior_events
                .iter()
                .filter(|event| event.event_type == EventType::LoopStarted)
                .filter_map(|event| event.loop_id.clone())
                .collect(),
            terminal_line,
            stream_bytes,
            line_count: prior_events.len(),
            lifecycle,
        })
    }

    fn validate_appended(
        &mut self,
        path: &Path,
        text: &str,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        if !text.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} appended suffix must end with LF",
                path.display()
            )));
        }

        let mut appended_events = Vec::new();
        for line in text.split_terminator('\n') {
            let line_number = self.line_count + 1;
            // WHY: incremental tail validation must preserve the same cumulative
            // public stream budgets as full replay validation.
            if u64::try_from(line_number).unwrap_or(u64::MAX) > MAX_LOOP_EVENTS {
                return Err(RuntimeError::Protocol(format!(
                    "{} runtime event budget exceeded at line {line_number}: max {MAX_LOOP_EVENTS}",
                    path.display()
                )));
            }
            self.stream_bytes = self
                .stream_bytes
                .checked_add(line.len().saturating_add(1))
                .unwrap_or(usize::MAX);
            if self.stream_bytes > MAX_LOOP_EVENT_STREAM_BYTES {
                return Err(RuntimeError::Protocol(format!(
                    "{} event stream budget exceeded at line {line_number}: {} bytes exceeds max {MAX_LOOP_EVENT_STREAM_BYTES}",
                    path.display(),
                    self.stream_bytes
                )));
            }
            if line.ends_with('\r') {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use LF-only line endings",
                    path.display()
                )));
            }
            let event: EventEnvelope = serde_json::from_str(line)?;
            let canonical = event.canonical_jsonl().map_err(|err| {
                RuntimeError::Protocol(format!("{} line {line_number}: {err}", path.display()))
            })?;
            if canonical != format!("{line}\n") {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use canonical JSONL bytes",
                    path.display()
                )));
            }
            if event.session_id != self.expected_session_id {
                return Err(RuntimeError::Protocol(format!(
                    "{} must use one session_id",
                    path.display()
                )));
            }
            if !validate_session_id(&event.session_id) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a valid session_id",
                    path.display()
                )));
            }
            if event.event_id.is_empty() {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a non-empty event_id",
                    path.display()
                )));
            }
            if event.source.is_empty() {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a non-empty source",
                    path.display()
                )));
            }
            if !is_rfc3339_utc_timestamp(&event.timestamp) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use an RFC3339 UTC timestamp",
                    path.display()
                )));
            }
            if event
                .correlation_id
                .as_ref()
                .is_some_and(|correlation_id| correlation_id.is_empty())
            {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a non-empty correlation_id",
                    path.display()
                )));
            }
            if event
                .loop_id
                .as_ref()
                .is_some_and(|loop_id| loop_id.is_empty())
            {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a non-empty loop_id",
                    path.display()
                )));
            }
            if event
                .parent_loop_id
                .as_ref()
                .is_some_and(|parent_loop_id| parent_loop_id.is_empty())
            {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a non-empty parent_loop_id",
                    path.display()
                )));
            }
            if event.sequence <= self.previous_sequence {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} sequence must increase",
                    path.display()
                )));
            }
            self.previous_sequence = event.sequence;
            if !self.event_ids.insert(event.event_id.clone()) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a unique event_id",
                    path.display()
                )));
            }
            if let Some(terminal_line) = self.terminal_line {
                return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} appears after terminal session event on line {terminal_line}",
                path.display()
            )));
            }
            validate_event_payload(path, line_number, &event)?;
            if event.event_type == EventType::LoopStarted {
                let loop_id = event.loop_id.as_deref().ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "{} line {line_number} loop.started must include loop_id",
                        path.display()
                    ))
                })?;
                if !self.loop_started_ids.insert(loop_id.to_owned()) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} must use a unique loop_id for loop.started",
                        path.display()
                    )));
                }
            }
            self.lifecycle.validate_event(path, line_number, &event)?;
            if matches!(
                event.event_type,
                EventType::SessionCompleted | EventType::SessionFailed
            ) {
                self.terminal_line = Some(line_number);
            }
            self.line_count = line_number;
            let is_terminal = matches!(
                event.event_type,
                EventType::SessionCompleted | EventType::SessionFailed
            );
            appended_events.push(event);
            if is_terminal {
                self.lifecycle
                    .validate_terminal_session(path, appended_events.last())?;
            }
        }
        Ok(appended_events)
    }
}

fn validate_session_log_text(
    path: &Path,
    expected_session_id: &str,
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    let events = validate_protocol_jsonl_text(path, text)?;
    let actual_session_id = &events
        .first()
        .expect("validated streams contain at least one event")
        .session_id;
    if actual_session_id != expected_session_id {
        return Err(RuntimeError::Protocol(format!(
            "{} contains session_id {actual_session_id:?}, expected {expected_session_id:?}",
            path.display()
        )));
    }
    Ok(events)
}

/// Validates lifecycle invariants after envelope and payload validation:
/// every loop/step/tool/message must start before use, terminal lifecycle
/// items cannot receive later events, and terminal sessions cannot leave open
/// lifecycle items.
fn validate_session_lifecycle(path: &Path, events: &[EventEnvelope]) -> Result<(), RuntimeError> {
    if events
        .first()
        .expect("validated streams contain at least one event")
        .event_type
        != EventType::SessionStarted
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line 1 must start with session.started",
            path.display()
        )));
    }

    let mut state = SessionLifecycleState::default();

    for (index, event) in events.iter().enumerate() {
        let line_number = index + 1;
        state.validate_event(path, line_number, event)?;
    }

    state.validate_terminal_session(path, events.last())?;
    Ok(())
}

#[derive(Default)]
struct SessionLifecycleState {
    loops: LifecycleTracker<String>,
    loop_parents: BTreeMap<String, Option<String>>,
    steps: LifecycleTracker<StepLifecycleKey>,
    tools: LifecycleTracker<ToolLifecycleKey>,
    messages: LifecycleTracker<MessageLifecycleKey>,
    active_message_roles: BTreeMap<MessageLifecycleKey, String>,
    active_phases: BTreeMap<String, String>,
    active_steps: BTreeMap<String, StepLifecycleKey>,
}

impl SessionLifecycleState {
    fn validate_event(
        &mut self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<(), RuntimeError> {
        if line_number > 1 && event.event_type == EventType::SessionStarted {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} session.started is only valid as the first event",
                path.display()
            )));
        }

        if event.event_type != EventType::LoopStarted {
            if let Some(loop_id) = &event.loop_id {
                if !self.loops.is_started(loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow loop.started for loop_id {loop_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                if let Some(terminal_line) = self.loops.terminal_line(loop_id) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "loop",
                        loop_id,
                        terminal_line,
                    ));
                }
            }
        }
        validate_lifecycle_parent(path, line_number, event, &self.loops, &self.loop_parents)?;

        match event.event_type {
            EventType::LoopStarted => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                self.loop_parents
                    .insert(loop_id.clone(), event.parent_loop_id.clone());
                self.loops.start(loop_id);
            }
            EventType::LoopCompleted | EventType::LoopFailed => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if !self.loops.is_started(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow loop.started for loop_id {loop_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                self.loops.finish(loop_id, line_number);
            }
            EventType::PhaseEntered => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} phase.entered requires no active step for loop_id {:?}; active step_id {:?}",
                        path.display(),
                        loop_id,
                        active_step.step_id
                    )));
                }
                self.active_phases
                    .insert(loop_id, lifecycle_payload_string(event, "phase_id"));
            }
            EventType::StepStarted => {
                let active_phase =
                    require_active_phase(path, line_number, event, &self.active_phases)?;
                let step = lifecycle_step_key(event, &self.active_phases);
                if step.phase_id.as_deref() != Some(active_phase.as_str()) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started phase_id {:?} must match active phase {:?}",
                        path.display(),
                        step.phase_id,
                        active_phase
                    )));
                }
                if let Some(terminal_line) = self.steps.terminal_line(&step) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started requires no active step for loop_id {:?}; active step_id {:?}",
                        path.display(),
                        loop_id,
                        active_step.step_id
                    )));
                }
                self.active_steps.insert(loop_id, step.clone());
                self.steps.start(step);
            }
            EventType::StepCompleted => {
                let step = lifecycle_step_key(event, &self.active_phases);
                if let Some(terminal_line) = self.steps.terminal_line(&step) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                if !self.steps.is_started(&step) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.completed must follow step.started for step_id {:?}",
                        path.display(),
                        step.step_id
                    )));
                }
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                match self.active_steps.get(&loop_id) {
                    Some(active_step) if active_step == &step => {}
                    Some(active_step) => {
                        return Err(RuntimeError::Protocol(format!(
                            "{} line {line_number} step.completed requires active step_id {:?}, found {:?}",
                            path.display(),
                            step.step_id,
                            active_step.step_id
                        )));
                    }
                    None => {
                        return Err(RuntimeError::Protocol(format!(
                            "{} line {line_number} step.completed requires active step for step_id {:?}",
                            path.display(),
                            step.step_id
                        )));
                    }
                }
                self.active_steps.remove(&loop_id);
                self.steps.finish(step, line_number);
            }
            EventType::ToolStarted => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                self.tools.start(tool);
            }
            EventType::ToolProgress | EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                if !self.tools.is_started(&tool) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow tool.started for tool_id {:?}",
                        path.display(),
                        event.event_type.as_str(),
                        tool.tool_id
                    )));
                }
                if matches!(
                    event.event_type,
                    EventType::ToolCompleted | EventType::ToolTimedOut
                ) {
                    self.tools.finish(tool, line_number);
                }
            }
            EventType::ToolFailed => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                if !self.tools.is_started(&tool) && self.active_phases.contains_key(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} tool.failed must follow tool.started after phase.entered for loop_id {loop_id:?}",
                        path.display()
                    )));
                }
                self.tools.finish(tool, line_number);
            }
            EventType::MessageDelta => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.messages.terminal_line(&message) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.1,
                        terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                match self.active_message_roles.get(&message) {
                    Some(active_role) if active_role != &role => {
                        return Err(RuntimeError::Protocol(format!(
                            "{} line {line_number} message.delta role {:?} must match active role {:?} for message_id {:?}",
                            path.display(),
                            role,
                            active_role,
                            message.1
                        )));
                    }
                    Some(_) => {}
                    None => {
                        self.messages.start(message.clone());
                        self.active_message_roles.insert(message, role);
                    }
                }
            }
            EventType::MessageCompleted => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.messages.terminal_line(&message) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.1,
                        terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                let Some(active_role) = self.active_message_roles.get(&message) else {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed must follow message.delta for message_id {:?}",
                        path.display(),
                        message.1
                    )));
                };
                if active_role != &role {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed role {:?} must match active role {:?} for message_id {:?}",
                        path.display(),
                        role,
                        active_role,
                        message.1
                    )));
                }
                self.messages.finish(message, line_number);
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
        Ok(())
    }

    fn validate_terminal_session(
        &self,
        path: &Path,
        last_event: Option<&EventEnvelope>,
    ) -> Result<(), RuntimeError> {
        if !last_event.is_some_and(|event| {
            matches!(
                event.event_type,
                EventType::SessionCompleted | EventType::SessionFailed
            )
        }) {
            return Ok(());
        }
        for loop_id in self.loops.started_keys() {
            if !self.loops.is_terminal(loop_id) {
                return Err(open_lifecycle_error(path, "loop", loop_id));
            }
        }
        for step in self.steps.started_keys() {
            if !self.steps.is_terminal(step) {
                return Err(open_lifecycle_error(path, "step", &step.step_id));
            }
        }
        for tool in self.tools.started_keys() {
            if !self.tools.is_terminal(tool) {
                return Err(open_lifecycle_error(path, "tool", &tool.tool_id));
            }
        }
        for message in self.messages.started_keys() {
            if !self.messages.is_terminal(message) {
                return Err(open_lifecycle_error(path, "message", &message.1));
            }
        }
        Ok(())
    }
}

fn open_lifecycle_error(path: &Path, kind: &str, id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} terminal session has open {kind} {id:?}",
        path.display()
    ))
}

struct LifecycleTracker<K: Ord> {
    started: BTreeSet<K>,
    terminal: BTreeMap<K, usize>,
}

impl<K: Ord> Default for LifecycleTracker<K> {
    fn default() -> Self {
        Self {
            started: BTreeSet::new(),
            terminal: BTreeMap::new(),
        }
    }
}

impl<K: Ord> LifecycleTracker<K> {
    fn start(&mut self, key: K) {
        self.started.insert(key);
    }

    fn finish(&mut self, key: K, line_number: usize) {
        self.terminal.insert(key, line_number);
    }

    fn is_started(&self, key: &K) -> bool {
        self.started.contains(key)
    }

    fn is_terminal(&self, key: &K) -> bool {
        self.terminal.contains_key(key)
    }

    fn terminal_line(&self, key: &K) -> Option<usize> {
        self.terminal.get(key).copied()
    }

    fn started_keys(&self) -> impl Iterator<Item = &K> {
        self.started.iter()
    }
}

fn started_tool_without_progress(events: &[EventEnvelope]) -> Option<String> {
    let mut active_phases = BTreeMap::new();
    let mut active_steps = BTreeMap::new();
    let mut started_without_progress = BTreeMap::new();

    for event in events {
        match event.event_type {
            EventType::PhaseEntered => {
                if let Some(loop_id) = &event.loop_id {
                    active_phases
                        .insert(loop_id.clone(), lifecycle_payload_string(event, "phase_id"));
                    active_steps.remove(loop_id);
                }
            }
            EventType::StepStarted => {
                if let Some(loop_id) = &event.loop_id {
                    active_steps.insert(loop_id.clone(), lifecycle_step_key(event, &active_phases));
                }
            }
            EventType::StepCompleted => {
                if let Some(loop_id) = &event.loop_id {
                    active_steps.remove(loop_id);
                }
            }
            EventType::ToolStarted => {
                let tool = lifecycle_tool_key(event, &active_phases, &active_steps);
                started_without_progress.insert(tool.clone(), tool.tool_id);
            }
            EventType::ToolProgress
            | EventType::ToolCompleted
            | EventType::ToolFailed
            | EventType::ToolTimedOut => {
                let tool = lifecycle_tool_key(event, &active_phases, &active_steps);
                started_without_progress.remove(&tool);
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::LoopStarted
            | EventType::LoopCompleted
            | EventType::LoopFailed
            | EventType::MessageDelta
            | EventType::MessageCompleted
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
    }

    started_without_progress.into_values().next()
}

fn terminal_lifecycle_error(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    kind: &str,
    id: &str,
    terminal_line: usize,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} appears after terminal {kind} {id:?} on line {terminal_line}",
        path.display(),
        event.event_type.as_str()
    ))
}

fn require_lifecycle_loop_id(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<String, RuntimeError> {
    event.loop_id.clone().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} must include loop_id",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

/// Ensures parent loop references are already started, still active, and
/// consistent with the parent recorded by loop.started.
fn validate_lifecycle_parent(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    loops: &LifecycleTracker<String>,
    loop_parents: &BTreeMap<String, Option<String>>,
) -> Result<(), RuntimeError> {
    if event.parent_loop_id.is_some() && event.loop_id.is_none() {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} parent_loop_id requires loop_id",
            path.display()
        )));
    }

    let Some(loop_id) = &event.loop_id else {
        return Ok(());
    };

    if let Some(parent_loop_id) = &event.parent_loop_id {
        if parent_loop_id == loop_id {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id must not match loop_id {loop_id:?}",
                path.display()
            )));
        }
        if !loops.is_started(parent_loop_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id {parent_loop_id:?} must reference an already started loop",
                path.display()
            )));
        }
        if let Some(terminal_line) = loops.terminal_line(parent_loop_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id {parent_loop_id:?} references terminal loop on line {terminal_line}",
                path.display()
            )));
        }
    }

    if let Some(expected_parent) = loop_parents.get(loop_id) {
        if expected_parent != &event.parent_loop_id {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id for loop_id {loop_id:?} must match loop.started",
                path.display()
            )));
        }
    }

    Ok(())
}

type MessageLifecycleKey = (String, String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StepLifecycleKey {
    loop_id: Option<String>,
    phase_id: Option<String>,
    step_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ToolLifecycleKey {
    loop_id: Option<String>,
    phase_id: Option<String>,
    step_id: Option<String>,
    tool_id: String,
}

fn require_active_phase(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
) -> Result<String, RuntimeError> {
    let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
    active_phases.get(&loop_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active phase for loop_id {loop_id:?}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

fn require_active_step(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    active_steps: &BTreeMap<String, StepLifecycleKey>,
) -> Result<StepLifecycleKey, RuntimeError> {
    let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
    active_steps.get(&loop_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active step for loop_id {loop_id:?}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

fn lifecycle_step_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
) -> StepLifecycleKey {
    let loop_id = event.loop_id.clone();
    let phase_id = event
        .payload
        .get("phase_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            loop_id
                .as_ref()
                .and_then(|loop_id| active_phases.get(loop_id))
                .cloned()
        });
    StepLifecycleKey {
        loop_id,
        phase_id,
        step_id: lifecycle_payload_string(event, "step_id"),
    }
}

fn lifecycle_tool_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
    active_steps: &BTreeMap<String, StepLifecycleKey>,
) -> ToolLifecycleKey {
    let loop_id = event.loop_id.clone();
    let active_step = loop_id
        .as_ref()
        .and_then(|loop_id| active_steps.get(loop_id));
    let phase_id = active_step
        .and_then(|step| step.phase_id.clone())
        .or_else(|| {
            loop_id
                .as_ref()
                .and_then(|loop_id| active_phases.get(loop_id))
                .cloned()
        });
    let step_id = active_step.map(|step| step.step_id.clone());
    ToolLifecycleKey {
        loop_id,
        phase_id,
        step_id,
        tool_id: lifecycle_payload_string(event, "tool_id"),
    }
}

fn lifecycle_message_key(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<MessageLifecycleKey, RuntimeError> {
    Ok((
        require_lifecycle_loop_id(path, line_number, event)?,
        lifecycle_payload_string(event, "message_id"),
    ))
}

fn lifecycle_payload_string(event: &EventEnvelope, field: &str) -> String {
    event
        .payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .expect("payload contract validation ensures lifecycle key fields are strings")
        .to_owned()
}

fn stream_is_failed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionFailed)
}

fn stream_is_completed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionCompleted)
}

fn next_event_id(sequence: u64, events: &[EventEnvelope]) -> String {
    let existing = events
        .iter()
        .map(|event| event.event_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut candidate_sequence = sequence;
    loop {
        let candidate = format!("evt-{candidate_sequence:03}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
        candidate_sequence += 1;
    }
}

fn is_rfc3339_utc_timestamp(value: &str) -> bool {
    parse_rfc3339_utc_timestamp(value).is_some()
}

fn parse_rfc3339_utc_timestamp(value: &str) -> Option<i64> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;

    let mut date_parts = date.split('-');
    let year = date_parts.next().and_then(|part| parse_digits(part, 4))?;
    let month = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    let day = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let minute = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let second_part = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }

    let (second, fraction) = second_part
        .split_once('.')
        .map_or((second_part, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let second = parse_digits(second, 2)?;
    if fraction
        .is_some_and(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }

    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    Some(
        days.saturating_mul(86_400)
            .saturating_add(i64::from(hour) * 3_600)
            .saturating_add(i64::from(minute) * 60)
            .saturating_add(i64::from(second)),
    )
}

fn format_unix_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn parse_digits(value: &str, len: usize) -> Option<u16> {
    if value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests;
