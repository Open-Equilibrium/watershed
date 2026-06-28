//! Loop Agent M1 deterministic runtime.

use proto::{EventEnvelope, EventType};
use std::{
    collections::BTreeSet,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const LOCAL_SESSION_DIR: &str = ".loop/sessions";
pub const LOCAL_LOG_DIR: &str = ".loop/logs";

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
    "M0 defines Loop Agent contracts and fixtures; runtime execution lands in M1"
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
    ) {
        Ok(runtime) => runtime,
        Err(err) => {
            reservation.rollback();
            return Err(err);
        }
    };

    let result = (|| {
        let stream = canonical_event_stream(&runtime.events)?;
        let events = validate_protocol_jsonl_text(Path::new("runtime.jsonl"), &stream)?;
        let session_id = events
            .first()
            .expect("validated streams contain at least one event")
            .session_id
            .clone();
        if session_id != expected_session_id {
            return Err(RuntimeError::Protocol(format!(
                "runtime emitted session_id {session_id:?}, expected {expected_session_id:?}"
            )));
        }
        let failed = runtime.failed;
        if failed {
            let fixture_name = runtime.sandbox_decision_fixture.ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "failed loop {} did not record an expected sandbox decision fixture",
                    loop_block.identity.id
                ))
            })?;
            validate_failed_sandbox_decisions(fixture_name, &events)?;
        }
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
    let mut stream = read_to_string(&path)?;
    let mut events = validate_session_log_text(&path, session_id, &stream)?;
    write_tail_chunk(writer, emit, session_id, &stream)?;

    while !stream_is_failed(&events) && !stream_is_completed(&events) {
        thread::sleep(Duration::from_millis(25));
        let current = read_to_string(&path)?;
        if current.len() < stream.len() || !current.starts_with(&stream) {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside append-only tail semantics",
                path.display()
            )));
        }
        if current.len() == stream.len() {
            continue;
        }
        let appended = &current[stream.len()..];
        let current_events = validate_session_log_text(&path, session_id, &current)?;
        write_tail_chunk(writer, emit, session_id, appended)?;
        stream = current;
        events = current_events;
    }

    if emit == EmitMode::Human {
        write_tail_chunk(writer, emit, session_id, "")?;
    }

    Ok(RunOutput {
        event_count: events.len(),
        failed: stream_is_failed(&events),
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: String::new(),
    })
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
    let before = read_to_string(&path)?;
    let mut events = validate_session_log_text(&path, session_id, &before)?;
    if stream_is_failed(&events) || stream_is_completed(&events) {
        return Err(RuntimeError::TerminalSession(session_id.to_owned()));
    }

    let sequence = events
        .last()
        .expect("validated streams contain at least one event")
        .sequence
        + 1;
    let event = EventEnvelope::new(
        next_event_id(sequence, &events),
        EventType::SessionResumed,
        session_id.to_owned(),
        sequence,
        resume_timestamp(sequence),
        "loop-agent-cli",
        serde_json::json!({"reason":"resume"}),
    );
    let line = event.canonical_jsonl().map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize session.resumed event: {err}"))
    })?;
    let combined = format!("{before}{line}");
    validate_session_log_text(&path, session_id, &combined)?;
    append_session_log_line(&path, &line)?;
    events.push(event);

    Ok(RunOutput {
        event_count: events.len(),
        failed: false,
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: match emit {
            EmitMode::Jsonl => line,
            EmitMode::Human => format!("session {session_id} resumed\n"),
        },
    })
}

fn read_existing_session(
    workspace: &Path,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let path = session_path(workspace, session_id)?;
    ensure_existing_session_log_path(workspace, &path)?;
    let stream = read_to_string(&path)?;
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
) -> Result<(), RuntimeError> {
    match emit {
        EmitMode::Jsonl => writer
            .write_all(jsonl.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<tail>"),
                source,
            }),
        EmitMode::Human => {
            if jsonl.is_empty() {
                writer
                    .write_all(format!("session {session_id} tailed\n").as_bytes())
                    .and_then(|_| writer.flush())
                    .map_err(|source| RuntimeError::Io {
                        path: PathBuf::from("<tail>"),
                        source,
                    })?;
            }
            Ok(())
        }
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
    session_path: PathBuf,
    session_id: String,
}

impl SessionReservation {
    fn rollback(&self) {
        let _ = fs::remove_file(&self.session_path);
        let _ = fs::remove_file(&self.log_path);
    }
}

fn reserve_session_log(
    workspace: &Path,
    session_id: &str,
) -> Result<SessionReservation, RuntimeError> {
    let (session_dir, log_dir) = ensure_runtime_dirs(workspace)?;
    let session_path = session_dir.join(format!("{session_id}.jsonl"));
    let log_path = log_dir.join(format!("{session_id}.log"));
    reserve_session_file(&session_path, session_id)?;
    if let Err(err) = reserve_new_file(&log_path) {
        let _ = fs::remove_file(&session_path);
        return Err(err);
    }
    Ok(SessionReservation {
        log_path,
        session_path,
        session_id: session_id.to_owned(),
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
            Err(err) => return Err(err),
        }
    }

    Err(RuntimeError::Protocol(format!(
        "could not allocate a unique session_id for {base_session_id}"
    )))
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

#[cfg(test)]
fn write_session_log(
    workspace: &Path,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    let reservation = reserve_session_log(workspace, session_id)?;
    let result = write_reserved_session_log(&reservation, session_id, stream, event_count);
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
    append_existing_file(
        &reservation.session_path,
        &stream.as_bytes()[first_line_end + 1..],
    )?;
    write_existing_file(
        &reservation.log_path,
        format!("session_id={session_id}\nevents={event_count}\n").as_bytes(),
    )
}

fn ensure_runtime_dirs(workspace: &Path) -> Result<(PathBuf, PathBuf), RuntimeError> {
    let loop_dir = workspace.join(".loop");
    ensure_real_directory(&loop_dir)?;
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    ensure_real_directory(&session_dir)?;
    let log_dir = workspace.join(LOCAL_LOG_DIR);
    ensure_real_directory(&log_dir)?;
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

fn ensure_real_directory(path: &Path) -> Result<(), RuntimeError> {
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
    if metadata.file_type().is_symlink() {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
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

fn write_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_real_file(path)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
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
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(contents).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
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

fn append_session_log_line(path: &Path, line: &str) -> Result<(), RuntimeError> {
    ensure_real_file(path)?;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(line.as_bytes())
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })
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

struct RuntimeExecution {
    events: Vec<EventEnvelope>,
    failed: bool,
    sandbox_decision_fixture: Option<&'static str>,
}

#[derive(Clone, Debug)]
struct LoopInvocation {
    loop_id: String,
    parent_loop_id: Option<String>,
}

struct RuntimeFailure {
    reason: String,
    message: &'static str,
    sandbox_decision_fixture: &'static str,
    tool_id: Option<String>,
}

fn runtime_policy_artifact(
    artifacts: &[core_policy::PolicyArtifact],
) -> Result<&core_policy::PolicyArtifact, RuntimeError> {
    artifacts
        .iter()
        .find(|artifact| artifact.target == core_policy::PolicyTarget::LinuxLandlockSeccomp)
        .ok_or_else(|| RuntimeError::Protocol("missing linux runtime policy artifact".to_owned()))
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
) -> Result<RuntimeExecution, RuntimeError> {
    let mut builder = RuntimeEventBuilder::new(session_id.to_owned());
    builder.emit(
        None,
        EventType::SessionStarted,
        serde_json::json!({"reason":"fixture-start"}),
    );

    let failed = emit_loop_block(workspace, registry, policy, root_loop, None, &mut builder)?;
    if let Some(failure) = failed {
        let sandbox_decision_fixture = Some(failure.sandbox_decision_fixture);
        builder.emit(
            None,
            EventType::SessionFailed,
            serde_json::json!({"reason":failure.reason}),
        );
        Ok(RuntimeExecution {
            events: builder.events,
            failed: true,
            sandbox_decision_fixture,
        })
    } else {
        builder.emit(None, EventType::SessionCompleted, serde_json::json!({}));
        Ok(RuntimeExecution {
            events: builder.events,
            failed: false,
            sandbox_decision_fixture: None,
        })
    }
}

fn emit_loop_block(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    parent_loop_id: Option<String>,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let invocation = builder.next_loop_invocation(parent_loop_id);
    builder.emit(
        Some(&invocation),
        EventType::LoopStarted,
        serde_json::json!({
            "loop_definition_id": loop_block.identity.id,
            "loop_name": loop_block.identity.name,
        }),
    );

    if let Some(failure) = sandbox_runtime_failure(registry, policy, loop_block)? {
        emit_runtime_failure(loop_block, &invocation, &failure, builder);
        return Ok(Some(failure));
    }

    for (index, phase_ref) in loop_block.phase_refs.iter().enumerate() {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        emit_phase(workspace, registry, policy, phase, &invocation, builder)?;

        if index == 0 {
            for subloop_ref in &loop_block.subloop_refs {
                let subloop = registry.loop_block(subloop_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
                })?;
                if let Some(failure) = emit_loop_block(
                    workspace,
                    registry,
                    policy,
                    subloop,
                    Some(invocation.loop_id.clone()),
                    builder,
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
            emit_tool(
                workspace,
                tool,
                command_policy,
                policy.runtime_limits.timeout_ms,
                invocation,
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
    policy: &core_policy::CommandPolicy,
    timeout_ms: u64,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    ensure_tool_matches_policy(tool, policy)?;
    builder.emit(
        Some(invocation),
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": policy.allowed_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
            "network_access": tool_network_access_name(&tool.network),
            "read_scope": policy.filesystem.read_roots,
            "tool_id": tool.identity.id,
            "tool_kind": policy_tool_kind_name(&policy.tool_kind),
            "tool_name": tool.identity.name,
            "write_scope": policy.filesystem.write_roots,
        }),
    );

    if let Some(message) = execute_tool(workspace, tool, policy, timeout_ms, builder.sequence)? {
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
    policy: &core_policy::CommandPolicy,
    timeout_ms: u64,
    sequence: u64,
) -> Result<Option<&'static str>, RuntimeError> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, .. },
        ) => Ok(predefined_command_progress(command_id)),
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(_)) => {
            execute_own_script(workspace, tool, policy, timeout_ms, sequence)?;
            Ok(Some("stub write completed"))
        }
        _ => Err(RuntimeError::Protocol(format!(
            "tool command shape does not match {}",
            tool.identity.id
        ))),
    }
}

fn predefined_command_progress(command_id: &str) -> Option<&'static str> {
    match command_id {
        "agent-read" => Some("stub read completed"),
        _ => None,
    }
}

fn execute_own_script(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    policy: &core_policy::CommandPolicy,
    timeout_ms: u64,
    sequence: u64,
) -> Result<(), RuntimeError> {
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
    prepare_own_script_write_targets(workspace, policy, script_body)?;

    let script_dir = workspace.join(LOCAL_LOG_DIR).join("scripts");
    ensure_real_directory(&script_dir)?;
    let script_path = script_dir.join(format!("{}-{sequence}.sh", tool.identity.id));
    ensure_writable_regular_leaf(&script_path)?;
    fs::write(&script_path, script_body).map_err(|source| RuntimeError::Io {
        path: script_path.clone(),
        source,
    })?;

    let mut child = Command::new("sh")
        .arg(&script_path)
        .current_dir(workspace)
        .env_clear()
        .env("SUMMARY", "hello")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| RuntimeError::Io {
            path: PathBuf::from("sh"),
            source,
        })?;
    let deadline = Duration::from_millis(timeout_ms.max(1));
    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|source| RuntimeError::Io {
                path: script_path.clone(),
                source,
            })?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .map_err(|source| RuntimeError::Io {
                    path: script_path.clone(),
                    source,
                })?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RuntimeError::Protocol(format!(
                "tool {} exited with status {}: {}",
                tool.identity.id,
                output.status,
                stderr.trim()
            )));
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RuntimeError::Protocol(format!(
                "tool {} timed out after {timeout_ms} ms",
                tool.identity.id
            )));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn prepare_own_script_write_targets(
    workspace: &Path,
    policy: &core_policy::CommandPolicy,
    script_body: &str,
) -> Result<(), RuntimeError> {
    for line in script_body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(target) = script_redirection_target(line)? {
            prepare_script_write_target(workspace, policy, &target)?;
        }
    }
    Ok(())
}

fn script_redirection_target(line: &str) -> Result<Option<String>, RuntimeError> {
    if line.contains(">>") {
        return Err(RuntimeError::Protocol(
            "own-script append redirection is not supported in M1".to_owned(),
        ));
    }
    let Some((_, target)) = line.split_once('>') else {
        return Ok(None);
    };
    if target.contains('>') {
        return Err(RuntimeError::Protocol(
            "own-script multiple redirections are not supported in M1".to_owned(),
        ));
    }
    let target = unquote_script_path(target.trim())?;
    Ok(Some(target))
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

fn prepare_script_write_target(
    workspace: &Path,
    policy: &core_policy::CommandPolicy,
    target: &str,
) -> Result<(), RuntimeError> {
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
    let path = workspace.join(relative);
    if let Some(parent) = path.parent() {
        ensure_real_directory(parent)?;
    }
    ensure_writable_regular_leaf(&path)
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
        parts.push(part);
    }
    Ok(parts.join("/"))
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

fn ensure_writable_regular_leaf(path: &Path) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
            path.display()
        ))),
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
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
    let Some(fixture_name) = sandbox_negative_fixture_for_loop_name(&loop_block.identity.name)
    else {
        return Ok(None);
    };
    let Some(text) = linux_sandbox_expected_decision_text(fixture_name) else {
        return Ok(None);
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
    if !sandbox_loop_matches_decision(registry, policy, loop_block, &decision.attempt)? {
        return Ok(None);
    }
    let reason = decision.reason_code.as_str().to_owned();
    let tool_id = if matches!(
        decision.attempt,
        core_policy::DeniedAttempt::ToolOutOfPhase { .. }
    ) {
        None
    } else {
        Some(denied_attempt_tool_id(&decision.attempt).to_owned())
    };

    Ok(Some(RuntimeFailure {
        reason,
        message: denial_message(decision.reason_code),
        sandbox_decision_fixture: fixture_name,
        tool_id,
    }))
}

fn sandbox_negative_fixture_for_loop_name(loop_name: &str) -> Option<&'static str> {
    match loop_name {
        "SandboxNegativeEnvironment" => Some("sandbox-negative-environment"),
        "SandboxNegativeInterpreter" => Some("sandbox-negative-interpreter"),
        "SandboxNegativeNetwork" => Some("sandbox-negative-network"),
        "SandboxNegativeProtectedPath" => Some("sandbox-negative-protected-path"),
        "SandboxNegativeSymlink" => Some("sandbox-negative-symlink"),
        "SandboxNegativeToolOutOfPhase" => Some("sandbox-negative-tool-out-of-phase"),
        "SandboxNegativeWrite" => Some("sandbox-negative-write"),
        _ => None,
    }
}

fn sandbox_loop_matches_decision(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    attempt: &core_policy::DeniedAttempt,
) -> Result<bool, RuntimeError> {
    match attempt {
        core_policy::DeniedAttempt::Write { tool_id, .. } => sandbox_loop_has_agent_negative_tool(
            registry,
            policy,
            loop_block,
            "negative",
            tool_id,
            |tool| tool.write_scope.is_empty(),
        ),
        core_policy::DeniedAttempt::Network { tool_id, .. }
        | core_policy::DeniedAttempt::Environment { tool_id, .. }
        | core_policy::DeniedAttempt::ProtectedPath { tool_id, .. }
        | core_policy::DeniedAttempt::InterpreterEscape { tool_id, .. } => {
            sandbox_loop_has_agent_negative_tool(
                registry,
                policy,
                loop_block,
                "negative",
                tool_id,
                |tool| {
                    tool.write_scope.is_empty()
                        && matches!(tool.network, core_script::NetworkPolicy::Deny(_))
                },
            )
        }
        core_policy::DeniedAttempt::SymlinkEscape { tool_id, .. } => {
            sandbox_loop_has_agent_negative_tool(
                registry,
                policy,
                loop_block,
                "negative-symlink",
                tool_id,
                |tool| {
                    tool.write_scope
                        .iter()
                        .any(|scope| scope == "workspace/links")
                },
            )
        }
        core_policy::DeniedAttempt::ToolOutOfPhase { phase_id, tool_id } => {
            sandbox_loop_has_out_of_phase_tool(registry, policy, loop_block, phase_id, tool_id)
        }
    }
}

fn sandbox_loop_has_agent_negative_tool<F>(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    phase_id: &str,
    tool_id: &str,
    tool_predicate: F,
) -> Result<bool, RuntimeError>
where
    F: Fn(&core_script::ToolBlock) -> bool,
{
    for phase_ref in &loop_block.phase_refs {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        if phase.identity.id != phase_id || !policy_phase_contains_tool(policy, phase_id, tool_id) {
            continue;
        }
        for tool_ref in &phase.tool_refs {
            let tool = registry.tool_block(tool_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
            })?;
            if tool.identity.id == tool_id
                && tool_predefined_command_id(tool) == Some("agent-negative")
                && policy_command_matches_tool(policy, tool_id, "agent-negative")
                && tool_predicate(tool)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn sandbox_loop_has_out_of_phase_tool(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    loop_block: &core_script::LoopBlock,
    phase_id: &str,
    tool_id: &str,
) -> Result<bool, RuntimeError> {
    for phase_ref in &loop_block.phase_refs {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        if phase.identity.id != phase_id {
            continue;
        }
        let phase_contains_tool = phase
            .tool_refs
            .iter()
            .map(|tool_ref| {
                registry
                    .tool_block(tool_ref)
                    .map(|tool| tool.identity.id == tool_id)
                    .ok_or_else(|| {
                        RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                    })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?
            .into_iter()
            .any(|matches| matches);
        return Ok(!phase_contains_tool && !policy_phase_contains_tool(policy, phase_id, tool_id));
    }
    Ok(false)
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

fn policy_command_matches_tool(
    policy: &core_policy::PolicyArtifact,
    tool_id: &str,
    command_id: &str,
) -> bool {
    policy.commands.iter().any(|command| {
        command.tool_id == tool_id
            && command.command_id == command_id
            && command.tool_kind == core_policy::ToolKind::PredefinedCommand
    })
}

fn tool_predefined_command_id(tool: &core_script::ToolBlock) -> Option<&str> {
    match &tool.command {
        core_script::ToolCommand::Predefined { command_id, .. } => Some(command_id.as_str()),
        core_script::ToolCommand::OwnScript(_) => None,
    }
}

fn linux_sandbox_expected_decision_text(loop_id: &str) -> Option<&'static str> {
    sandbox_expected_decision_texts(loop_id)?
        .into_iter()
        .find_map(|(target, text)| {
            (target == core_policy::PolicyTarget::LinuxLandlockSeccomp).then_some(text)
        })
}

fn denied_attempt_tool_id(attempt: &core_policy::DeniedAttempt) -> &str {
    match attempt {
        core_policy::DeniedAttempt::Write { tool_id, .. }
        | core_policy::DeniedAttempt::Network { tool_id, .. }
        | core_policy::DeniedAttempt::Environment { tool_id, .. }
        | core_policy::DeniedAttempt::ToolOutOfPhase { tool_id, .. }
        | core_policy::DeniedAttempt::ProtectedPath { tool_id, .. }
        | core_policy::DeniedAttempt::SymlinkEscape { tool_id, .. }
        | core_policy::DeniedAttempt::InterpreterEscape { tool_id, .. } => tool_id,
    }
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
    match loop_id {
        "smoke-loop" => "smoke001".to_owned(),
        "hello-loop" => "hello001".to_owned(),
        "sandbox-negative-environment" => "negenv001".to_owned(),
        "sandbox-negative-interpreter" => "neginterp001".to_owned(),
        "sandbox-negative-network" => "negnet001".to_owned(),
        "sandbox-negative-protected-path" => "negpath001".to_owned(),
        "sandbox-negative-symlink" => "negsymlink001".to_owned(),
        "sandbox-negative-tool-out-of-phase" => "negphase001".to_owned(),
        "sandbox-negative-write" => "negwrite001".to_owned(),
        _ => {
            let mut token = loop_id.to_ascii_lowercase();
            token.retain(|ch| {
                ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
            });
            if token.is_empty() {
                token.push_str("session");
            }
            let suffix = if token.len() <= 125 {
                "001".to_owned()
            } else {
                format!("-{:016x}001", stable_hash64(loop_id.as_bytes()))
            };
            token.truncate(128 - suffix.len());
            token.push_str(&suffix);
            token
        }
    }
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

fn terminal_failure_reason(events: &[EventEnvelope]) -> Option<&str> {
    events
        .iter()
        .rev()
        .find(|event| event.event_type == EventType::SessionFailed)?
        .payload
        .get("reason")?
        .as_str()
}

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
                if metadata.file_type().is_symlink() {
                    return Err(RuntimeError::Usage(
                        ".loop/config.yaml registry_root must not contain symlinks".to_owned(),
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
    validate_session_lifecycle(path, &events)?;
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

    let mut started_loops = BTreeSet::new();
    let mut started_steps = BTreeSet::new();
    let mut started_tools = BTreeSet::new();

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
            }
        }

        match event.event_type {
            EventType::LoopStarted => {
                started_loops.insert(require_lifecycle_loop_id(path, line_number, event)?);
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
            }
            EventType::StepStarted => {
                started_steps.insert(lifecycle_payload_key(event, "step_id"));
            }
            EventType::StepCompleted => {
                let step = lifecycle_payload_key(event, "step_id");
                if !started_steps.contains(&step) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.completed must follow step.started for step_id {:?}",
                        path.display(),
                        step.1
                    )));
                }
            }
            EventType::ToolStarted => {
                started_tools.insert(lifecycle_payload_key(event, "tool_id"));
            }
            EventType::ToolProgress | EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = lifecycle_payload_key(event, "tool_id");
                if !started_tools.contains(&tool) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow tool.started for tool_id {:?}",
                        path.display(),
                        event.event_type.as_str(),
                        tool.1
                    )));
                }
            }
            EventType::ToolFailed => {
                // Pre-dispatch sandbox denials are recorded as tool.failed without tool.started.
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::PhaseEntered
            | EventType::MessageDelta
            | EventType::MessageCompleted
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
    }

    Ok(())
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

fn lifecycle_payload_key(event: &EventEnvelope, field: &str) -> (Option<String>, String) {
    let value = event
        .payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .expect("payload contract validation ensures lifecycle key fields are strings")
        .to_owned();
    (event.loop_id.clone(), value)
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
mod tests {
    use super::*;
    use std::{
        io::{self, Write},
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn m1_surfaces_exclude_rpc_and_embedding() {
        let m1 = m1_runtime_surfaces();

        assert!(m1.contains(&RuntimeSurface::HumanCli));
        assert!(m1.contains(&RuntimeSurface::JsonlEventStream));
        assert!(!m1.contains(&RuntimeSurface::DesignedRpc));
        assert!(!m1.contains(&RuntimeSurface::FutureEmbeddedCoreApi));
    }

    #[test]
    fn session_id_validation_uses_protocol_contract() {
        assert!(validate_session_id("hello001"));
        assert!(!validate_session_id("Hello001"));
        assert!(!validate_session_id("../hello001"));
    }

    #[test]
    fn fallback_session_ids_preserve_valid_loop_id_separators() {
        assert_eq!(session_id_for_loop("foo-bar"), "foo-bar001");
        assert_eq!(session_id_for_loop("foo_bar"), "foo_bar001");
        assert_eq!(session_id_for_loop("foobar"), "foobar001");
        assert_ne!(
            session_id_for_loop("foo-bar"),
            session_id_for_loop("foo_bar")
        );

        let long = "a".repeat(128);
        let session_id = session_id_for_loop(&long);
        assert!(validate_session_id(&session_id));
        assert!(session_id.len() <= 128);
        assert_ne!(session_id, session_id_for_loop(&format!("{long}b")));
    }

    #[test]
    fn registry_root_must_stay_inside_workspace() {
        let workspace = workspace_copy("smoke-loop");
        fs::write(
            workspace.join(".loop/config.yaml"),
            "fixture_profile: stub-model\nregistry_root: ../registry\nstub_model: deterministic\n",
        )
        .expect("config rewrite succeeds");

        let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect_err("escaped registry root must fail");

        assert!(matches!(err, RuntimeError::Usage(message) if message.contains("registry_root")));
        assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
    }

    #[cfg(unix)]
    #[test]
    fn registry_root_rejects_symlinked_path_components() {
        use std::os::unix::fs::symlink;

        let workspace = workspace_copy("smoke-loop");
        let outside = empty_workspace("outside-registry-root");
        copy_dir(
            &fixture_dir("smoke-loop").join("registry"),
            &outside.join("registry"),
        );
        symlink(&outside, workspace.join("link")).expect("registry root symlink created");
        fs::write(
            workspace.join(".loop/config.yaml"),
            "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
        )
        .expect("config rewrite succeeds");

        let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect_err("symlinked registry root component must fail");

        assert!(matches!(err, RuntimeError::Usage(message) if message.contains("symlink")));
        assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
    }

    #[test]
    fn run_loop_executes_registry_without_expected_streams() {
        let workspace = workspace_copy("smoke-loop");
        fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");

        let output = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect("loop executes from registry");

        assert!(!output.failed);
        assert_eq!(output.event_count, 11);
        assert_eq!(
            output.stdout,
            expected_stream("smoke-loop", "smoke-loop.jsonl")
        );
    }

    #[test]
    fn run_loop_executes_non_fixture_predefined_registry_command() {
        let workspace = workspace_copy("smoke-loop");
        fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
        let tool_path = workspace.join("registry/tools/echo.yaml");
        let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
        fs::write(
            &tool_path,
            source.replace("command_id: agent-echo", "command_id: agent-custom"),
        )
        .expect("tool fixture rewritten");

        let output = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect("custom predefined registry command executes");

        assert!(!output.failed);
        assert_eq!(output.event_count, 11);
        assert!(output.stdout.contains("\"tool_id\":\"echo\""));
        assert!(!output.stdout.contains("unsupported tool command"));
    }

    #[test]
    fn run_loop_executes_own_script_without_exact_fixture_body() {
        let workspace = workspace_copy("hello-loop");
        fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
        let tool_path = workspace.join("registry/tools/write-summary.yaml");
        let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
        fs::write(
            &tool_path,
            source.replace(
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/custom-summary.txt",
            ),
        )
        .expect("tool fixture rewritten");

        let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect("own-script body executes through M1 runner");

        assert!(!output.failed);
        assert_eq!(
            fs::read_to_string(workspace.join("out/custom-summary.txt"))
                .expect("custom summary is written"),
            "hello\n"
        );
    }

    #[test]
    fn run_loop_allocates_unique_session_id_for_repeated_valid_runs() {
        let workspace = workspace_copy("smoke-loop");

        let first =
            run_loop(&workspace, "smoke-loop", EmitMode::Jsonl).expect("first loop run succeeds");
        let second = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect("second loop run gets a unique session id");

        assert_eq!(first.session_id, "smoke001");
        assert_eq!(
            first.stdout,
            expected_stream("smoke-loop", "smoke-loop.jsonl")
        );
        assert_eq!(second.session_id, "smoke001-2");
        assert!(second.stdout.contains("\"session_id\":\"smoke001-2\""));
        assert_eq!(
            validate_protocol_jsonl_text(Path::new("second-run.jsonl"), &second.stdout)
                .expect("second run stream remains protocol-valid")
                .len(),
            first.event_count
        );
        assert!(workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke001.jsonl")
            .is_file());
        assert!(workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke001-2.jsonl")
            .is_file());
    }

    #[test]
    fn run_loop_emits_resolved_ids_for_name_references() {
        let workspace = workspace_copy("hello-loop");
        let phase_path = workspace.join("registry/phases/inspect.yaml");
        let source = fs::read_to_string(&phase_path).expect("phase fixture readable");
        fs::write(
            &phase_path,
            source
                .replace(
                    "instruction_refs: [inspect-input]",
                    "instruction_refs: [InspectInput]",
                )
                .replace("tool_refs: [read-file]", "tool_refs: [ReadFile]")
                .replace(
                    "connection_refs: [inspect-data]",
                    "connection_refs: [InspectData]",
                ),
        )
        .expect("phase fixture rewritten");

        let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect("loop executes with name refs");

        assert_eq!(
            output.stdout,
            expected_stream("hello-loop", "hello-loop.jsonl")
        );
    }

    #[test]
    fn run_loop_preflights_existing_session_before_tool_side_effects() {
        let workspace = workspace_copy("hello-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        fs::write(session_dir.join("hello001.jsonl"), "reserved\n").expect("session reserved");

        let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect_err("existing session must fail before execution");

        assert!(matches!(
            err,
            RuntimeError::Json(_) | RuntimeError::Protocol(_)
        ));
        assert!(!workspace.join("out/summary.txt").exists());
        assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
    }

    #[test]
    fn run_loop_rejects_write_summary_without_declared_write_scope() {
        let workspace = workspace_copy("hello-loop");
        let tool_path = workspace.join("registry/tools/write-summary.yaml");
        let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
        fs::write(
            &tool_path,
            source.replace(r#"write_scope: ["workspace/out"]"#, "write_scope: []"),
        )
        .expect("tool fixture rewritten");

        let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect_err("undeclared write scope must fail");

        assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("write scope")));
        assert!(!workspace.join("out/summary.txt").exists());
        assert!(!workspace
            .join(LOCAL_SESSION_DIR)
            .join("hello001.jsonl")
            .exists());
        assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
    }

    #[test]
    fn run_loop_allows_summary_write_inside_enclosing_write_scope() {
        let workspace = workspace_copy("hello-loop");
        let tool_path = workspace.join("registry/tools/write-summary.yaml");
        let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
        fs::write(
            &tool_path,
            source.replace(
                r#"write_scope: ["workspace/out"]"#,
                r#"write_scope: ["workspace"]"#,
            ),
        )
        .expect("tool fixture rewritten");

        let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect("enclosing write scope permits summary artifact");

        assert!(!output.failed);
        assert_eq!(
            fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
            "hello\n"
        );
    }

    #[test]
    fn sandbox_denial_follows_resolved_operation_not_loop_id() {
        let workspace = workspace_copy("sandbox-negative");
        let loop_path = workspace.join("registry/loops/sandbox-negative-write.yaml");
        let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
        fs::write(
            &loop_path,
            source.replace("id: sandbox-negative-write", "id: renamed-negative-write"),
        )
        .expect("loop fixture rewritten");

        let output = run_loop(&workspace, "renamed-negative-write", EmitMode::Jsonl)
            .expect("renamed negative operation runs");

        assert!(output.failed);
        assert!(output.stdout.contains("\"reason\":\"write_denied\""));
        assert!(output
            .stdout
            .contains("\"loop_definition_id\":\"renamed-negative-write\""));
    }

    #[test]
    fn sandbox_denial_requires_negative_registry_shape_not_fixture_id() {
        let workspace = workspace_copy("sandbox-negative");
        let loop_path = workspace.join("registry/loops/sandbox-negative-write.yaml");
        let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
        fs::write(
            &loop_path,
            source.replace("phase_refs: [negative]", "phase_refs: [benign]"),
        )
        .expect("loop fixture rewritten");
        fs::write(
            workspace.join("registry/phases/benign.yaml"),
            "phase:\n  id: benign\n  name: Benign\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  steps:\n    - id: attempt\n      name: Attempt\n",
        )
        .expect("benign phase written");

        let output = run_loop(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
            .expect("loop with reused fixture id runs");

        assert!(!output.failed);
        assert!(output
            .stdout
            .contains("\"event_type\":\"session.completed\""));
        assert!(!output.stdout.contains("write_denied"));
    }

    #[test]
    fn corrupted_session_log_is_rejected_without_rewrite() {
        let workspace = workspace_copy("smoke-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let path = session_dir.join("bad001.jsonl");
        fs::write(&path, "{\"not\":\"an event\"}\n").expect("corrupt log written");
        let before = fs::read_to_string(&path).expect("corrupt log readable");

        for action in [
            replay_session(&workspace, "bad001", EmitMode::Jsonl),
            tail_session(&workspace, "bad001", EmitMode::Jsonl),
            resume_session(&workspace, "bad001", EmitMode::Jsonl),
        ] {
            assert!(action.is_err());
            assert_eq!(
                fs::read_to_string(&path).expect("corrupt log remains readable"),
                before
            );
        }
    }

    #[test]
    fn session_log_reservation_is_atomic_for_duplicate_session_ids() {
        let workspace = empty_workspace("reservation");
        let first =
            reserve_session_log(&workspace, "reserve001").expect("first reservation succeeds");

        let err = reserve_session_log(&workspace, "reserve001")
            .expect_err("second reservation must fail atomically");

        assert!(
            matches!(err, RuntimeError::SessionLogExists(session_id) if session_id == "reserve001")
        );
        assert!(first.session_path.exists());
        assert!(first.log_path.exists());
        first.rollback();
    }

    #[test]
    fn completed_session_log_append_keeps_audit_when_log_update_fails() {
        let workspace = empty_workspace("audit-retained");
        let reservation =
            reserve_session_log(&workspace, "audit001").expect("reservation succeeds");
        write_initial_session_log(&reservation, "audit001").expect("initial audit writes");
        let initial =
            fs::read_to_string(&reservation.session_path).expect("initial audit readable");
        let completed = EventEnvelope::new(
            "evt-002",
            EventType::SessionCompleted,
            "audit001",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({}),
        )
        .canonical_jsonl()
        .expect("completed event serializes");
        let stream = format!("{initial}{completed}");
        fs::remove_file(&reservation.log_path).expect("reserved log removed");
        fs::create_dir(&reservation.log_path).expect("log path replaced by directory");

        let err = complete_reserved_session_log(&reservation, "audit001", &stream, 2)
            .expect_err("log metadata update fails");

        assert!(
            matches!(err, RuntimeError::Protocol(message) if message.contains("must be a file"))
        );
        assert_eq!(
            fs::read_to_string(&reservation.session_path).expect("audit stream remains readable"),
            stream
        );
        fs::remove_dir_all(&reservation.log_path).expect("log directory cleanup");
        reservation.rollback();
    }

    #[test]
    fn session_log_filename_must_match_envelope_session_id() {
        let workspace = workspace_copy("smoke-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        fs::write(
            session_dir.join("wrong001.jsonl"),
            first_event_line("smoke-loop", "smoke-loop.jsonl"),
        )
        .expect("mismatched log written");

        let err = replay_session(&workspace, "wrong001", EmitMode::Jsonl)
            .expect_err("session id mismatch must fail");

        assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("expected")));
    }

    #[test]
    fn resume_rejects_session_log_without_started_event() {
        let workspace = workspace_copy("smoke-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let path = session_dir.join("missing-start.jsonl");
        let event = EventEnvelope::new(
            "evt-001",
            EventType::ToolCompleted,
            "missing-start",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({
                "exit_code": 0,
                "tool_id": "read-fixture",
            }),
        )
        .canonical_jsonl()
        .expect("tool event serializes");
        fs::write(&path, &event).expect("malformed lifecycle log written");

        let err = resume_session(&workspace, "missing-start", EmitMode::Jsonl)
            .expect_err("missing-start log must not resume");

        assert!(
            matches!(err, RuntimeError::Protocol(message) if message.contains("must start with session.started"))
        );
        assert_eq!(
            fs::read_to_string(&path).expect("malformed lifecycle log remains readable"),
            event
        );
    }

    #[test]
    fn resume_rejects_tool_completion_without_tool_start() {
        let workspace = workspace_copy("smoke-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let path = session_dir.join("missing-tool-start.jsonl");
        let started = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "missing-tool-start",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        )
        .canonical_jsonl()
        .expect("session event serializes");
        let loop_started = EventEnvelope {
            loop_id: Some("loop-001".to_owned()),
            ..EventEnvelope::new(
                "evt-002",
                EventType::LoopStarted,
                "missing-tool-start",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({
                    "loop_definition_id": "smoke-loop",
                }),
            )
        }
        .canonical_jsonl()
        .expect("loop event serializes");
        let tool_completed = EventEnvelope {
            loop_id: Some("loop-001".to_owned()),
            ..EventEnvelope::new(
                "evt-003",
                EventType::ToolCompleted,
                "missing-tool-start",
                3,
                "2026-01-01T00:00:02Z",
                "loop-agent-cli",
                serde_json::json!({
                    "exit_code": 0,
                    "tool_id": "echo",
                }),
            )
        }
        .canonical_jsonl()
        .expect("tool event serializes");
        let before = format!("{started}{loop_started}{tool_completed}");
        fs::write(&path, &before).expect("malformed tool lifecycle log written");

        let err = resume_session(&workspace, "missing-tool-start", EmitMode::Jsonl)
            .expect_err("missing tool start log must not resume");

        assert!(
            matches!(err, RuntimeError::Protocol(message) if message.contains("tool.completed must follow tool.started"))
        );
        assert_eq!(
            fs::read_to_string(&path).expect("malformed tool lifecycle log remains readable"),
            before
        );
    }

    #[test]
    fn resume_rejects_events_after_terminal_without_rewriting_log() {
        let workspace = workspace_copy("smoke-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let path = session_dir.join("terminal-plus.jsonl");
        let started = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "terminal-plus",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        )
        .canonical_jsonl()
        .expect("started event serializes");
        let completed = EventEnvelope::new(
            "evt-002",
            EventType::SessionCompleted,
            "terminal-plus",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({}),
        )
        .canonical_jsonl()
        .expect("completed event serializes");
        let appended = EventEnvelope::new(
            "evt-003",
            EventType::SessionPaused,
            "terminal-plus",
            3,
            "2026-01-01T00:00:02Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"external-append"}),
        )
        .canonical_jsonl()
        .expect("appended event serializes");
        let before = format!("{started}{completed}{appended}");
        fs::write(&path, &before).expect("malformed terminal log written");

        let err = resume_session(&workspace, "terminal-plus", EmitMode::Jsonl)
            .expect_err("terminal-plus log must not resume");

        assert!(
            matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal"))
        );
        assert_eq!(
            fs::read_to_string(&path).expect("malformed terminal log remains readable"),
            before
        );
    }

    #[test]
    fn resume_appends_to_nonterminal_session_log() {
        let workspace = workspace_copy("smoke-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let event = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "partial001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        )
        .canonical_jsonl()
        .expect("event serializes");
        fs::write(session_dir.join("partial001.jsonl"), event).expect("partial log written");

        let output =
            resume_session(&workspace, "partial001", EmitMode::Jsonl).expect("session resumes");

        assert_eq!(output.event_count, 2);
        assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
        assert_eq!(
            validate_session_log_text(
                &workspace.join(LOCAL_SESSION_DIR).join("partial001.jsonl"),
                "partial001",
                &fs::read_to_string(workspace.join(LOCAL_SESSION_DIR).join("partial001.jsonl"))
                    .expect("resumed log readable"),
            )
            .expect("resumed log remains valid")
            .len(),
            2
        );
    }

    #[test]
    fn resume_generates_unique_event_id_before_append() {
        let workspace = workspace_copy("smoke-loop");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let event = EventEnvelope::new(
            "evt-002",
            EventType::SessionStarted,
            "partial002",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        )
        .canonical_jsonl()
        .expect("event serializes");
        let path = session_dir.join("partial002.jsonl");
        fs::write(&path, event).expect("partial log written");

        let output =
            resume_session(&workspace, "partial002", EmitMode::Jsonl).expect("session resumes");

        assert!(output.stdout.contains("\"event_id\":\"evt-003\""));
        assert_eq!(
            validate_session_log_text(
                &path,
                "partial002",
                &fs::read_to_string(&path).expect("resumed log readable"),
            )
            .expect("resumed log remains valid")
            .len(),
            2
        );
    }

    #[test]
    fn tail_session_streams_current_prefix_then_appended_events() {
        let workspace = empty_workspace("tail-follow");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let path = session_dir.join("tail001.jsonl");
        let started = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "tail001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        )
        .canonical_jsonl()
        .expect("started event serializes");
        let completed = EventEnvelope::new(
            "evt-002",
            EventType::SessionCompleted,
            "tail001",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({}),
        )
        .canonical_jsonl()
        .expect("completed event serializes");
        fs::write(&path, &started).expect("initial session log written");

        let bytes = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let mut writer = NotifyingWriter {
            bytes: Arc::clone(&bytes),
            first_write: Some(tx),
        };
        let tail_workspace = workspace.clone();
        let handle = thread::spawn(move || {
            tail_session_to_writer(&tail_workspace, "tail001", EmitMode::Jsonl, &mut writer)
        });

        rx.recv_timeout(Duration::from_secs(1))
            .expect("tail writes current prefix before append");
        assert_eq!(
            String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
                .expect("tail prefix is utf8"),
            started
        );
        append_session_log_line(&path, &completed).expect("terminal event appended");

        let output = handle
            .join()
            .expect("tail thread joins")
            .expect("tail succeeds");
        assert_eq!(output.event_count, 2);
        assert!(!output.failed);
        assert_eq!(
            String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
                .expect("tail stream is utf8"),
            format!("{started}{completed}")
        );
    }

    #[test]
    fn protocol_validator_rejects_sequence_that_does_not_start_at_one() {
        let event = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "meta001",
            2,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        );

        assert_invalid_event("bad-sequence.jsonl", event, "first sequence");
    }

    #[test]
    fn protocol_validator_rejects_required_envelope_metadata() {
        let mut empty_source = base_event();
        empty_source.source.clear();
        assert_invalid_event("empty-source.jsonl", empty_source, "source");

        let mut invalid_timestamp = base_event();
        invalid_timestamp.timestamp = "not-a-time".to_owned();
        assert_invalid_event("invalid-timestamp.jsonl", invalid_timestamp, "timestamp");

        let mut empty_correlation_id = base_event();
        empty_correlation_id.correlation_id = Some(String::new());
        assert_invalid_event(
            "empty-correlation-id.jsonl",
            empty_correlation_id,
            "correlation_id",
        );

        let mut empty_loop_id = base_event();
        empty_loop_id.loop_id = Some(String::new());
        assert_invalid_event("empty-loop-id.jsonl", empty_loop_id, "loop_id");

        let mut empty_parent_loop_id = base_event();
        empty_parent_loop_id.parent_loop_id = Some(String::new());
        assert_invalid_event(
            "empty-parent-loop-id.jsonl",
            empty_parent_loop_id,
            "parent_loop_id",
        );
    }

    #[test]
    fn protocol_validator_rejects_event_payload_contract_violations() {
        let mut missing_reason = base_event();
        missing_reason.event_type = EventType::SessionFailed;
        missing_reason.payload = serde_json::json!({});
        assert_invalid_event(
            "missing-session-failed-reason.jsonl",
            missing_reason,
            "session.failed payload.reason",
        );

        let mut incomplete_tool = base_event();
        incomplete_tool.event_type = EventType::ToolStarted;
        incomplete_tool.payload = serde_json::json!({
            "allowed_parameters": [],
            "network_access": "deny",
            "tool_id": "read-file",
            "tool_kind": "predefined-command",
            "tool_name": "ReadFile",
        });
        assert_invalid_event(
            "incomplete-tool-started.jsonl",
            incomplete_tool,
            "tool.started payload.read_scope",
        );

        let mut mismatched_connections = base_event();
        mismatched_connections.event_type = EventType::StepStarted;
        mismatched_connections.payload = serde_json::json!({
            "connection_ids": ["inspect-data"],
            "step_id": "inspect",
            "step_name": "Inspect",
        });
        assert_invalid_event(
            "mismatched-step-connections.jsonl",
            mismatched_connections,
            "connection arrays",
        );

        let mut non_numeric_metric = base_event();
        non_numeric_metric.event_type = EventType::MetricSample;
        non_numeric_metric.payload = serde_json::json!({
            "metric_name": "fsm.p95",
            "value": "1",
        });
        assert_invalid_event(
            "non-numeric-metric.jsonl",
            non_numeric_metric,
            "metric.sample payload.value",
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_rejects_symlinked_log_dir_without_side_effects() {
        use std::os::unix::fs::symlink;

        let workspace = workspace_copy("smoke-loop");
        let outside = empty_workspace("outside-log");
        fs::create_dir_all(workspace.join(".loop")).expect("loop dir");
        symlink(&outside, workspace.join(LOCAL_LOG_DIR)).expect("log dir symlink");

        let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect_err("symlinked log dir must fail");

        assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
        assert!(!outside.join("smoke001.log").exists());
        assert!(!workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke001.jsonl")
            .exists());
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_rejects_symlinked_session_leaf_without_side_effects() {
        use std::os::unix::fs::symlink;

        let workspace = workspace_copy("smoke-loop");
        let outside = empty_workspace("outside-session");
        let session_dir = workspace.join(LOCAL_SESSION_DIR);
        fs::create_dir_all(&session_dir).expect("session dir");
        let outside_target = outside.join("victim.jsonl");
        symlink(&outside_target, session_dir.join("smoke001.jsonl")).expect("session leaf symlink");

        let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect_err("symlinked session leaf must fail");

        assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
        assert!(!outside_target.exists());
        assert!(!workspace.join(LOCAL_LOG_DIR).join("smoke001.log").exists());
    }

    #[cfg(unix)]
    #[test]
    fn run_loop_rejects_symlinked_summary_leaf_without_side_effects() {
        use std::os::unix::fs::symlink;

        let workspace = workspace_copy("hello-loop");
        let outside = empty_workspace("outside-summary");
        let outside_target = outside.join("summary.txt");
        fs::write(&outside_target, "outside\n").expect("outside target written");
        fs::create_dir_all(workspace.join("out")).expect("out dir");
        symlink(&outside_target, workspace.join("out/summary.txt")).expect("summary leaf symlink");

        let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect_err("symlinked summary leaf must fail");

        assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
        assert_eq!(
            fs::read_to_string(&outside_target).expect("outside target readable"),
            "outside\n"
        );
        assert!(!workspace
            .join(LOCAL_SESSION_DIR)
            .join("hello001.jsonl")
            .exists());
        assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
    }

    #[test]
    fn m1_performance_budgets_hold_for_fixture_runtime() {
        let hello = expected_stream("hello-loop", "hello-loop.jsonl");
        let hello_events =
            validate_protocol_jsonl_text(Path::new("hello-loop.jsonl"), &hello).expect("valid");
        let event_count = hello_events.len() as u128;

        let fsm_p95 = p95_duration((0..100).map(|_| {
            let started = Instant::now();
            let events =
                validate_protocol_jsonl_text(Path::new("hello-loop.jsonl"), &hello).expect("valid");
            assert_eq!(events.len(), hello_events.len());
            started.elapsed()
        }));
        assert!(
            fsm_p95.as_nanos() / event_count <= 1_000_000,
            "FSM/event p95 {:?} for {event_count} events",
            fsm_p95
        );

        let log_workspace = empty_workspace("log-budget");
        let log_p95 = p95_duration((0..50).map(|index| {
            let started = Instant::now();
            write_session_log(
                &log_workspace,
                &format!("log{index:03}"),
                &hello,
                hello_events.len(),
            )
            .expect("session log writes");
            started.elapsed()
        }));
        assert!(
            log_p95.as_nanos() / event_count <= 5_000_000,
            "log append/event p95 {:?} for {event_count} events",
            log_p95
        );

        let smoke_workspace = workspace_copy("smoke-loop");
        let dispatch_p95 = p95_duration((0..25).map(|_| {
            clear_runtime_state(&smoke_workspace);
            let started = Instant::now();
            let output =
                run_loop(&smoke_workspace, "smoke-loop", EmitMode::Jsonl).expect("loop runs");
            assert!(!output.failed);
            started.elapsed()
        }));
        assert!(
            dispatch_p95 <= Duration::from_millis(50),
            "no-op dispatch p95 {dispatch_p95:?}"
        );

        let fixture_bytes = fixture_size("hello-loop") + fixture_size("smoke-loop");
        assert!(
            fixture_bytes < 10 * 1024 * 1024,
            "fixture runtime state budget is {fixture_bytes} bytes"
        );
    }

    #[test]
    fn ten_fixture_loops_complete_concurrently() {
        let handles = (0..10)
            .map(|_| {
                thread::spawn(|| {
                    let workspace = workspace_copy("smoke-loop");
                    run_loop(workspace, "smoke-loop", EmitMode::Jsonl).expect("loop runs")
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            let output = handle.join().expect("thread joins");
            assert!(!output.failed);
            assert_eq!(output.event_count, 11);
        }
    }

    fn fixture_dir(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join(name)
    }

    fn workspace_copy(fixture: &str) -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let target = std::env::temp_dir().join(format!(
            "watershed-loop-agent-core-{}-{id}",
            std::process::id()
        ));
        if target.exists() {
            fs::remove_dir_all(&target).expect("stale temp workspace removed");
        }
        copy_dir(&fixture_dir(fixture), &target);
        target
    }

    fn empty_workspace(label: &str) -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let target = std::env::temp_dir().join(format!(
            "watershed-loop-agent-core-{label}-{}-{id}",
            std::process::id()
        ));
        if target.exists() {
            fs::remove_dir_all(&target).expect("stale temp workspace removed");
        }
        fs::create_dir_all(&target).expect("temp workspace created");
        target
    }

    fn copy_dir(source: &Path, target: &Path) {
        fs::create_dir_all(target).expect("target directory created");
        for entry in fs::read_dir(source).expect("source directory readable") {
            let entry = entry.expect("source entry readable");
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            if source_path.is_dir() {
                copy_dir(&source_path, &target_path);
            } else {
                fs::copy(&source_path, &target_path).expect("fixture file copied");
            }
        }
    }

    fn expected_stream(fixture: &str, stream: &str) -> String {
        fs::read_to_string(fixture_dir(fixture).join("expected").join(stream))
            .expect("expected stream is readable")
    }

    fn first_event_line(fixture: &str, stream: &str) -> String {
        expected_stream(fixture, stream)
            .lines()
            .next()
            .expect("stream has first event")
            .to_owned()
            + "\n"
    }

    fn base_event() -> EventEnvelope {
        EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "meta001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        )
    }

    fn assert_invalid_event(name: &str, event: EventEnvelope, expected: &str) {
        let text = event.canonical_jsonl().expect("event serializes");
        let err = validate_protocol_jsonl_text(Path::new(name), &text)
            .expect_err("invalid event must fail");

        assert!(err.to_string().contains(expected), "{err}");
    }

    fn clear_runtime_state(workspace: &Path) {
        let _ = fs::remove_dir_all(workspace.join(LOCAL_SESSION_DIR));
        let _ = fs::remove_dir_all(workspace.join(LOCAL_LOG_DIR));
        let _ = fs::remove_file(workspace.join("out/summary.txt"));
    }

    fn fixture_size(fixture: &str) -> u64 {
        dir_size(&fixture_dir(fixture))
    }

    struct NotifyingWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
        first_write: Option<mpsc::Sender<()>>,
    }

    impl Write for NotifyingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .expect("tail bytes lock")
                .extend_from_slice(buf);
            if let Some(sender) = self.first_write.take() {
                let _ = sender.send(());
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn dir_size(path: &Path) -> u64 {
        fs::read_dir(path)
            .expect("fixture dir readable")
            .map(|entry| {
                let path = entry.expect("fixture entry readable").path();
                if path.is_dir() {
                    dir_size(&path)
                } else {
                    fs::metadata(&path).expect("fixture metadata").len()
                }
            })
            .sum()
    }

    fn p95_duration(samples: impl IntoIterator<Item = Duration>) -> Duration {
        let mut samples = samples.into_iter().collect::<Vec<_>>();
        samples.sort();
        let index = ((samples.len() * 95).div_ceil(100)).saturating_sub(1);
        samples[index]
    }
}
