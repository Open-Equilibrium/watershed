//! Loop Agent M1 deterministic runtime.

use proto::{EventEnvelope, EventType};
use std::{
    collections::BTreeSet,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
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
    let registry = core_script::load_registry_root(workspace.join(&config.registry_root))?;
    let loop_block = registry
        .loop_block(loop_ref)
        .ok_or_else(|| RuntimeError::Usage(format!("unknown loop {loop_ref}")))?;
    let _artifacts =
        core_policy::compile_policy_artifacts(&loop_block.identity.id, &registry, loop_ref)?;
    let runtime = execute_loop(workspace, &registry, loop_block)?;
    let stream = canonical_event_stream(&runtime.events)?;
    let events = validate_protocol_jsonl_text(Path::new("runtime.jsonl"), &stream)?;
    let session_id = events
        .first()
        .expect("validated streams contain at least one event")
        .session_id
        .clone();
    let failed = runtime.failed;
    if failed {
        validate_failed_sandbox_decisions(&loop_block.identity.id, &events)?;
    }
    let session_path = session_path(workspace, &session_id)?;
    if session_path.exists() {
        return Err(RuntimeError::SessionLogExists(session_id));
    }
    write_session_log(workspace, &session_id, &stream, events.len())?;

    Ok(RunOutput {
        event_count: events.len(),
        failed,
        session_id,
        session_path,
        stdout: match emit {
            EmitMode::Jsonl => stream,
            EmitMode::Human => format!("loop {} completed\n", loop_block.identity.id),
        },
    })
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
    read_existing_session(workspace.as_ref(), session_id, emit)
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

fn write_session_log(
    workspace: &Path,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    let (session_dir, log_dir) = ensure_runtime_dirs(workspace)?;
    let session_path = session_dir.join(format!("{session_id}.jsonl"));
    let log_path = log_dir.join(format!("{session_id}.log"));
    ensure_new_leaf_available(&session_path)?;
    ensure_new_leaf_available(&log_path)?;
    write_new_file(&session_path, stream.as_bytes())?;
    write_new_file(
        &log_path,
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

fn write_new_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_new_leaf_available(path)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
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
    root_loop: &core_script::LoopBlock,
) -> Result<RuntimeExecution, RuntimeError> {
    let mut builder = RuntimeEventBuilder::new(session_id_for_loop(&root_loop.identity.id));
    builder.emit(
        None,
        EventType::SessionStarted,
        serde_json::json!({"reason":"fixture-start"}),
    );

    let failed = emit_loop_block(workspace, registry, root_loop, None, &mut builder)?;
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

fn emit_loop_block(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
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

    if let Some(failure) = sandbox_runtime_failure(&loop_block.identity.id)? {
        emit_runtime_failure(loop_block, &invocation, &failure, builder);
        return Ok(Some(failure));
    }

    for (index, phase_ref) in loop_block.phase_refs.iter().enumerate() {
        let phase = registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        emit_phase(workspace, registry, phase, &invocation, builder)?;

        if index == 0 {
            for subloop_ref in &loop_block.subloop_refs {
                let subloop = registry.loop_block(subloop_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing loop {subloop_ref}"))
                })?;
                if let Some(failure) = emit_loop_block(
                    workspace,
                    registry,
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
    phase: &core_script::PhaseBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::PhaseEntered,
        serde_json::json!({
            "instruction_ids": phase.instruction_refs,
            "phase_id": phase.identity.id,
            "phase_name": phase.identity.name,
            "tool_ids": phase.tool_refs,
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
            emit_tool(workspace, tool, invocation, builder)?;
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
        let object = payload
            .as_object_mut()
            .expect("step payload is constructed as an object");
        object.insert(
            "connection_ids".to_owned(),
            serde_json::json!(step.connection_refs),
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

fn emit_tool(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    invocation: &LoopInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": tool.allowed_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
            "network_access": network_access_name(&tool.network),
            "read_scope": tool.read_scope,
            "tool_id": tool.identity.id,
            "tool_kind": tool_kind_name(&tool.tool_kind),
            "tool_name": tool.identity.name,
            "write_scope": tool.write_scope,
        }),
    );

    match tool.identity.id.as_str() {
        "read-file" => emit_tool_progress("stub read completed", tool, invocation, builder),
        "write-summary" => {
            write_summary_artifact(workspace)?;
            emit_tool_progress("stub write completed", tool, invocation, builder);
        }
        _ => {}
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

fn write_summary_artifact(workspace: &Path) -> Result<(), RuntimeError> {
    let out_dir = workspace.join("out");
    ensure_real_directory(&out_dir)?;
    let summary = out_dir.join("summary.txt");
    ensure_writable_regular_leaf(&summary)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&summary)
        .map_err(|source| RuntimeError::Io {
            path: summary.clone(),
            source,
        })?;
    file.write_all(b"hello\n")
        .map_err(|source| RuntimeError::Io {
            path: summary,
            source,
        })
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

fn sandbox_runtime_failure(loop_id: &str) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let Some(text) = linux_sandbox_expected_decision_text(loop_id) else {
        return Ok(None);
    };
    let decision: core_policy::ExpectedDecision = serde_json::from_str(text)?;
    decision.validate().map_err(|err| {
        RuntimeError::Protocol(format!("{loop_id} linux expected decision: {err}"))
    })?;
    if decision.fixture_name != loop_id {
        return Err(RuntimeError::Protocol(format!(
            "{loop_id} expected decision fixture_name must match loop id"
        )));
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
        tool_id,
    }))
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
            let mut token = loop_id
                .bytes()
                .filter(|byte| byte.is_ascii_alphanumeric())
                .map(|byte| byte.to_ascii_lowercase() as char)
                .collect::<String>();
            if token.is_empty() {
                token.push_str("session");
            }
            token.truncate(125);
            token.push_str("001");
            token
        }
    }
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

fn network_access_name(policy: &core_script::NetworkPolicy) -> &'static str {
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
    loop_id: &str,
    events: &[EventEnvelope],
) -> Result<(), RuntimeError> {
    let Some(decision_texts) = sandbox_expected_decision_texts(loop_id) else {
        return Ok(());
    };
    let reason = terminal_failure_reason(events).ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "sandbox-negative loop {loop_id} must end with session.failed reason"
        ))
    })?;

    for (target, text) in decision_texts {
        let decision: core_policy::ExpectedDecision = serde_json::from_str(text)?;
        decision.validate().map_err(|err| {
            RuntimeError::Protocol(format!("{loop_id} {target:?} expected decision: {err}"))
        })?;
        if decision.fixture_name != loop_id {
            return Err(RuntimeError::Protocol(format!(
                "{loop_id} {target:?} expected decision fixture_name must match loop id"
            )));
        }
        if decision.target != target {
            return Err(RuntimeError::Protocol(format!(
                "{loop_id} {target:?} expected decision target mismatch"
            )));
        }
        if decision.expected != core_policy::ExpectedDecisionKind::Deny {
            return Err(RuntimeError::Protocol(format!(
                "{loop_id} {target:?} expected decision must deny"
            )));
        }
        if decision.side_effects_allowed {
            return Err(RuntimeError::Protocol(format!(
                "{loop_id} {target:?} expected decision must disallow side effects"
            )));
        }
        if decision.reason_code.as_str() != reason {
            return Err(RuntimeError::Protocol(format!(
                "{loop_id} {target:?} expected decision reason {} does not match stream reason {reason}",
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
        sync::atomic::{AtomicUsize, Ordering},
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
