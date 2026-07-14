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
    let stdout = CapturedOutput::default();
    let captured = stdout.bytes.clone();
    let mut output = resume_session_to_writer(workspace, session_id, emit, stdout)?;
    let stdout = captured
        .lock()
        .map_err(|_| RuntimeError::Protocol("runtime output lock was poisoned".to_owned()))?
        .clone();
    output.stdout = String::from_utf8(stdout).map_err(|source| {
        RuntimeError::Protocol(format!("runtime emitted non-UTF-8 output: {source}"))
    })?;
    Ok(output)
}

/// Resumes a session and publishes newly committed output incrementally to `writer`.
///
/// JSONL events are written only after their identical canonical bytes have been appended to
/// the session log. A failed observer is detached; callers can catch up through replay by
/// `sequence` and `event_id`.
pub fn resume_session_to_writer<W>(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    writer: W,
) -> Result<RunOutput, RuntimeError>
where
    W: Write + Send + 'static,
{
    resume_session_to_writer_internal(workspace, session_id, emit, writer, None)
}

fn resume_session_to_writer_internal<W>(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    writer: W,
    timings: Option<&mut EventWriterTimings>,
) -> Result<RunOutput, RuntimeError>
where
    W: Write + Send + 'static,
{
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
    let loop_id = resumable_loop_id(&path, session_id, &events)?;

    let config = load_workspace_config(workspace)?;
    let registry_path = registry_root_path(workspace, &config.registry_root)?;
    let registry = core_script::load_registry_root(registry_path)?;
    let loop_block = registry.loop_block(&loop_id).ok_or_else(|| {
        RuntimeError::Protocol(format!("resolved registry missing loop {loop_id}"))
    })?;
    verify_resume_definition_metadata(workspace, session_id, &registry, loop_block)?;
    let definition_hashes = session_definition_hashes(&registry, loop_block)?;
    let artifacts =
        core_policy::compile_policy_artifacts(&loop_block.identity.id, &registry, &loop_id)?;
    let policy = runtime_policy_artifact(&artifacts)?;
    let clock = resume_event_clock(&config, &events)?;
    let planned_runtime = execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        session_id,
        LoopExecutionOptions::with_stub_model_fixture_profile(
            clock,
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
            config.stub_model_fixture_profile,
        ),
    )?;
    verify_recorded_context_manifests(
        workspace,
        session_id,
        &events,
        &planned_runtime.context_manifests,
    )?;
    let resume_prefix = validate_resume_replay_prefix(
        &path,
        &events,
        &planned_runtime.events,
        loop_block,
        clock,
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
        LoopExecutionOptions::with_stub_model_fixture_profile(
            clock,
            ToolSideEffectMode::PreflightResume {
                prefix_event_count: resume_prefix.planned_event_count as u64,
            },
            SideEffectRecorder::none(),
            config.stub_model_fixture_profile,
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
        clock,
    )?;

    let context_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{session_id}.contexts.jsonl"));
    let validation = SessionAppendValidationState::from_prior_events(&path, session_id, &events)?;
    let mut serial_writer = SerialSessionWriter::start_prevalidated(
        SerialWriterStart {
            context_path,
            path: path.clone(),
            session_id: session_id.to_owned(),
            validation,
            commit_reservation: None,
            emit,
            timings,
        },
        writer,
    )?;
    let runtime_result = {
        let mut resume_sink = ResumeEventSink {
            clock,
            marker_committed: false,
            marker_event: append_plan.marker_event,
            marker_stream: append_plan.marker_stream,
            planned_event_count: resume_prefix.planned_event_count,
            resume_marker_count: resume_prefix.resume_marker_count,
            writer: &mut serial_writer,
        };
        execute_loop_with_sink(
            workspace,
            &registry,
            policy,
            loop_block,
            session_id,
            LoopExecutionOptions::with_stub_model_fixture_profile(
                clock,
                ToolSideEffectMode::Resume {
                    prefix_event_count: resume_prefix.planned_event_count as u64,
                },
                SideEffectRecorder::none(),
                config.stub_model_fixture_profile,
            ),
            Some(&mut resume_sink),
        )
    };
    let finish_result = serial_writer.finish();
    let resumed_runtime = runtime_result?;
    finish_result?;
    let terminal_error = resumed_runtime.terminal_error;
    let resumed_failed = resumed_runtime.failed;
    if terminal_error.is_none() && resumed_runtime.events != planned_runtime.events {
        return Err(RuntimeError::Protocol(format!(
            "{} resumed runtime did not match deterministic replay",
            path.display()
        )));
    }
    let committed = read_session_log_to_string(&path)?;
    let combined_events = validate_session_log_text(&path, session_id, &committed)?;
    write_existing_session_metadata(
        workspace,
        session_id,
        combined_events.len(),
        &definition_hashes,
    )?;
    if let Some(err) = terminal_error {
        return Err(err);
    }

    serial_writer.publish_human_status(&human_session_status(
        session_id,
        "resumed",
        &combined_events,
    ));

    Ok(RunOutput {
        event_count: combined_events.len(),
        failed: resumed_failed,
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: String::new(),
    })
}

struct ResumeReplayPrefix {
    planned_event_count: usize,
    resume_marker_count: usize,
}

struct ResumeAppendPlan {
    marker_event: EventEnvelope,
    marker_stream: String,
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
    validate_session_log_text(path, session_id, &combined)?;
    prepare_session_log_append(path, &appended_stream)?;
    Ok(ResumeAppendPlan {
        marker_event: resume_event,
        marker_stream,
    })
}

fn prepare_session_log_append(path: &Path, text: &str) -> Result<(), RuntimeError> {
    ensure_session_log_growth_within_limit(path, text.len())?;
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
    path: &Path,
    session_id: &str,
    events: &[EventEnvelope],
) -> Result<String, RuntimeError> {
    let event = events
        .iter()
        .find(|event| event.event_type == EventType::LoopStarted && event.parent_loop_id.is_none())
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} cannot resume session {session_id} before durable loop progress",
                path.display()
            ))
        })?;
    Ok(lifecycle_payload_string(event, "loop_definition_id"))
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
    context_path: PathBuf,
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
    let context_path = log_dir.join(format!("{session_id}.contexts.jsonl"));
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
    if let Err(err) = reserve_new_file(&context_path) {
        let _ = fs::remove_file(&session_path);
        let _ = fs::remove_file(&lock_path);
        let _ = fs::remove_file(&log_path);
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
        side_effects_applied: Cell::new(false),
    })
}

fn persist_context_manifests(
    path: &Path,
    manifests: &[ContextManifest],
) -> Result<(), RuntimeError> {
    let byte_count = manifests
        .iter()
        .map(|manifest| manifest.line.len())
        .sum::<usize>();
    if u64::try_from(byte_count).unwrap_or(u64::MAX) > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest size {byte_count} bytes exceeds max {MAX_SESSION_LOG_BYTES}",
            path.display()
        )));
    }
    let mut stream = String::with_capacity(byte_count);
    for manifest in manifests {
        stream.push_str(&manifest.line);
    }
    replace_existing_file_atomically(path, stream.as_bytes())
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

fn write_existing_session_metadata(
    workspace: &Path,
    session_id: &str,
    event_count: usize,
    definition_hashes: &SessionDefinitionHashes,
) -> Result<(), RuntimeError> {
    let path = session_log_metadata_path(workspace, session_id)?;
    replace_existing_file_atomically(
        &path,
        session_log_metadata_text(session_id, event_count, Some(definition_hashes)).as_bytes(),
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

fn preflight_session_completion_stream(
    reservation: &SessionReservation,
    expected_session_id: &str,
    events: &[EventEnvelope],
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    let stream = canonical_event_stream(events)?;
    let validated_events =
        validate_session_log_text(Path::new("runtime.jsonl"), expected_session_id, &stream)?;
    ensure_session_log_growth_within_limit(&reservation.session_path, stream.len())?;
    Ok(validated_events)
}
