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
        if proto::is_valid_session_id(stem) {
            let metadata = fs::symlink_metadata(&path).map_err(|source| RuntimeError::Io {
                path: path.clone(),
                source,
            })?;
            if has_windows_reparse_point(&metadata) || !metadata.is_file() {
                continue;
            }
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
    let (mut output, prior_event_count) =
        resume_session_internal(workspace, session_id, None, None)?;
    if emit == EmitMode::Jsonl {
        let stream = read_session_log_to_string(&output.session_path)?;
        let events = validate_session_log_text(&output.session_path, session_id, &stream)?;
        output.stdout = canonical_event_stream(&events[prior_event_count..])?;
    }
    Ok(output)
}

/// Resumes a session with bounded, non-blocking committed-event notifications.
///
/// The caller owns the receiver and any blocking transport. Notifications cover only newly
/// committed events; read their payloads from [`SessionEventReader`] by sequence.
pub fn resume_session_with_live_events(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let (mut output, _) = resume_session_internal(workspace, session_id, Some(notifier), None)?;
    output.stdout.clear();
    Ok(output)
}

fn resume_session_internal(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
) -> Result<(RunOutput, usize), RuntimeError> {
    let workspace = workspace.as_ref();
    let path = session_path(workspace, session_id)?;
    ensure_existing_session_log_path(workspace, &path)?;
    ensure_non_hardlinked_real_file(&path)?;
    let lock = acquire_session_lock(workspace, session_id)?;
    let inspection = inspect_resume_session(&path, session_id)?;
    let prior_event_count = inspection.prior_event_count;
    if matches!(
        inspection.last_event_type,
        EventType::SessionFailed | EventType::SessionCompleted
    ) {
        return Err(RuntimeError::TerminalSession(session_id.to_owned()));
    }
    let metadata = require_session_log_metadata(workspace, session_id)?;
    let loop_id = resumable_loop_id(
        &path,
        session_id,
        inspection.root_loop_definition_id.as_deref(),
        &metadata,
    )?;

    let config = load_workspace_config(workspace)?;
    let registry =
        core_script::load_loop_registry_from_workspace(workspace, &config.registry_root, &loop_id)?;
    let loop_block = registry.loop_block(&loop_id).ok_or_else(|| {
        RuntimeError::Protocol(format!("resolved registry missing loop {loop_id}"))
    })?;
    verify_resume_definition_metadata_values(session_id, &metadata, &registry, loop_block)?;
    let policy = core_policy::compile_policy_artifact(
        &loop_block.identity.id,
        &registry,
        &loop_id,
        runtime_policy_target(),
    )?;
    let clock = resume_event_clock(&config, inspection.clock)?;
    let recorded_context = read_recorded_context_manifest_signature(
        workspace,
        session_id,
        inspection.completed_turns,
    )?;
    let mut prefix_sink =
        RuntimePrefixSink::new(inspection.event_prefix.clone(), recorded_context.clone());
    let planned_runtime = execute_loop_with_sink(
        workspace,
        &registry,
        &policy,
        loop_block,
        session_id,
        LoopExecutionOptions::with_stub_model_fixture_profile(
            clock,
            ToolSideEffectMode::DryRun,
            config.stub_model_fixture_profile,
        ),
        Some(&mut prefix_sink),
    )?;
    if inspection.completed_turns > planned_runtime.context_manifests.record_count
        || recorded_context.record_count > planned_runtime.context_manifests.record_count
        || !prefix_sink.context_prefix_matches()
    {
        let context_path = workspace
            .join(LOCAL_LOG_DIR)
            .join(format!("{session_id}.contexts.jsonl"));
        return Err(RuntimeError::Protocol(format!(
            "{} context manifests do not match deterministic replay",
            context_path.display()
        )));
    }
    let resume_prefix = validate_resume_replay_prefix(
        &path,
        &inspection,
        &prefix_sink,
        &planned_runtime,
        loop_block,
    )?;
    if let Some(tool_id) = inspection.validation.tool_without_progress() {
        return Err(RuntimeError::Protocol(format!(
            "cannot resume session {session_id} with in-flight tool {tool_id:?} before progress or terminal event"
        )));
    }
    let append_plan = resume_append_plan(session_id, &inspection.validation, clock)?;
    let preflight_matches = {
        let mut preflight_sink = ResumePreflightSink {
            appended_bytes: append_plan.marker_stream.len(),
            clock,
            path: &path,
            planned_event_count: resume_prefix.planned_event_count,
            resume_marker_count: resume_prefix.resume_marker_count,
        };
        let runtime = execute_loop_with_sink(
            workspace,
            &registry,
            &policy,
            loop_block,
            session_id,
            LoopExecutionOptions::with_stub_model_fixture_profile(
                clock,
                ToolSideEffectMode::PreflightResume {
                    prefix_event_count: resume_prefix.planned_event_count as u64,
                },
                config.stub_model_fixture_profile,
            ),
            Some(&mut preflight_sink),
        )?;
        let matches = runtime.matches_plan(&planned_runtime);
        preflight_sink.finish()?;
        matches
    };
    if !preflight_matches {
        return Err(RuntimeError::Protocol(format!(
            "{} resume preflight did not match deterministic replay",
            path.display()
        )));
    }
    let context_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{session_id}.contexts.jsonl"));
    let mut serial_writer = SerialSessionWriter::start_prevalidated(SerialWriterStart {
        context_path,
        path: path.clone(),
        session_id: session_id.to_owned(),
        validation: inspection.validation,
        commit_reservation: None,
        notifier,
        timings,
    })?;
    let (runtime_result, replay_matches) = {
        let mut resume_sink = ResumeEventSink {
            clock,
            marker_committed: false,
            marker_event: append_plan.marker_event,
            marker_stream: append_plan.marker_stream,
            planned_event_count: resume_prefix.planned_event_count,
            resume_marker_count: resume_prefix.resume_marker_count,
            writer: &mut serial_writer,
        };
        let result = execute_loop_with_sink(
            workspace,
            &registry,
            &policy,
            loop_block,
            session_id,
            LoopExecutionOptions::with_stub_model_fixture_profile(
                clock,
                ToolSideEffectMode::Resume {
                    prefix_event_count: resume_prefix.planned_event_count as u64,
                },
                config.stub_model_fixture_profile,
            ),
            Some(&mut resume_sink),
        );
        let matches = result
            .as_ref()
            .is_ok_and(|runtime| runtime.matches_plan(&planned_runtime));
        (result, matches)
    };
    let finish_result = serial_writer.finish();
    let resumed_runtime = runtime_result?;
    finish_result?;
    let terminal_error = resumed_runtime.terminal_error;
    let resumed_failed = resumed_runtime.failed;
    let failure_status = resumed_runtime.failure_status;
    if terminal_error.is_none() && !replay_matches {
        return Err(RuntimeError::Protocol(format!(
            "{} resumed runtime did not match deterministic replay",
            path.display()
        )));
    }
    let combined_event_count = planned_runtime
        .events
        .record_count
        .saturating_add(resume_prefix.resume_marker_count)
        .saturating_add(1);
    lock.release()?;
    if let Some(err) = terminal_error {
        return Err(RuntimeError::session_failed(session_id, err));
    }

    Ok((
        RunOutput {
            event_count: combined_event_count,
            failed: resumed_failed,
            session_id: session_id.to_owned(),
            session_path: path,
            stdout: human_session_status_from_failure(
                session_id,
                "resumed",
                failure_status.as_deref(),
            ),
        },
        prior_event_count,
    ))
}

struct ResumeReplayPrefix {
    planned_event_count: usize,
    resume_marker_count: usize,
}

struct ResumeAppendPlan {
    marker_event: EventEnvelope,
    marker_stream: String,
}

struct ResumeSessionInspection {
    clock: EventClock,
    completed_turns: usize,
    event_prefix: RuntimeStreamSignature,
    last_event_type: EventType,
    planned_event_count: usize,
    prefix_metadata_valid: bool,
    prior_event_count: usize,
    resume_marker_count: usize,
    root_loop_definition_id: Option<String>,
    validation: SessionAppendValidationState,
}

struct ResumeInspectionBuilder {
    clock: Option<EventClock>,
    completed_turns: usize,
    event_prefix: RuntimeStreamSignatureBuilder,
    last_event_type: Option<EventType>,
    planned_event_count: usize,
    prefix_metadata_valid: bool,
    prior_event_count: usize,
    resume_marker_count: usize,
    root_loop_definition_id: Option<String>,
}

impl ResumeInspectionBuilder {
    fn new() -> Self {
        Self {
            clock: None,
            completed_turns: 0,
            event_prefix: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            last_event_type: None,
            planned_event_count: 0,
            prefix_metadata_valid: true,
            prior_event_count: 0,
            resume_marker_count: 0,
            root_loop_definition_id: None,
        }
    }

    fn observe(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
        let clock = match self.clock {
            Some(clock) => clock,
            None => {
                let clock = EventClock::from_first_event(event).ok_or_else(|| {
                    RuntimeError::Protocol(
                        "session first event timestamp cannot anchor resume".to_owned(),
                    )
                })?;
                self.clock = Some(clock);
                clock
            }
        };
        self.prior_event_count = self.prior_event_count.saturating_add(1);
        self.last_event_type = Some(event.event_type);
        self.completed_turns += usize::from(event.event_type == EventType::MessageCompleted);
        if self.root_loop_definition_id.is_none()
            && event.event_type == EventType::LoopStarted
            && event.parent_loop_id.is_none()
        {
            self.root_loop_definition_id =
                Some(lifecycle_payload_string(event, "loop_definition_id"));
        }
        if event.event_type == EventType::SessionResumed {
            self.resume_marker_count = self.resume_marker_count.saturating_add(1);
            return Ok(());
        }

        self.prefix_metadata_valid &= event.event_id == format!("evt-{:03}", event.sequence)
            && event.timestamp == clock.timestamp(event.sequence);
        let normalized_sequence = event
            .sequence
            .checked_sub(self.resume_marker_count as u64)
            .filter(|sequence| *sequence > 0)
            .ok_or_else(|| {
                RuntimeError::Protocol("resume marker count exceeds event sequence".to_owned())
            })?;
        let mut normalized = event.clone();
        normalized.sequence = normalized_sequence;
        normalized.event_id = format!("evt-{normalized_sequence:03}");
        normalized.timestamp = clock.timestamp(normalized_sequence);
        let canonical = normalized.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!(
                "failed to serialize normalized resume event: {err}"
            ))
        })?;
        self.event_prefix.push(canonical.as_bytes());
        self.planned_event_count = self.planned_event_count.saturating_add(1);
        Ok(())
    }
}

fn inspect_resume_session(
    path: &Path,
    session_id: &str,
) -> Result<ResumeSessionInspection, RuntimeError> {
    let mut validation = SessionAppendValidationState::empty(session_id);
    let mut inspection = ResumeInspectionBuilder::new();
    for_each_file_line_with_limit(path, MAX_SESSION_LOG_BYTES, |line| {
        validation.validate_appended_with(path, line, |event| inspection.observe(event))
    })?;
    let Some(clock) = inspection.clock else {
        return Err(RuntimeError::Protocol(format!(
            "{} must contain at least one event",
            path.display()
        )));
    };
    let Some(last_event_type) = inspection.last_event_type else {
        unreachable!("a recorded clock requires an event");
    };
    Ok(ResumeSessionInspection {
        clock,
        completed_turns: inspection.completed_turns,
        event_prefix: inspection.event_prefix.signature(),
        last_event_type,
        planned_event_count: inspection.planned_event_count,
        prefix_metadata_valid: inspection.prefix_metadata_valid,
        prior_event_count: inspection.prior_event_count,
        resume_marker_count: inspection.resume_marker_count,
        root_loop_definition_id: inspection.root_loop_definition_id,
        validation,
    })
}

fn validate_resume_replay_prefix(
    path: &Path,
    inspection: &ResumeSessionInspection,
    prefix_sink: &RuntimePrefixSink,
    planned: &RuntimeExecution,
    loop_block: &core_script::LoopBlock,
) -> Result<ResumeReplayPrefix, RuntimeError> {
    if !inspection.prefix_metadata_valid
        || inspection.planned_event_count > planned.events.record_count
        || !prefix_sink.event_prefix_matches()
    {
        return Err(invalid_resume_prefix_error(path, loop_block));
    }

    Ok(ResumeReplayPrefix {
        planned_event_count: inspection.planned_event_count,
        resume_marker_count: inspection.resume_marker_count,
    })
}

fn invalid_resume_prefix_error(path: &Path, loop_block: &core_script::LoopBlock) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} is not a valid prefix of loop {}",
        path.display(),
        loop_block.identity.id
    ))
}

fn resume_append_plan(
    session_id: &str,
    validation: &SessionAppendValidationState,
    clock: EventClock,
) -> Result<ResumeAppendPlan, RuntimeError> {
    let sequence = validation.previous_sequence.saturating_add(1);
    let mut candidate_sequence = sequence;
    let event_id = loop {
        let candidate = format!("evt-{candidate_sequence:03}");
        if !validation.event_ids.contains(&candidate) {
            break candidate;
        }
        candidate_sequence = candidate_sequence.saturating_add(1);
    };
    let resume_event = EventEnvelope::new(
        event_id,
        EventType::SessionResumed,
        session_id.to_owned(),
        sequence,
        clock.timestamp(sequence),
        "loop-agent-cli",
        serde_json::json!({"reason":"resume"}),
    );
    let marker_stream = canonical_event_stream(std::slice::from_ref(&resume_event))?;
    Ok(ResumeAppendPlan {
        marker_event: resume_event,
        marker_stream,
    })
}

fn prepare_session_log_append(path: &Path, appended_bytes: usize) -> Result<(), RuntimeError> {
    ensure_session_log_growth_within_limit(path, appended_bytes)?;
    append_existing_file(path, b"")
}

#[cfg(any(not(any(unix, windows)), test))]
fn append_session_log_bytes(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_session_log_growth_within_limit(path, contents.len())?;
    append_existing_file(path, contents)
}

fn ensure_session_log_growth_within_limit(
    path: &Path,
    appended_bytes: usize,
) -> Result<u64, RuntimeError> {
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
    Ok(existing_bytes)
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
    path: &Path,
    session_id: &str,
    event_loop_id: Option<&str>,
    metadata: &SessionLogMetadata,
) -> Result<String, RuntimeError> {
    let recorded_loop_id = metadata.loop_definition_id.as_deref().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing loop_definition_id metadata"
        ))
    })?;
    if event_loop_id.is_some_and(|event_loop_id| event_loop_id != recorded_loop_id) {
        return Err(RuntimeError::Protocol(format!(
            "{} session {session_id} loop definition metadata does not match durable events",
            path.display()
        )));
    }
    Ok(recorded_loop_id.to_owned())
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
            EmitMode::Human => human_session_status(session_id, "replayed", &events),
        },
    })
}

fn session_path(workspace: &Path, session_id: &str) -> Result<PathBuf, RuntimeError> {
    if !proto::is_valid_session_id(session_id) {
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
    context_path: PathBuf,
    log_path: PathBuf,
    lock_path: PathBuf,
    session_path: PathBuf,
    session_id: String,
    cleanup_on_drop: Cell<bool>,
    committed: Cell<bool>,
}

impl SessionReservation {
    fn rollback(&self) {
        if !self.committed.get() {
            let _ = fs::remove_file(&self.session_path);
            let _ = fs::remove_file(&self.log_path);
            let _ = fs::remove_file(&self.context_path);
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
    cleanup_on_drop: Cell<bool>,
}

impl SessionLockGuard {
    fn release(&self) -> Result<(), RuntimeError> {
        fs::remove_file(&self.path).map_err(|source| RuntimeError::Io {
            path: self.path.clone(),
            source,
        })?;
        self.cleanup_on_drop.set(false);
        Ok(())
    }
}

impl Drop for SessionLockGuard {
    fn drop(&mut self) {
        if self.cleanup_on_drop.get() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionDefinitionMetadata {
    loop_definition_id: String,
    registry_hash: String,
    loop_definition_hash: String,
}

#[derive(Default, Debug, Eq, PartialEq)]
struct SessionLogMetadata {
    loop_definition_id: Option<String>,
    registry_hash: Option<String>,
    loop_definition_hash: Option<String>,
}

fn session_definition_metadata(
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
) -> Result<SessionDefinitionMetadata, RuntimeError> {
    let registry_json = registry.canonical_json()?;
    let loop_json = proto::canonical_json(&serde_json::to_value(loop_block)?).map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize loop definition hash: {err}"))
    })?;
    Ok(SessionDefinitionMetadata {
        loop_definition_id: loop_block.identity.id.clone(),
        registry_hash: sha256_hash_text(registry_json.as_bytes()),
        loop_definition_hash: sha256_hash_text(loop_json.as_bytes()),
    })
}

fn sha256_hash_text(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

#[cfg(test)]
fn verify_resume_definition_metadata(
    workspace: &Path,
    session_id: &str,
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
) -> Result<(), RuntimeError> {
    // WHY: resume hashes bind a partial session to the registry definitions that produced
    // it; incomplete metadata cannot prove the prefix matches the current registry.
    let metadata = require_session_log_metadata(workspace, session_id)?;
    verify_resume_definition_metadata_values(session_id, &metadata, registry, loop_block)
}

fn verify_resume_definition_metadata_values(
    session_id: &str,
    metadata: &SessionLogMetadata,
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
) -> Result<(), RuntimeError> {
    let Some(recorded_registry_hash) = metadata.registry_hash.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing registry_hash metadata"
        )));
    };
    let Some(recorded_loop_definition_hash) = metadata.loop_definition_hash.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing loop_definition_hash metadata"
        )));
    };
    let Some(recorded_loop_definition_id) = metadata.loop_definition_id.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing loop_definition_id metadata"
        )));
    };

    let expected = session_definition_metadata(registry, loop_block)?;
    if recorded_loop_definition_id != expected.loop_definition_id
        || recorded_registry_hash != expected.registry_hash
        || recorded_loop_definition_hash != expected.loop_definition_hash
    {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: recorded definition metadata does not match current registry"
        )));
    }
    Ok(())
}

fn require_session_log_metadata(
    workspace: &Path,
    session_id: &str,
) -> Result<SessionLogMetadata, RuntimeError> {
    read_session_log_metadata(workspace, session_id)?.ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing definition metadata"
        ))
    })
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
            "loop_definition_id" => metadata.loop_definition_id = Some(value.to_owned()),
            "registry_hash" => metadata.registry_hash = Some(value.to_owned()),
            "loop_definition_hash" => metadata.loop_definition_hash = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(metadata)
}

fn session_log_metadata_path(workspace: &Path, session_id: &str) -> Result<PathBuf, RuntimeError> {
    if !proto::is_valid_session_id(session_id) {
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
    reserve_session_log_with_publish_observer(workspace, session_id, || {})
}

fn reserve_session_log_with_publish_observer(
    workspace: &Path,
    session_id: &str,
    after_publish: impl FnOnce(),
) -> Result<SessionReservation, RuntimeError> {
    let (session_dir, log_dir) = ensure_runtime_dirs(workspace)?;
    let session_path = session_dir.join(format!("{session_id}.jsonl"));
    let log_path = log_dir.join(format!("{session_id}.log"));
    let context_path = log_dir.join(format!("{session_id}.contexts.jsonl"));
    let lock_path = session_lock_path(workspace, session_id)?;
    ensure_session_file_available(&session_path, session_id)?;
    reserve_session_lock_file(&lock_path, session_id)?;
    if let Err(err) = reserve_session_file(&session_path, session_id, after_publish) {
        let _ = fs::remove_file(&lock_path);
        return Err(err);
    }
    if let Err(err) = reserve_new_file(&log_path) {
        let _ = fs::remove_file(&session_path);
        let _ = fs::remove_file(&lock_path);
        return Err(err);
    }
    if let Err(err) = reserve_new_file(&context_path) {
        let _ = fs::remove_file(&session_path);
        let _ = fs::remove_file(&log_path);
        let _ = fs::remove_file(&lock_path);
        return Err(err);
    }
    Ok(SessionReservation {
        context_path,
        log_path,
        lock_path,
        session_path,
        session_id: session_id.to_owned(),
        cleanup_on_drop: Cell::new(true),
        committed: Cell::new(false),
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
            Err(RuntimeError::SessionLogExists(_) | RuntimeError::ActiveSession { .. }) => continue,
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
    debug_assert!(proto::is_valid_session_id(&candidate));
    candidate
}

fn reserve_session_file(
    path: &Path,
    session_id: &str,
    after_publish: impl FnOnce(),
) -> Result<(), RuntimeError> {
    ensure_session_file_available(path, session_id)?;
    reserve_new_file(path).map_err(|err| match err {
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::AlreadyExists => {
            RuntimeError::SessionLogExists(session_id.to_owned())
        }
        other => other,
    })?;
    after_publish();
    Ok(())
}

fn ensure_session_file_available(path: &Path, session_id: &str) -> Result<(), RuntimeError> {
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
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
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
    if !proto::is_valid_session_id(session_id) {
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
    Ok(SessionLockGuard {
        path,
        cleanup_on_drop: Cell::new(true),
    })
}

fn write_reserved_session_metadata(
    reservation: &SessionReservation,
    definition_metadata: Option<&SessionDefinitionMetadata>,
) -> Result<(), RuntimeError> {
    replace_existing_file_atomically(
        &reservation.log_path,
        session_log_metadata_text(definition_metadata).as_bytes(),
    )
}

fn session_log_metadata_text(definition_metadata: Option<&SessionDefinitionMetadata>) -> String {
    let mut metadata = String::new();
    if let Some(definition) = definition_metadata {
        metadata.push_str("registry_hash=");
        metadata.push_str(&definition.registry_hash);
        metadata.push('\n');
        metadata.push_str("loop_definition_hash=");
        metadata.push_str(&definition.loop_definition_hash);
        metadata.push('\n');
        metadata.push_str("loop_definition_id=");
        metadata.push_str(&definition.loop_definition_id);
        metadata.push('\n');
    }
    metadata
}
