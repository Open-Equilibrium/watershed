//! Loop Agent M1 deterministic runtime.

use proto::{EventEnvelope, EventType};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub const LOCAL_SESSION_DIR: &str = ".loop/sessions";
pub const LOCAL_LOG_DIR: &str = ".loop/logs";
pub const MAX_SESSION_LOG_BYTES: u64 = 16 * 1024 * 1024;
const TAIL_TRANSIENT_READ_RETRY_ATTEMPTS: usize = 200;
const TAIL_TRANSIENT_READ_RETRY_MS: u64 = 5;
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
pub enum RuntimeSurface {
    HumanCli,
    JsonlEventStream,
    LocalSessionLog,
    TailReplayResume,
    DesignedRpc,
    FutureEmbeddedCoreApi,
}

pub fn m1_runtime_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::HumanCli,
        RuntimeSurface::JsonlEventStream,
        RuntimeSurface::LocalSessionLog,
        RuntimeSurface::TailReplayResume,
    ]
}

pub fn designed_future_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::DesignedRpc,
        RuntimeSurface::FutureEmbeddedCoreApi,
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitMode {
    Human,
    Jsonl,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutput {
    pub event_count: usize,
    pub failed: bool,
    pub session_id: String,
    pub session_path: PathBuf,
    pub stdout: String,
}

#[derive(Debug)]
pub enum RuntimeError {
    Io { path: PathBuf, source: io::Error },
    Json(serde_json::Error),
    Policy(core_policy::PolicyCompileError),
    Registry(core_script::RegistryError),
    Protocol(String),
    SessionLogExists(String),
    TerminalSession(String),
    Usage(String),
}

impl RuntimeError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Protocol(_) | Self::SessionLogExists(_) | Self::TerminalSession(_) => 65,
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
            Self::Protocol(message) | Self::Usage(message) => f.write_str(message),
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
            Self::Protocol(_)
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

pub fn validate_session_id(session_id: &str) -> bool {
    proto::is_valid_session_id(session_id)
}

pub fn m0_runtime_notice() -> &'static str {
    "M1 runs deterministic in-process Loop Agent execution; OS sandbox enforcement is post-M1"
}

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
    let artifacts =
        core_policy::compile_policy_artifacts(&loop_block.identity.id, &registry, loop_ref)?;
    let policy = runtime_policy_artifact(&artifacts)?;
    preflight_loop_tools(&registry, policy, loop_block)?;
    let base_session_id = session_id_for_loop(&loop_block.identity.id);
    let reservation = reserve_unique_session_log(workspace, &base_session_id)?;
    let expected_session_id = reservation.session_id.clone();
    if let Err(err) = write_initial_session_log(&reservation, &expected_session_id) {
        reservation.rollback();
        return Err(err);
    }
    let runtime = match execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        &expected_session_id,
        ToolSideEffectMode::ApplyAll,
    ) {
        Ok(runtime) => runtime,
        Err(err) => {
            reservation.rollback();
            return Err(err);
        }
    };

    let result = (|| {
        let stream = canonical_event_stream(&runtime.events)?;
        let events =
            validate_session_log_text(Path::new("runtime.jsonl"), &expected_session_id, &stream)?;
        let session_id = events
            .first()
            .expect("validated streams contain at least one event")
            .session_id
            .clone();
        let failed = runtime.failed;
        complete_reserved_session_log(&reservation, &session_id, &stream, events.len())?;

        Ok(RunOutput {
            event_count: events.len(),
            failed,
            session_id,
            session_path: reservation.session_path.clone(),
            stdout: match emit {
                EmitMode::Jsonl => stream,
                EmitMode::Human if failed => format!("loop {} failed\n", loop_block.identity.id),
                EmitMode::Human => format!("loop {} completed\n", loop_block.identity.id),
            },
        })
    })();
    if result.is_err() {
        reservation.rollback();
    }
    result
}

pub fn replay_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    read_existing_session(workspace.as_ref(), session_id, emit)
}

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

pub fn tail_session_to_writer(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
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

    while !stream_is_failed(&events) && !stream_is_completed(&events) {
        thread::sleep(Duration::from_millis(25));
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
        let appended_events =
            validate_appended_session_log_text(&path, session_id, &events, &appended)?;
        let mut current_events = events.clone();
        current_events.extend(appended_events);
        if !write_tail_chunk(writer, emit, session_id, &appended)? {
            return Ok(RunOutput {
                event_count: current_events.len(),
                failed: stream_is_failed(&current_events),
                session_id: session_id.to_owned(),
                session_path: path,
                stdout: String::new(),
            });
        }
        stream.push_str(&appended);
        events = current_events;
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

fn complete_jsonl_prefix(text: &str) -> &str {
    text.rfind('\n')
        .map_or("", |newline_index| &text[..=newline_index])
}

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

pub fn resume_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let path = session_path(workspace, session_id)?;
    ensure_existing_session_log_path(workspace, &path)?;
    let _lock = acquire_session_lock(workspace, session_id)?;
    let before = read_session_log_to_string(&path)?;
    let events = validate_session_log_text(&path, session_id, &before)?;
    if stream_is_failed(&events) || stream_is_completed(&events) {
        return Err(RuntimeError::TerminalSession(session_id.to_owned()));
    }

    let config = load_workspace_config(workspace)?;
    let registry_path = registry_root_path(workspace, &config.registry_root)?;
    let registry = core_script::load_registry_root(registry_path)?;
    let loop_id = resumable_loop_id(&events, &registry, session_id)?;
    let loop_block = registry.loop_block(&loop_id).ok_or_else(|| {
        RuntimeError::Protocol(format!("resolved registry missing loop {loop_id}"))
    })?;
    let artifacts =
        core_policy::compile_policy_artifacts(&loop_block.identity.id, &registry, &loop_id)?;
    let policy = runtime_policy_artifact(&artifacts)?;
    let planned_runtime = execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        session_id,
        ToolSideEffectMode::DryRun,
    )?;
    if events.len() > planned_runtime.events.len() {
        return Err(RuntimeError::Protocol(format!(
            "{} is not a valid prefix of loop {}",
            path.display(),
            loop_block.identity.id
        )));
    }
    let expected_prefix = canonical_event_stream(&planned_runtime.events[..events.len()])?;
    if before != expected_prefix {
        return Err(RuntimeError::Protocol(format!(
            "{} is not a valid prefix of loop {}",
            path.display(),
            loop_block.identity.id
        )));
    }
    if let Some(tool_id) = started_tool_without_progress(&events) {
        return Err(RuntimeError::Protocol(format!(
            "cannot resume session {session_id} with in-flight tool {tool_id:?} before progress or terminal event"
        )));
    }
    prepare_session_log_append(&path)?;

    let resumed_runtime = execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        session_id,
        ToolSideEffectMode::Resume {
            prefix_event_count: events.len() as u64,
        },
    )?;
    if resumed_runtime.events != planned_runtime.events {
        return Err(RuntimeError::Protocol(format!(
            "{} resumed runtime did not match deterministic replay",
            path.display()
        )));
    }

    let sequence = events
        .last()
        .expect("validated streams contain at least one event")
        .sequence
        + 1;
    let resume_event = EventEnvelope::new(
        next_event_id(sequence, &events),
        EventType::SessionResumed,
        session_id.to_owned(),
        sequence,
        resume_timestamp(sequence),
        "loop-agent-cli",
        serde_json::json!({"reason":"resume"}),
    );
    let mut appended_events = vec![resume_event];
    appended_events.extend(
        resumed_runtime.events[events.len()..]
            .iter()
            .cloned()
            .map(shift_resumed_suffix_event),
    );
    let appended_stream = canonical_event_stream(&appended_events)?;
    let combined = format!("{before}{appended_stream}");
    let combined_events = validate_session_log_text(&path, session_id, &combined)?;
    append_session_log_text(&path, &appended_stream)?;

    Ok(RunOutput {
        event_count: combined_events.len(),
        failed: resumed_runtime.failed,
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: match emit {
            EmitMode::Jsonl => appended_stream,
            EmitMode::Human => format!("session {session_id} resumed\n"),
        },
    })
}

fn append_session_log_text(path: &Path, text: &str) -> Result<(), RuntimeError> {
    append_session_log_bytes(path, text.as_bytes())
}

fn prepare_session_log_append(path: &Path) -> Result<(), RuntimeError> {
    append_session_log_bytes(path, b"")
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

fn shift_resumed_suffix_event(mut event: EventEnvelope) -> EventEnvelope {
    event.sequence += 1;
    event.event_id = format!("evt-{:03}", event.sequence);
    event.timestamp = event_timestamp(event.sequence);
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
}

impl SessionReservation {
    fn rollback(&self) {
        let _ = fs::remove_file(&self.session_path);
        let _ = fs::remove_file(&self.log_path);
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

fn reserve_session_log(
    workspace: &Path,
    session_id: &str,
) -> Result<SessionReservation, RuntimeError> {
    let (session_dir, log_dir) = ensure_runtime_dirs(workspace)?;
    let session_path = session_dir.join(format!("{session_id}.jsonl"));
    let log_path = log_dir.join(format!("{session_id}.log"));
    let lock_path = session_lock_path(workspace, session_id)?;
    reserve_session_lock_file(&lock_path, session_id)?;
    if let Err(err) = reserve_session_file(&session_path, session_id) {
        let _ = fs::remove_file(&lock_path);
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
            Err(RuntimeError::SessionLogExists(_)) => {
                read_existing_session(workspace, &candidate, EmitMode::Jsonl)?;
            }
            Err(err) if is_active_session_error(&err, &candidate) => continue,
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
        RuntimeError::Protocol(message)
            if message == &format!("session {session_id} is already active")
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
        Ok(_) => Err(RuntimeError::SessionLogExists(session_id.to_owned())),
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
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Err(RuntimeError::Protocol(
            format!("session {session_id} is already active"),
        )),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
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

#[cfg(test)]
fn write_session_log(
    workspace: &Path,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    let reservation = reserve_session_log(workspace, session_id)?;
    let result = write_reserved_session_log(&reservation, session_id, stream, event_count)
        .and_then(|()| reservation.release_lock());
    if result.is_err() {
        reservation.rollback();
    }
    result
}

#[cfg(test)]
fn write_reserved_session_log(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    write_existing_file(&reservation.session_path, stream.as_bytes())?;
    write_existing_file(
        &reservation.log_path,
        format!("session_id={session_id}\nevents={event_count}\n").as_bytes(),
    )
}

fn write_initial_session_log(
    reservation: &SessionReservation,
    session_id: &str,
) -> Result<(), RuntimeError> {
    let stream = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        session_id.to_owned(),
        1,
        event_timestamp(1),
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .map_err(|err| RuntimeError::Protocol(format!("failed to serialize initial event: {err}")))?;
    write_existing_file(&reservation.session_path, stream.as_bytes())
}

fn complete_reserved_session_log(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    let first_line_end = stream.find('\n').ok_or_else(|| {
        RuntimeError::Protocol("validated runtime stream must contain an initial event".to_owned())
    })?;
    let append_result = append_existing_file(
        &reservation.session_path,
        &stream.as_bytes()[first_line_end + 1..],
    );
    let metadata_result = if append_result.is_ok() {
        write_existing_file(
            &reservation.log_path,
            format!("session_id={session_id}\nevents={event_count}\n").as_bytes(),
        )
    } else {
        Ok(())
    };
    let release_result = reservation.release_lock();
    append_result?;
    metadata_result?;
    release_result
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
    validate_real_directory(path, &metadata)
}

fn ensure_optional_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_real_directory(path, &metadata)?;
            Ok(true)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn ensure_created_real_directory(path: &Path) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_real_directory(path, &metadata),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?;
            let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?;
            validate_real_directory(path, &metadata)
        }
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn validate_real_directory(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || has_windows_reparse_point(metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            path.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a directory",
            path.display()
        )));
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

fn write_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_real_file(path)?;
    if !hard_link_count_is_verifiable() {
        return replace_existing_file_without_link_count(path, contents);
    }
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

fn append_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_real_file(path)?;
    if !hard_link_count_is_verifiable() {
        return append_existing_file_without_link_count(path, contents);
    }
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

fn append_existing_file_without_link_count(
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    let mut appended = read_to_bytes(path)?;
    appended.extend_from_slice(contents);
    replace_existing_file_without_link_count(path, &appended)
}

fn replace_existing_file_without_link_count(
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    ensure_parent_real_directory(path)?;
    ensure_real_file(path)?;
    let (temp_path, mut temp_file) = create_replacement_temp(path)?;
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
    if let Err(source) = fs::remove_file(path) {
        let _ = fs::remove_file(&temp_path);
        return Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        });
    }
    ensure_parent_real_directory(path)?;
    if let Err(source) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
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

#[cfg(test)]
fn append_session_log_line(path: &Path, line: &str) -> Result<(), RuntimeError> {
    append_session_log_bytes(path, line.as_bytes())
}

fn ensure_real_file(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
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

fn ensure_parent_real_directory(path: &Path) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(|| {
        RuntimeError::Protocol(format!("{} must have a parent directory", path.display()))
    })?;
    ensure_existing_real_directory(parent)
}

struct RuntimeExecution {
    events: Vec<EventEnvelope>,
    failed: bool,
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
}

#[derive(Clone, Copy)]
struct RuntimeToolPolicy<'a> {
    command: &'a core_policy::CommandPolicy,
    protected_path_match_mode: ProtectedPathMatchMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProtectedPathMatchMode {
    CaseSensitive,
    CaseInsensitive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolSideEffectMode {
    ApplyAll,
    DryRun,
    Resume { prefix_event_count: u64 },
}

impl ToolSideEffectMode {
    fn should_execute_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::ApplyAll => true,
            Self::DryRun => false,
            Self::Resume { prefix_event_count } => completed_sequence > prefix_event_count,
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

fn protected_path_match_mode_for_policy_target(
    target: &core_policy::PolicyTarget,
) -> ProtectedPathMatchMode {
    match target {
        core_policy::PolicyTarget::LinuxLandlockSeccomp => ProtectedPathMatchMode::CaseSensitive,
        core_policy::PolicyTarget::MacosSeatbelt => ProtectedPathMatchMode::CaseInsensitive,
    }
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
    events: Vec<EventEnvelope>,
    loop_counter: u64,
    message_counter: u64,
    sequence: u64,
    session_id: String,
}

impl RuntimeEventBuilder {
    fn new(session_id: String) -> Self {
        Self {
            events: Vec::new(),
            loop_counter: 0,
            message_counter: 0,
            sequence: 0,
            session_id,
        }
    }

    fn next_loop_invocation(&mut self, parent_loop_id: Option<String>) -> LoopInvocation {
        self.loop_counter += 1;
        LoopInvocation {
            loop_id: format!("loop-{:03}", self.loop_counter),
            parent_loop_id,
        }
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
    ) {
        self.sequence += 1;
        let mut event = EventEnvelope::new(
            format!("evt-{:03}", self.sequence),
            event_type,
            self.session_id.clone(),
            self.sequence,
            event_timestamp(self.sequence),
            "loop-agent-cli",
            payload,
        );
        if let Some(invocation) = invocation {
            event.loop_id = Some(invocation.loop_id.clone());
            event.parent_loop_id = invocation.parent_loop_id.clone();
        }
        self.events.push(event);
    }
}

fn execute_loop(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_loop: &core_script::LoopBlock,
    session_id: &str,
    side_effect_mode: ToolSideEffectMode,
) -> Result<RuntimeExecution, RuntimeError> {
    let mut builder = RuntimeEventBuilder::new(session_id.to_owned());
    builder.emit(
        None,
        EventType::SessionStarted,
        serde_json::json!({"reason":"fixture-start"}),
    );

    let failed = emit_loop_block(
        workspace,
        registry,
        policy,
        root_loop,
        None,
        side_effect_mode,
        &mut builder,
    )?;
    if let Some(failure) = failed {
        builder.emit(
            None,
            EventType::SessionFailed,
            serde_json::json!({"reason":failure.reason}),
        );
        Ok(RuntimeExecution {
            events: builder.events,
            failed: true,
        })
    } else {
        builder.emit(None, EventType::SessionCompleted, serde_json::json!({}));
        Ok(RuntimeExecution {
            events: builder.events,
            failed: false,
        })
    }
}

fn preflight_loop_tools(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
) -> Result<(), RuntimeError> {
    preflight_loop_tools_at_depth(registry, policy, loop_block, 1)
}

fn preflight_loop_tools_at_depth(
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

    if sandbox_runtime_failure(registry, policy, loop_block)?.is_some() {
        return Ok(());
    }

    for (index, phase_ref) in loop_block.phase_refs.iter().enumerate() {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        preflight_phase_tools(registry, policy, phase)?;

        if index == 0 {
            for subloop_ref in &loop_block.subloop_refs {
                let subloop = registry.loop_block(subloop_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
                })?;
                preflight_loop_tools_at_depth(registry, policy, subloop, depth + 1)?;
            }
        }
    }

    Ok(())
}

fn preflight_phase_tools(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    phase: &core_script::PhaseBlock,
) -> Result<(), RuntimeError> {
    for tool_ref in &phase.tool_refs {
        let tool = registry.tool_block(tool_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
        })?;
        let command_policy = command_policy_for_phase(policy, &phase.identity.id, tool)?;
        ensure_tool_matches_policy(tool, command_policy)?;
        planned_tool_progress(
            tool,
            runtime_protected_path_match_mode(&policy.target),
            command_policy,
        )?;
    }
    Ok(())
}

fn emit_loop_block(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    parent_loop_id: Option<String>,
    side_effect_mode: ToolSideEffectMode,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let context = LoopEmitContext {
        workspace,
        registry,
        policy,
        side_effect_mode,
    };
    emit_loop_block_at_depth(&context, loop_block, parent_loop_id, builder, 1)
}

struct LoopEmitContext<'a> {
    workspace: &'a Path,
    registry: &'a core_script::ResolvedRegistry,
    policy: &'a core_policy::PolicyArtifact,
    side_effect_mode: ToolSideEffectMode,
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

    let invocation = builder.next_loop_invocation(parent_loop_id);
    builder.emit(
        Some(&invocation),
        EventType::LoopStarted,
        serde_json::json!({
            "loop_definition_id": loop_block.identity.id,
            "loop_name": loop_block.identity.name,
        }),
    );

    if let Some(failure) = sandbox_runtime_failure(context.registry, context.policy, loop_block)? {
        emit_runtime_failure(loop_block, &invocation, &failure, builder);
        return Ok(Some(failure));
    }

    for (index, phase_ref) in loop_block.phase_refs.iter().enumerate() {
        let phase = context.registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        emit_phase(
            context.workspace,
            context.registry,
            context.policy,
            phase,
            &invocation,
            context.side_effect_mode,
            builder,
        )?;

        if index == 0 {
            for subloop_ref in &loop_block.subloop_refs {
                let subloop = context.registry.loop_block(subloop_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
                })?;
                if let Some(failure) = emit_loop_block_at_depth(
                    context,
                    subloop,
                    Some(invocation.loop_id.clone()),
                    builder,
                    depth + 1,
                )? {
                    emit_runtime_failure(loop_block, &invocation, &failure, builder);
                    return Ok(Some(failure));
                }
            }
        }
    }

    builder.emit(
        Some(&invocation),
        EventType::LoopCompleted,
        serde_json::json!({
            "loop_definition_id": loop_block.identity.id,
            "loop_name": loop_block.identity.name,
        }),
    );
    Ok(None)
}

fn emit_phase(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    phase: &core_script::PhaseBlock,
    invocation: &LoopInvocation,
    side_effect_mode: ToolSideEffectMode,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    let instruction_ids = phase
        .instruction_refs
        .iter()
        .map(|instruction_ref| {
            registry
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
            registry
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
    );

    for step in &phase.steps {
        let step_payload = step_payload(registry, phase, step)?;
        builder.emit(
            Some(invocation),
            EventType::StepStarted,
            step_payload.clone(),
        );

        if let Some(content) = stub_message_content(registry, phase)? {
            let message_id = builder.next_message_id();
            builder.emit(
                Some(invocation),
                EventType::MessageDelta,
                serde_json::json!({
                    "content_delta": content,
                    "message_id": message_id,
                    "role": "assistant",
                }),
            );
            builder.emit(
                Some(invocation),
                EventType::MessageCompleted,
                serde_json::json!({
                    "message_id": message_id,
                    "role": "assistant",
                }),
            );
        }

        for tool_ref in &phase.tool_refs {
            let tool = registry.tool_block(tool_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
            })?;
            let command_policy = command_policy_for_phase(policy, &phase.identity.id, tool)?;
            let tool_policy = RuntimeToolPolicy {
                command: command_policy,
                protected_path_match_mode: runtime_protected_path_match_mode(&policy.target),
            };
            emit_tool(
                workspace,
                tool,
                tool_policy,
                invocation,
                side_effect_mode,
                builder,
            )?;
        }

        builder.emit(Some(invocation), EventType::StepCompleted, step_payload);
    }

    Ok(())
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
    if policy.network.default != core_policy::NetworkDefault::Deny
        || !policy.network.allow.is_empty()
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
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    ensure_tool_matches_policy(tool, policy.command)?;
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
    );

    let side_effect_sequence = builder.sequence + 1;
    let completed_sequence = side_effect_sequence + u64::from(planned_progress.is_some());
    let replay_guard_sequence = if planned_progress.is_some() {
        side_effect_sequence
    } else {
        completed_sequence
    };
    let progress = if side_effect_mode.should_execute_tool(replay_guard_sequence) {
        execute_tool(
            workspace,
            tool,
            policy.protected_path_match_mode,
            policy.command,
        )?
    } else {
        planned_progress
    };

    if let Some(message) = progress {
        emit_tool_progress(message, tool, invocation, builder);
    }

    builder.emit(
        Some(invocation),
        EventType::ToolCompleted,
        serde_json::json!({
            "exit_code": 0,
            "tool_id": tool.identity.id,
        }),
    );
    Ok(())
}

fn execute_tool(
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
            execute_own_script(workspace, tool, protected_path_match_mode, policy)?;
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
                write_script_output(workspace, &target, &contents)?;
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
    if value.is_empty() || value.split_whitespace().count() != 1 {
        return Err(RuntimeError::Protocol(
            "own-script redirection target must be one literal path".to_owned(),
        ));
    }
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        Ok(value[1..value.len() - 1].to_owned())
    } else {
        Ok(value.to_owned())
    }
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
        .any(|root| workspace_scope_contains(root, &scoped))
    {
        return Err(RuntimeError::Protocol(format!(
            "tool {} lacks write scope {scoped}",
            policy.tool_id
        )));
    }
    ensure_script_target_not_protected(protected_path_match_mode, policy, &scoped)?;
    Ok(relative)
}

fn write_script_output(
    workspace: &Path,
    target: &str,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    let path = ensure_real_workspace_write_path(workspace, target)?;
    replace_script_output_atomically(workspace, target, &path, contents)
}

fn replace_script_output_atomically(
    workspace: &Path,
    target: &str,
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    ensure_real_workspace_write_path(workspace, target)?;
    let initial_leaf_existed = ensure_writable_regular_leaf(path)?;
    let (temp_path, mut temp_file) = create_replacement_temp(path)?;
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

    ensure_real_workspace_write_path(workspace, target)?;
    if initial_leaf_existed {
        if ensure_writable_regular_leaf(path)? {
            if let Err(source) = fs::remove_file(path) {
                let _ = fs::remove_file(&temp_path);
                return Err(RuntimeError::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        }
    } else {
        ensure_new_leaf_available(path)?;
    }
    ensure_real_workspace_write_path(workspace, target)?;
    if let Err(source) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        });
    }
    Ok(())
}

fn create_replacement_temp(path: &Path) -> Result<(PathBuf, fs::File), RuntimeError> {
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
    Err(RuntimeError::Protocol(format!(
        "could not allocate temporary replacement path for {}",
        path.display()
    )))
}

fn replacement_temp_path(path: &Path, attempt: u32) -> Result<PathBuf, RuntimeError> {
    let mut file_name = path
        .file_name()
        .ok_or_else(|| RuntimeError::Protocol("replacement path must have a file name".to_owned()))?
        .to_os_string();
    file_name.push(format!(".watershed-{}-{attempt}.tmp", std::process::id()));
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

    Err(RuntimeError::Protocol(format!(
        "tool {} cannot write protected path {scoped_target}",
        policy.tool_id
    )))
}

fn ensure_real_workspace_write_path(
    workspace: &Path,
    target: &str,
) -> Result<PathBuf, RuntimeError> {
    let mut parts = target.split('/').peekable();
    let mut path = workspace.to_path_buf();
    while let Some(part) = parts.next() {
        path.push(part);
        if parts.peek().is_some() {
            ensure_created_real_directory(&path)?;
        }
    }
    Ok(path)
}

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
    if hard_link_count(&file_metadata) > 1 {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be hard-linked",
            path.display()
        )));
    }

    Ok(())
}

#[cfg(unix)]
fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file_metadata(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn hard_link_count(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    metadata.nlink()
}

#[cfg(not(unix))]
fn hard_link_count(_metadata: &fs::Metadata) -> u64 {
    1
}

#[cfg(unix)]
fn hard_link_count_is_verifiable() -> bool {
    true
}

#[cfg(not(unix))]
fn hard_link_count_is_verifiable() -> bool {
    false
}

fn normalize_script_write_target(target: &str) -> Result<String, RuntimeError> {
    let target = target.replace('\\', "/");
    if target.is_empty()
        || target.starts_with('/')
        || target.contains(':')
        || target.contains('$')
        || target.contains('*')
        || target.contains('?')
    {
        return Err(RuntimeError::Protocol(format!(
            "own-script write target {target:?} must be a literal workspace-relative path"
        )));
    }
    let mut parts = Vec::new();
    for part in target.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(RuntimeError::Protocol(format!(
                "own-script write target {target:?} must stay inside the workspace"
            )));
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return Err(RuntimeError::Protocol(format!(
                "own-script write target {target:?} must not use a Windows path alias"
            )));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn protected_path_pattern_matches(
    match_mode: ProtectedPathMatchMode,
    pattern: &str,
    path: &str,
) -> bool {
    let pattern = normalize_protected_path_match_input(match_mode, pattern);
    let path = normalize_protected_path_match_input(match_mode, path);
    let pattern_segments = pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let path_segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    protected_segments_match(&pattern_segments, &path_segments)
}

fn normalize_protected_path_match_input(match_mode: ProtectedPathMatchMode, value: &str) -> String {
    let normalized = value.replace('\\', "/");
    match match_mode {
        ProtectedPathMatchMode::CaseSensitive => normalized,
        ProtectedPathMatchMode::CaseInsensitive => normalized.to_ascii_lowercase(),
    }
}

fn protected_segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.split_first(), path.split_first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some((pattern_segment, rest)), _) if *pattern_segment == "**" => {
            protected_segments_match(rest, path)
                || (!path.is_empty() && protected_segments_match(pattern, &path[1..]))
        }
        (Some((pattern_segment, rest_pattern)), Some((path_segment, rest_path))) => {
            protected_segment_match(pattern_segment, path_segment)
                && protected_segments_match(rest_pattern, rest_path)
        }
        (Some(_), None) => false,
    }
}

fn protected_segment_match(pattern: &str, path: &str) -> bool {
    let pattern = pattern.as_bytes();
    let path = path.as_bytes();
    let mut pattern_index = 0;
    let mut path_index = 0;
    let mut star_pattern_index = None;
    let mut star_path_index = 0;

    while path_index < path.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == path[path_index])
        {
            pattern_index += 1;
            path_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_pattern_index = Some(pattern_index);
            pattern_index += 1;
            star_path_index = path_index;
        } else if let Some(star_index) = star_pattern_index {
            pattern_index = star_index + 1;
            star_path_index += 1;
            path_index = star_path_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
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
) {
    builder.emit(
        Some(invocation),
        EventType::ToolProgress,
        serde_json::json!({
            "message": message,
            "tool_id": tool.identity.id,
        }),
    );
}

fn workspace_scope_contains(root: &str, path: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn ensure_writable_regular_leaf(path: &Path) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
            path.display()
        ))),
        Ok(metadata) if metadata.is_file() => {
            if hard_link_count_is_verifiable() && hard_link_count(&metadata) > 1 {
                return Err(RuntimeError::Protocol(format!(
                    "{} must not be hard-linked",
                    path.display()
                )));
            }
            Ok(true)
        }
        Ok(_) => Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn emit_runtime_failure(
    loop_block: &core_script::LoopBlock,
    invocation: &LoopInvocation,
    failure: &RuntimeFailure,
    builder: &mut RuntimeEventBuilder,
) {
    if let Some(tool_id) = &failure.tool_id {
        builder.emit(
            Some(invocation),
            EventType::ToolFailed,
            serde_json::json!({
                "error": failure.reason,
                "tool_id": tool_id,
            }),
        );
    }
    builder.emit(
        Some(invocation),
        EventType::Error,
        serde_json::json!({
            "code": failure.reason,
            "message": failure.message,
        }),
    );
    builder.emit(
        Some(invocation),
        EventType::LoopFailed,
        serde_json::json!({
            "error": failure.reason,
            "loop_definition_id": loop_block.identity.id,
        }),
    );
}

fn sandbox_runtime_failure(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    for phase_ref in &loop_block.phase_refs {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        if let Some(failure) = sandbox_out_of_phase_failure(registry, policy, phase) {
            return Ok(Some(failure));
        }
        for tool_ref in &phase.tool_refs {
            let tool = registry.tool_block(tool_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
            })?;
            let command_policy = command_policy_for_phase(policy, &phase.identity.id, tool)?;
            if let Some(failure) = sandbox_tool_dispatch_failure(tool, command_policy)? {
                return Ok(Some(failure));
            }
        }
    }

    Ok(None)
}

fn sandbox_tool_dispatch_failure(
    tool: &core_script::ToolBlock,
    command_policy: &core_policy::CommandPolicy,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    ensure_tool_matches_policy(tool, command_policy)?;
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
    let has_unavailable_sentinel = registry.tools.values().any(|tool| {
        is_sandbox_negative_sentinel_tool(tool)
            && !policy_phase_contains_tool(policy, &phase.identity.id, &tool.identity.id)
    });
    if !has_unavailable_sentinel {
        return None;
    }
    Some(runtime_failure_for_reason(
        core_policy::DenyReasonCode::ToolOutOfPhase,
        None,
    ))
}

fn is_sandbox_negative_sentinel_tool(tool: &core_script::ToolBlock) -> bool {
    let (
        core_script::ToolKind::PredefinedCommand,
        core_script::ToolCommand::Predefined { command_id, argv },
    ) = (&tool.tool_kind, &tool.command)
    else {
        return false;
    };
    command_id == "agent-negative"
        && matches!(
            argv.as_slice(),
            [operation] if sandbox_negative_reason_for_operation(operation).is_some()
        )
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

fn runtime_failure_for_reason(
    reason_code: core_policy::DenyReasonCode,
    tool_id: Option<String>,
) -> RuntimeFailure {
    RuntimeFailure {
        reason: reason_code.as_str().to_owned(),
        message: denial_message(reason_code),
        tool_id,
    }
}

#[cfg(test)]
fn linux_sandbox_expected_decision(
    fixture_name: &'static str,
) -> Result<core_policy::ExpectedDecision, RuntimeError> {
    let Some(text) = linux_sandbox_expected_decision_text(fixture_name) else {
        return Err(RuntimeError::Protocol(format!(
            "missing linux expected decision for {fixture_name}"
        )));
    };
    let decision: core_policy::ExpectedDecision = serde_json::from_str(text)?;
    decision.validate().map_err(|err| {
        RuntimeError::Protocol(format!("{fixture_name} linux expected decision: {err}"))
    })?;
    if decision.fixture_name != fixture_name {
        return Err(RuntimeError::Protocol(format!(
            "{fixture_name} expected decision fixture_name mismatch"
        )));
    }
    Ok(decision)
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

#[cfg(test)]
fn linux_sandbox_expected_decision_text(loop_id: &str) -> Option<&'static str> {
    sandbox_expected_decision_texts(loop_id)?
        .into_iter()
        .find_map(|(target, text)| {
            (target == core_policy::PolicyTarget::LinuxLandlockSeccomp).then_some(text)
        })
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

fn event_timestamp(sequence: u64) -> String {
    format!("2026-01-01T00:00:{:02}Z", sequence.saturating_sub(1) % 60)
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

#[cfg(test)]
fn validate_failed_sandbox_decisions(
    fixture_name: &str,
    events: &[EventEnvelope],
) -> Result<(), RuntimeError> {
    let Some(decision_texts) = sandbox_expected_decision_texts(fixture_name) else {
        return Ok(());
    };
    let reason = terminal_failure_reason(events).ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "sandbox-negative fixture {fixture_name} must end with session.failed reason"
        ))
    })?;

    for (target, text) in decision_texts {
        let decision: core_policy::ExpectedDecision = serde_json::from_str(text)?;
        decision.validate().map_err(|err| {
            RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision: {err}"
            ))
        })?;
        if decision.fixture_name != fixture_name {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision fixture_name mismatch"
            )));
        }
        if decision.target != target {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision target mismatch"
            )));
        }
        if decision.expected != core_policy::ExpectedDecisionKind::Deny {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision must deny"
            )));
        }
        if decision.side_effects_allowed {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision must disallow side effects"
            )));
        }
        if decision.reason_code.as_str() != reason {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision reason {} does not match stream reason {reason}",
                decision.reason_code.as_str()
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
fn terminal_failure_reason(events: &[EventEnvelope]) -> Option<&str> {
    events
        .iter()
        .rev()
        .find(|event| event.event_type == EventType::SessionFailed)?
        .payload
        .get("reason")?
        .as_str()
}

#[cfg(test)]
fn sandbox_expected_decision_texts(
    loop_id: &str,
) -> Option<[(core_policy::PolicyTarget, &'static str); 2]> {
    let (linux, macos) = match loop_id {
        "sandbox-negative-environment" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-environment/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-environment/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-interpreter" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-interpreter/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-interpreter/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-network" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-network/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-network/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-protected-path" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-protected-path/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-protected-path/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-symlink" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-symlink/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-symlink/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-tool-out-of-phase" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-tool-out-of-phase/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-tool-out-of-phase/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-write" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-write/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-write/macos-seatbelt.expected.json"
            ),
        ),
        _ => return None,
    };

    Some([
        (core_policy::PolicyTarget::LinuxLandlockSeccomp, linux),
        (core_policy::PolicyTarget::MacosSeatbelt, macos),
    ])
}

fn load_workspace_config(workspace: &Path) -> Result<WorkspaceConfig, RuntimeError> {
    let path = workspace.join(".loop/config.yaml");
    let text = read_to_string(&path)?;
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
    Ok(WorkspaceConfig { registry_root })
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
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

struct WorkspaceConfig {
    registry_root: PathBuf,
}

fn read_to_string(path: &Path) -> Result<String, RuntimeError> {
    fs::read_to_string(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_session_log_to_string(path: &Path) -> Result<String, RuntimeError> {
    read_to_string_with_limit(path, MAX_SESSION_LOG_BYTES)
}

fn session_log_len(path: &Path) -> Result<usize, RuntimeError> {
    let metadata = fs::metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
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
    let bytes = read_file_range(
        path,
        u64::try_from(offset).unwrap_or(u64::MAX),
        u64::try_from(suffix_len).unwrap_or(u64::MAX),
    )?;
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
    let metadata = fs::metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
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
    let mut file = fs::File::open(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
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
    for (index, line) in text.split_terminator('\n').enumerate() {
        let line_number = index + 1;
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

fn validate_appended_session_log_text(
    path: &Path,
    expected_session_id: &str,
    prior_events: &[EventEnvelope],
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    if prior_events.is_empty() {
        return validate_session_log_text(path, expected_session_id, text);
    }
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if !text.ends_with('\n') {
        return Err(RuntimeError::Protocol(format!(
            "{} appended suffix must end with LF",
            path.display()
        )));
    }

    let prior_session_id = &prior_events
        .first()
        .expect("prior events are non-empty")
        .session_id;
    if prior_session_id != expected_session_id {
        return Err(RuntimeError::Protocol(format!(
            "{} contains session_id {prior_session_id:?}, expected {expected_session_id:?}",
            path.display()
        )));
    }

    let mut previous_sequence = prior_events
        .last()
        .expect("prior events are non-empty")
        .sequence;
    let mut event_ids = prior_events
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<BTreeSet<_>>();
    let mut loop_started_ids = prior_events
        .iter()
        .filter(|event| event.event_type == EventType::LoopStarted)
        .filter_map(|event| event.loop_id.clone())
        .collect::<BTreeSet<_>>();
    let mut terminal_line = prior_events.iter().position(|event| {
        matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        )
    });
    terminal_line = terminal_line.map(|index| index + 1);

    let mut appended_events = Vec::new();
    for (index, line) in text.split_terminator('\n').enumerate() {
        let line_number = prior_events.len() + index + 1;
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
        if event.session_id != *prior_session_id {
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
        if matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        ) {
            terminal_line = Some(line_number);
        }
        appended_events.push(event);
    }

    let mut events = prior_events.to_vec();
    events.extend(appended_events.iter().cloned());
    validate_session_lifecycle(path, &events)?;
    Ok(appended_events)
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

    let mut started_loops: BTreeSet<String> = BTreeSet::new();
    let mut loop_parents: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut terminal_loops: BTreeMap<String, usize> = BTreeMap::new();
    let mut started_steps: BTreeSet<StepLifecycleKey> = BTreeSet::new();
    let mut terminal_steps: BTreeMap<StepLifecycleKey, usize> = BTreeMap::new();
    let mut started_tools: BTreeSet<ToolLifecycleKey> = BTreeSet::new();
    let mut terminal_tools: BTreeMap<ToolLifecycleKey, usize> = BTreeMap::new();
    let mut active_messages: BTreeMap<MessageLifecycleKey, String> = BTreeMap::new();
    let mut terminal_messages: BTreeMap<MessageLifecycleKey, usize> = BTreeMap::new();
    let mut active_phases: BTreeMap<String, String> = BTreeMap::new();
    let mut active_steps: BTreeMap<String, StepLifecycleKey> = BTreeMap::new();

    for (index, event) in events.iter().enumerate() {
        let line_number = index + 1;
        if line_number > 1 && event.event_type == EventType::SessionStarted {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} session.started is only valid as the first event",
                path.display()
            )));
        }

        if event.event_type != EventType::LoopStarted {
            if let Some(loop_id) = &event.loop_id {
                if !started_loops.contains(loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow loop.started for loop_id {loop_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                if let Some(terminal_line) = terminal_loops.get(loop_id) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "loop",
                        loop_id,
                        *terminal_line,
                    ));
                }
            }
        }
        validate_lifecycle_parent(
            path,
            line_number,
            event,
            &started_loops,
            &terminal_loops,
            &loop_parents,
        )?;

        match event.event_type {
            EventType::LoopStarted => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                loop_parents.insert(loop_id.clone(), event.parent_loop_id.clone());
                started_loops.insert(loop_id);
            }
            EventType::LoopCompleted | EventType::LoopFailed => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if !started_loops.contains(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow loop.started for loop_id {loop_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                terminal_loops.insert(loop_id, line_number);
            }
            EventType::PhaseEntered => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if let Some(active_step) = active_steps.get(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} phase.entered requires no active step for loop_id {:?}; active step_id {:?}",
                        path.display(),
                        loop_id,
                        active_step.step_id
                    )));
                }
                active_phases.insert(loop_id, lifecycle_payload_string(event, "phase_id"));
            }
            EventType::StepStarted => {
                let active_phase = require_active_phase(path, line_number, event, &active_phases)?;
                let step = lifecycle_step_key(event, &active_phases);
                if step.phase_id.as_deref() != Some(active_phase.as_str()) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started phase_id {:?} must match active phase {:?}",
                        path.display(),
                        step.phase_id,
                        active_phase
                    )));
                }
                if let Some(terminal_line) = terminal_steps.get(&step) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        *terminal_line,
                    ));
                }
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if let Some(active_step) = active_steps.get(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started requires no active step for loop_id {:?}; active step_id {:?}",
                        path.display(),
                        loop_id,
                        active_step.step_id
                    )));
                }
                active_steps.insert(loop_id, step.clone());
                started_steps.insert(step);
            }
            EventType::StepCompleted => {
                let step = lifecycle_step_key(event, &active_phases);
                if let Some(terminal_line) = terminal_steps.get(&step) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        *terminal_line,
                    ));
                }
                if !started_steps.contains(&step) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.completed must follow step.started for step_id {:?}",
                        path.display(),
                        step.step_id
                    )));
                }
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                match active_steps.get(&loop_id) {
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
                active_steps.remove(&loop_id);
                terminal_steps.insert(step, line_number);
            }
            EventType::ToolStarted => {
                require_active_step(path, line_number, event, &active_steps)?;
                let tool = lifecycle_tool_key(event, &active_phases, &active_steps);
                if let Some(terminal_line) = terminal_tools.get(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        *terminal_line,
                    ));
                }
                started_tools.insert(tool);
            }
            EventType::ToolProgress | EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = lifecycle_tool_key(event, &active_phases, &active_steps);
                if let Some(terminal_line) = terminal_tools.get(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        *terminal_line,
                    ));
                }
                if !started_tools.contains(&tool) {
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
                    terminal_tools.insert(tool, line_number);
                }
            }
            EventType::ToolFailed => {
                // Pre-dispatch sandbox denials are recorded as tool.failed without tool.started.
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                let tool = lifecycle_tool_key(event, &active_phases, &active_steps);
                if let Some(terminal_line) = terminal_tools.get(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        *terminal_line,
                    ));
                }
                if !started_tools.contains(&tool) && active_phases.contains_key(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} tool.failed must follow tool.started after phase.entered for loop_id {loop_id:?}",
                        path.display()
                    )));
                }
                terminal_tools.insert(tool, line_number);
            }
            EventType::MessageDelta => {
                require_active_step(path, line_number, event, &active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = terminal_messages.get(&message) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.1,
                        *terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                match active_messages.get(&message) {
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
                        active_messages.insert(message, role);
                    }
                }
            }
            EventType::MessageCompleted => {
                require_active_step(path, line_number, event, &active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = terminal_messages.get(&message) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.1,
                        *terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                let Some(active_role) = active_messages.get(&message) else {
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
                terminal_messages.insert(message, line_number);
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
    }

    if events.last().is_some_and(|event| {
        matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        )
    }) {
        for loop_id in &started_loops {
            if !terminal_loops.contains_key(loop_id) {
                return Err(open_lifecycle_error(path, "loop", loop_id));
            }
        }
        for step in &started_steps {
            if !terminal_steps.contains_key(step) {
                return Err(open_lifecycle_error(path, "step", &step.step_id));
            }
        }
        for tool in &started_tools {
            if !terminal_tools.contains_key(tool) {
                return Err(open_lifecycle_error(path, "tool", &tool.tool_id));
            }
        }
        for message in active_messages.keys() {
            if !terminal_messages.contains_key(message) {
                return Err(open_lifecycle_error(path, "message", &message.1));
            }
        }
    }

    Ok(())
}

fn open_lifecycle_error(path: &Path, kind: &str, id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} terminal session has open {kind} {id:?}",
        path.display()
    ))
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

fn validate_lifecycle_parent(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    started_loops: &BTreeSet<String>,
    terminal_loops: &BTreeMap<String, usize>,
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
        if !started_loops.contains(parent_loop_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id {parent_loop_id:?} must reference an already started loop",
                path.display()
            )));
        }
        if let Some(terminal_line) = terminal_loops.get(parent_loop_id) {
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

fn resume_timestamp(sequence: u64) -> String {
    format!("2026-01-01T00:00:{:02}Z", (sequence.saturating_sub(1)) % 60)
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
    let Some(value) = value.strip_suffix('Z') else {
        return false;
    };
    let Some((date, time)) = value.split_once('T') else {
        return false;
    };

    let mut date_parts = date.split('-');
    let Some(year) = date_parts.next().and_then(|part| parse_digits(part, 4)) else {
        return false;
    };
    let Some(month) = date_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    let Some(day) = date_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return false;
    }
    if day == 0 || day > days_in_month(year, month) {
        return false;
    }

    let mut time_parts = time.split(':');
    let Some(hour) = time_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    let Some(minute) = time_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    let Some(second_part) = time_parts.next() else {
        return false;
    };
    if time_parts.next().is_some() {
        return false;
    }

    let (second, fraction) = second_part
        .split_once('.')
        .map_or((second_part, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let Some(second) = parse_digits(second, 2) else {
        return false;
    };
    if fraction
        .is_some_and(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return false;
    }

    hour <= 23 && minute <= 59 && second <= 59
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
