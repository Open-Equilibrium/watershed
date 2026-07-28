use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_anchored_workspace},
    config_io::{
        load_workspace_config_from, path_io_error, require_fixture_execution_backend,
        resume_event_clock,
    },
    context::{read_anchored_context_manifest_signature, sha256_hex},
    event_construction::{RuntimeStreamSignature, RuntimeStreamSignatureBuilder},
    event_writer::{
        EventWriterTimings, ResumeEventSink, ResumePreflightSink, RuntimeEventSink,
        RuntimePrefixSink, SerialSessionWriter, SerialWriterStart,
    },
    failures::canonical_event_stream,
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredWorkspace, ensure_anchored_non_hardlinked_file,
        ensure_anchored_real_file, for_each_segmented_jsonl_line, open_anchored_runtime_dir,
        read_anchored_to_string_with_limit,
    },
    live_events::LiveEventNotifier,
    planning::{
        EVENT_PLAN_DOMAIN, FlowExecutionAction, FlowExecutionOptions, RuntimeExecution,
        ToolSideEffectMode, plan_flow_with_workspace, runtime_policy_target,
    },
    session::reconcile_controlled_stages,
    session_bundle::{SessionBundleInventory, SessionBundlePaths},
    session_reservation::acquire_anchored_session_lock,
    types::{
        EVENT_STREAM_LIMITS, EmitMode, EventClock, LOCAL_LOG_DIR, LOCAL_SESSION_DIR,
        MAX_FLOW_EVENTS, MAX_SESSION_METADATA_BYTES, RunOutput, RuntimeError,
        human_session_status_from_failure,
    },
    validate::{SessionAppendValidationState, lifecycle_payload_string},
};
#[cfg(test)]
use crate::runtime::{fs_guards::open_runtime_dir, session::post_writer_finish_observer};
use proto::{EventEnvelope, EventType};
use std::{io, path::Path};

/// Lists valid persisted session ids in canonical order.
pub fn resume_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    resume_session_internal(workspace, session_id, None, None, emit == EmitMode::Jsonl)
}

/// Resumes a session with bounded, non-blocking committed-event notifications.
///
/// The caller owns the receiver and any blocking transport. Notifications cover only newly
/// committed events; read their payloads from [`crate::SessionEventReader`] by sequence.
pub fn resume_session_with_live_events(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut output = resume_session_internal(workspace, session_id, Some(notifier), None, false)?;
    output.stdout.clear();
    Ok(output)
}

pub fn resume_session_internal(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    resume_session_internal_with_cleanup_observer_impl(
        workspace,
        session_id,
        notifier,
        timings,
        capture_jsonl,
        |_| {},
    )
}

#[cfg(test)]
pub(crate) fn resume_session_internal_with_cleanup_observer(
    workspace: impl AsRef<Path>,
    session_id: &str,
    capture_jsonl: bool,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    resume_session_internal_with_cleanup_observer_impl(
        workspace,
        session_id,
        None,
        None,
        capture_jsonl,
        before_cleanup,
    )
}

fn resume_session_internal_with_cleanup_observer_impl(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    if !proto::is_valid_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    let execution_workspace = AnchoredWorkspace::open(workspace)?;
    let config = load_workspace_config_from(execution_workspace.root())?;
    require_fixture_execution_backend(&config)?;
    let sessions =
        open_anchored_runtime_dir(&execution_workspace, "sessions")?.ok_or_else(|| {
            RuntimeError::Io {
                path: workspace.join(LOCAL_SESSION_DIR),
                source: io::Error::from(io::ErrorKind::NotFound),
            }
        })?;
    let path = SessionBundlePaths::events_in(&sessions, session_id);
    ensure_anchored_non_hardlinked_file(&path)?;
    let lock = acquire_anchored_session_lock(&sessions, session_id)?;
    let mut finalization_result = Ok(());
    let operation_result = (|| {
        let inspection = inspect_resume_session(&path, session_id)?;
        if matches!(
            inspection.last_event_type,
            EventType::SessionFailed | EventType::SessionCompleted
        ) {
            return Err(RuntimeError::TerminalSession(session_id.to_owned()));
        }
        let logs = open_anchored_runtime_dir(&execution_workspace, "logs")?
            .ok_or_else(|| missing_definition_metadata(session_id))?;
        let inventory = SessionBundleInventory::inspect(SessionBundlePaths::new(
            sessions.clone(),
            logs.clone(),
            session_id,
        ))?;
        inventory.validate_resumable_bundle()?;
        let metadata = require_anchored_session_log_metadata(&logs, session_id)?;
        let flow_id = resumable_flow_id(
            path.diagnostic_path(),
            session_id,
            inspection.root_flow_definition_id.as_deref(),
            &metadata,
        )?;

        let registry = core_script::load_flow_registry_from_workspace_dir(
            &execution_workspace.root().dir,
            workspace,
            &config.registry_root,
            &flow_id,
        )?;
        let flow_block = registry.flow_block(&flow_id).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing flow {flow_id}"))
        })?;
        verify_resume_definition_metadata_values(session_id, &metadata, &registry, flow_block)?;
        let policy =
            core_policy::compile_policy_artifact(&registry, &flow_id, runtime_policy_target())?;
        let clock = resume_event_clock(&config, inspection.clock)?;
        let recorded_context = read_anchored_context_manifest_signature(
            &logs,
            &sessions,
            session_id,
            inspection.completed_turns,
        )?;
        let mut prefix_sink =
            RuntimePrefixSink::new(inspection.event_prefix.clone(), recorded_context.clone());
        let plan = plan_flow_with_workspace(
            &execution_workspace,
            &registry,
            &policy,
            flow_block,
            session_id,
            FlowExecutionOptions::with_stub_model_fixture_profile(
                clock,
                ToolSideEffectMode::Plan,
                config.stub_model_fixture_profile,
            ),
        )?;
        for action in &plan.actions {
            if let FlowExecutionAction::Event(action) = action {
                prefix_sink.commit(
                    &action.event,
                    &action.canonical_jsonl,
                    action.context_checkpoint.clone(),
                    None,
                )?;
            }
        }
        if inspection.completed_turns > plan.execution.context_manifests.record_count
            || recorded_context.record_count > plan.execution.context_manifests.record_count
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
            path.diagnostic_path(),
            &inspection,
            &prefix_sink,
            &plan.execution,
            flow_block,
        )?;
        checked_resume_event_count(
            plan.execution.events.record_count,
            resume_prefix.resume_marker_count,
        )?;
        if let Some(tool_id) = inspection.validation.tool_without_progress() {
            return Err(RuntimeError::Protocol(format!(
                "cannot resume session {session_id} with in-flight tool {tool_id:?} before progress or terminal event"
            )));
        }
        let append_plan = resume_append_plan(session_id, &inspection.validation, clock)?;
        let context_path = SessionBundlePaths::contexts_in(&logs, session_id);
        let preflight_matches = {
            let mut preflight_sink = ResumePreflightSink::open(
                &path,
                &context_path,
                session_id,
                append_plan.marker_stream.len(),
                clock,
                resume_prefix.planned_event_count,
                resume_prefix.resume_marker_count,
            )?;
            let runtime = apply_flow_with_anchored_workspace(
                FlowApplication {
                    #[cfg(test)]
                    workspace,
                    session_id,
                    options: FlowExecutionOptions::with_stub_model_fixture_profile(
                        clock,
                        ToolSideEffectMode::PreflightResume {
                            prefix_event_count: resume_prefix.planned_event_count as u64,
                        },
                        config.stub_model_fixture_profile,
                    ),
                    plan: &plan,
                },
                &execution_workspace,
                Some(&mut preflight_sink),
            )?;
            let matches = runtime.matches_plan(&plan);
            preflight_sink.finish()?;
            matches
        };
        if !preflight_matches {
            return Err(RuntimeError::Protocol(format!(
                "{} resume preflight did not match deterministic replay",
                path.diagnostic_path().display()
            )));
        }
        let mut serial_writer = SerialSessionWriter::start_prevalidated(SerialWriterStart {
            context_path,
            path: path.clone(),
            session_id: session_id.to_owned(),
            validation: inspection.validation,
            commit_reservation: None,
            notifier,
            timings,
        })?;
        if capture_jsonl {
            serial_writer.enable_jsonl_capture();
        }
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
            apply_flow_with_anchored_workspace(
                FlowApplication {
                    #[cfg(test)]
                    workspace,
                    session_id,
                    options: FlowExecutionOptions::with_stub_model_fixture_profile(
                        clock,
                        ToolSideEffectMode::Resume {
                            prefix_event_count: resume_prefix.planned_event_count as u64,
                        },
                        config.stub_model_fixture_profile,
                    ),
                    plan: &plan,
                },
                &execution_workspace,
                Some(&mut resume_sink),
            )
        };
        finalization_result = serial_writer.finish();
        #[cfg(test)]
        post_writer_finish_observer(&path);
        let captured_jsonl = serial_writer.take_captured_jsonl();
        let resumed_runtime = runtime_result?;
        let terminal_error = resumed_runtime.terminal_error;
        let resumed_failed = resumed_runtime.failed;
        let resumed_event_count = resumed_runtime.events.record_count;
        let failure_status = resumed_runtime.failure_status;
        let stdout = if capture_jsonl {
            captured_jsonl.expect("JSONL capture enabled before resumed runtime application")
        } else {
            human_session_status_from_failure(session_id, "resumed", failure_status.as_deref())
        };
        if let Some(err) = terminal_error {
            return Err(RuntimeError::session_failed(session_id, err));
        }
        let combined_event_count =
            checked_resume_event_count(resumed_event_count, resume_prefix.resume_marker_count)?;

        Ok(RunOutput {
            event_count: combined_event_count,
            failed: resumed_failed,
            session_id: session_id.to_owned(),
            session_path: path.diagnostic_path().to_owned(),
            stdout,
        })
    })();
    before_cleanup(&lock.path);
    let cleanup_result = lock.release();
    reconcile_controlled_stages(operation_result, finalization_result, cleanup_result)
}

pub struct ResumeReplayPrefix {
    pub(crate) planned_event_count: usize,
    pub(crate) resume_marker_count: usize,
}

pub struct ResumeAppendPlan {
    pub(crate) marker_event: EventEnvelope,
    pub(crate) marker_stream: String,
}

pub struct ResumeSessionInspection {
    pub(crate) clock: EventClock,
    pub(crate) completed_turns: usize,
    pub(crate) event_prefix: RuntimeStreamSignature,
    pub(crate) last_event_type: EventType,
    pub(crate) prefix_metadata_valid: bool,
    pub(crate) resume_marker_count: usize,
    pub(crate) root_flow_definition_id: Option<String>,
    pub(crate) validation: SessionAppendValidationState,
}

pub struct ResumeInspectionBuilder {
    pub(crate) clock: Option<EventClock>,
    pub(crate) completed_turns: usize,
    pub(crate) event_prefix: RuntimeStreamSignatureBuilder,
    pub(crate) last_event_type: Option<EventType>,
    pub(crate) prefix_metadata_valid: bool,
    pub(crate) resume_marker_count: usize,
    pub(crate) root_flow_definition_id: Option<String>,
}

impl ResumeInspectionBuilder {
    pub(crate) fn new() -> Self {
        Self {
            clock: None,
            completed_turns: 0,
            event_prefix: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            last_event_type: None,
            prefix_metadata_valid: true,
            resume_marker_count: 0,
            root_flow_definition_id: None,
        }
    }

    pub(crate) fn observe(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
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
        self.last_event_type = Some(event.event_type);
        self.completed_turns += usize::from(event.event_type == EventType::MessageCompleted);
        if self.root_flow_definition_id.is_none()
            && event.event_type == EventType::FlowStarted
            && event.parent_flow_id.is_none()
        {
            self.root_flow_definition_id =
                Some(lifecycle_payload_string(event, "flow_definition_id"));
        }
        self.prefix_metadata_valid &= event.event_id == format!("evt-{:03}", event.sequence)
            && event.timestamp == clock.timestamp(event.sequence);
        if event.event_type == EventType::SessionResumed {
            self.resume_marker_count = self.resume_marker_count.saturating_add(1);
            return Ok(());
        }
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
        Ok(())
    }
}

pub fn inspect_resume_session(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<ResumeSessionInspection, RuntimeError> {
    let mut validation = SessionAppendValidationState::empty(session_id);
    let mut inspection = ResumeInspectionBuilder::new();
    for_each_segmented_jsonl_line(path, EVENT_STREAM_LIMITS, |line| {
        validation.validate_appended_with(path.diagnostic_path(), line, |event| {
            inspection.observe(event)
        })
    })?;
    let Some(clock) = inspection.clock else {
        return Err(RuntimeError::Protocol(format!(
            "{} must contain at least one event",
            path.diagnostic_path().display()
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
        prefix_metadata_valid: inspection.prefix_metadata_valid,
        resume_marker_count: inspection.resume_marker_count,
        root_flow_definition_id: inspection.root_flow_definition_id,
        validation,
    })
}

pub fn validate_resume_replay_prefix(
    path: &Path,
    inspection: &ResumeSessionInspection,
    prefix_sink: &RuntimePrefixSink,
    planned: &RuntimeExecution,
    flow_block: &core_script::FlowBlock,
) -> Result<ResumeReplayPrefix, RuntimeError> {
    if !inspection.prefix_metadata_valid
        || inspection.event_prefix.record_count > planned.events.record_count
        || !prefix_sink.event_prefix_matches()
    {
        return Err(invalid_resume_prefix_error(path, flow_block));
    }

    Ok(ResumeReplayPrefix {
        planned_event_count: inspection.event_prefix.record_count,
        resume_marker_count: inspection.resume_marker_count,
    })
}

pub fn checked_resume_event_count(
    planned_event_count: usize,
    resume_marker_count: usize,
) -> Result<usize, RuntimeError> {
    let total = (planned_event_count as u128) + (resume_marker_count as u128) + 1;
    if total > u128::from(MAX_FLOW_EVENTS) {
        return Err(RuntimeError::Protocol(format!(
            "runtime event budget exceeded: resume requires {total} events; max {MAX_FLOW_EVENTS}"
        )));
    }
    Ok(usize::try_from(total).expect("event limit fits usize"))
}

pub fn invalid_resume_prefix_error(
    path: &Path,
    flow_block: &core_script::FlowBlock,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} is not a valid prefix of flow {}",
        path.display(),
        flow_block.identity.id
    ))
}

pub fn resume_append_plan(
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
        "flow-agent-cli",
        serde_json::json!({"reason":"resume"}),
    );
    let marker_stream = canonical_event_stream(std::slice::from_ref(&resume_event))?;
    Ok(ResumeAppendPlan {
        marker_event: resume_event,
        marker_stream,
    })
}

pub fn shift_resumed_event(
    mut event: EventEnvelope,
    sequence_offset: u64,
    clock: EventClock,
) -> EventEnvelope {
    event.sequence += sequence_offset;
    event.event_id = format!("evt-{:03}", event.sequence);
    event.timestamp = clock.timestamp(event.sequence);
    event
}

pub fn resumable_flow_id(
    path: &Path,
    session_id: &str,
    event_flow_id: Option<&str>,
    metadata: &SessionLogMetadata,
) -> Result<String, RuntimeError> {
    let recorded_flow_id = metadata.flow_definition_id.as_deref().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing flow_definition_id metadata"
        ))
    })?;
    if event_flow_id.is_some_and(|event_flow_id| event_flow_id != recorded_flow_id) {
        return Err(RuntimeError::Protocol(format!(
            "{} session {session_id} flow definition metadata does not match durable events",
            path.display()
        )));
    }
    Ok(recorded_flow_id.to_owned())
}

pub struct SessionDefinitionMetadata {
    pub(crate) flow_definition_id: String,
    pub(crate) registry_hash: String,
    pub(crate) flow_definition_hash: String,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct SessionLogMetadata {
    pub(crate) flow_definition_id: Option<String>,
    pub(crate) registry_hash: Option<String>,
    pub(crate) flow_definition_hash: Option<String>,
}

pub fn session_definition_metadata(
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
) -> Result<SessionDefinitionMetadata, RuntimeError> {
    let registry_json = registry.canonical_json()?;
    let flow_json = proto::canonical_json(&serde_json::to_value(flow_block)?).map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize flow definition hash: {err}"))
    })?;
    Ok(SessionDefinitionMetadata {
        flow_definition_id: flow_block.identity.id.clone(),
        registry_hash: sha256_hash_text(registry_json.as_bytes()),
        flow_definition_hash: sha256_hash_text(flow_json.as_bytes()),
    })
}

pub fn sha256_hash_text(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

#[cfg(test)]
pub fn verify_resume_definition_metadata(
    workspace: &Path,
    session_id: &str,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
) -> Result<(), RuntimeError> {
    // WHY: resume hashes bind a partial session to the registry definitions that produced
    // it; incomplete metadata cannot prove the prefix matches the current registry.
    let logs = open_runtime_dir(workspace, "logs")?
        .ok_or_else(|| missing_definition_metadata(session_id))?;
    let metadata = require_anchored_session_log_metadata(&logs, session_id)?;
    verify_resume_definition_metadata_values(session_id, &metadata, registry, flow_block)
}

pub fn verify_resume_definition_metadata_values(
    session_id: &str,
    metadata: &SessionLogMetadata,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
) -> Result<(), RuntimeError> {
    let Some(recorded_registry_hash) = metadata.registry_hash.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing registry_hash metadata"
        )));
    };
    let Some(recorded_flow_definition_hash) = metadata.flow_definition_hash.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing flow_definition_hash metadata"
        )));
    };
    let Some(recorded_flow_definition_id) = metadata.flow_definition_id.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing flow_definition_id metadata"
        )));
    };

    let expected = session_definition_metadata(registry, flow_block)?;
    if recorded_flow_definition_id != expected.flow_definition_id
        || recorded_registry_hash != expected.registry_hash
        || recorded_flow_definition_hash != expected.flow_definition_hash
    {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: recorded definition metadata does not match current registry"
        )));
    }
    Ok(())
}

pub fn require_anchored_session_log_metadata(
    logs: &AnchoredDir,
    session_id: &str,
) -> Result<SessionLogMetadata, RuntimeError> {
    let path = SessionBundlePaths::metadata_in(logs, session_id);
    if let Some(alias) = ascii_case_alias(&path)? {
        return Err(RuntimeError::Protocol(format!(
            "{} contains non-canonical session metadata name {}",
            logs.path.display(),
            alias.leaf.display()
        )));
    }
    ensure_anchored_real_file(&path)
        .map_err(|error| map_missing_definition_metadata(error, session_id))?;
    parse_session_log_metadata(&read_anchored_to_string_with_limit(
        &path,
        MAX_SESSION_METADATA_BYTES,
    )?)
}

pub fn ascii_case_alias(path: &AnchoredFile) -> Result<Option<AnchoredFile>, RuntimeError> {
    let expected = path
        .leaf
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} must have a UTF-8 filename",
                path.diagnostic_path().display()
            ))
        })?;
    for entry in path
        .parent
        .dir
        .entries()
        .map_err(|source| path_io_error(&path.parent.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&path.parent.path, source))?;
        let name = entry.file_name();
        if name
            .to_str()
            .is_some_and(|name| name != expected && name.eq_ignore_ascii_case(expected))
        {
            return Ok(Some(path.parent.file(name)));
        }
    }
    Ok(None)
}

pub fn map_missing_definition_metadata(error: RuntimeError, session_id: &str) -> RuntimeError {
    if matches!(
        &error,
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound
    ) {
        missing_definition_metadata(session_id)
    } else {
        error
    }
}

pub fn missing_definition_metadata(session_id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "session {session_id} registry drift: missing definition metadata"
    ))
}

pub fn parse_session_log_metadata(text: &str) -> Result<SessionLogMetadata, RuntimeError> {
    let mut metadata = SessionLogMetadata::default();
    for (line_number, line) in text.lines().enumerate() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(RuntimeError::Protocol(format!(
                "session metadata line {} is not key=value",
                line_number + 1
            )));
        };
        match key {
            "flow_definition_id" => metadata.flow_definition_id = Some(value.to_owned()),
            "registry_hash" => metadata.registry_hash = Some(value.to_owned()),
            "flow_definition_hash" => metadata.flow_definition_hash = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(metadata)
}
