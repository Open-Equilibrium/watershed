#[cfg(test)]
use crate::runtime::event_writer::post_writer_finish_observer;
use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_anchored_workspace},
    config_io::{
        GlobalConfigAuthority, load_global_config_authority, require_fixture_execution_backend,
        resume_event_clock,
    },
    context_persistence::read_anchored_context_manifest_signature,
    event_writer::{
        ResumeEventSink, ResumePreflightSink, RuntimeEventSink, RuntimePrefixSink,
        SerialSessionWriter, SerialWriterStart,
    },
    execution_plan::{
        FlowExecutionAction, FlowExecutionOptions, FlowExecutionPlan, ToolSideEffectMode,
    },
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredWorkspace, ensure_anchored_non_hardlinked_file,
    },
    live_events::LiveEventNotifier,
    planning::plan_flow_with_workspace,
    resume_inspection::{
        ResumeAppendPlan, ResumeReplayPrefix, ResumeSessionInspection, checked_resume_event_count,
        inspect_resume_session, resume_append_plan, validate_resume_replay_prefix,
    },
    segmented_appender::{EventLogAppender, SessionLogAppender},
    session_bundle::{SessionBundleInventory, SessionBundlePaths},
    session_definition::{
        SessionLogMetadata, missing_definition_metadata, require_anchored_session_log_metadata,
        resumable_flow_id, verify_resume_definition_metadata_values,
    },
    session_reservation::acquire_anchored_session_lock,
    session_store::{open_anchored_runtime_dir, workspace_store_path},
    stage_results::reconcile_controlled_stages,
    types::{
        EventClock, LOG_STORAGE_DIR, RunOutput, RuntimeError, SESSION_STORAGE_DIR,
        human_run_status_from_failure,
    },
};
use proto::EventType;
use std::{io, path::Path};

/// Resumes one current Fixture session without repeating completed work.
pub(crate) fn resume_fixture_session_internal(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    if !proto::is_valid_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    let execution_workspace = AnchoredWorkspace::open(workspace)?;
    let authority = load_global_config_authority()?;
    require_fixture_execution_backend(&authority.config)?;
    let session_dir_path = workspace_store_path(&execution_workspace)?.join(SESSION_STORAGE_DIR);
    let sessions = open_anchored_runtime_dir(&execution_workspace, SESSION_STORAGE_DIR)?
        .ok_or_else(|| RuntimeError::Io {
            path: session_dir_path,
            source: io::Error::from(io::ErrorKind::NotFound),
        })?;
    let path = SessionBundlePaths::events_in(&sessions, session_id);
    ensure_anchored_non_hardlinked_file(&path)?;
    let lock = acquire_anchored_session_lock(&execution_workspace, &sessions, session_id)?;
    let session = LockedSession {
        #[cfg(test)]
        workspace,
        anchored: &execution_workspace,
        authority: &authority,
        sessions: &sessions,
        path: &path,
        session_id,
    };
    let mut finalization_result = Ok(());
    let operation_result =
        resume_locked_session(&session, notifier, capture_jsonl, &mut finalization_result);
    let cleanup_result = lock.release();
    reconcile_controlled_stages(operation_result, finalization_result, cleanup_result)
}

/// Handles of one Fixture session whose session lock the caller already holds.
struct LockedSession<'a> {
    #[cfg(test)]
    workspace: &'a Path,
    anchored: &'a AnchoredWorkspace,
    authority: &'a GlobalConfigAuthority,
    sessions: &'a AnchoredDir,
    path: &'a AnchoredFile,
    session_id: &'a str,
}

/// Deterministic replay plus the append and context evidence that a resume writes against.
struct ResumePlan {
    plan: FlowExecutionPlan,
    clock: EventClock,
    prefix: ResumeReplayPrefix,
    append: ResumeAppendPlan,
    context_path: AnchoredFile,
}

/// Runs the locked resume phases in their durable order.
fn resume_locked_session(
    session: &LockedSession<'_>,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
    finalization_result: &mut Result<(), RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    SessionLogAppender::open(session.path)?.sync(session.path.diagnostic_path())?;
    let inspection = inspect_resume_session(session.path, session.session_id)?;
    if matches!(
        inspection.last_event_type,
        EventType::SessionFailed | EventType::SessionCompleted
    ) {
        return Err(RuntimeError::TerminalSession(session.session_id.to_owned()));
    }
    let logs = open_resumable_logs(session)?;
    let metadata = require_anchored_session_log_metadata(&logs, session.session_id)?;
    let flow_id = resumable_flow_id(
        session.path.diagnostic_path(),
        session.session_id,
        inspection.root_flow_definition_id.as_deref(),
        &metadata,
    )?;
    let resume = plan_resume(session, &logs, &inspection, &metadata, &flow_id)?;
    preflight_resume(session, &resume)?;
    run_resume(
        session,
        resume,
        inspection,
        notifier,
        capture_jsonl,
        finalization_result,
    )
}

/// Opens the session's log directory and validates the bundle that it belongs to.
fn open_resumable_logs(session: &LockedSession<'_>) -> Result<AnchoredDir, RuntimeError> {
    let logs = open_anchored_runtime_dir(session.anchored, LOG_STORAGE_DIR)?
        .ok_or_else(|| missing_definition_metadata(session.session_id))?;
    let inventory = SessionBundleInventory::inspect(SessionBundlePaths::new(
        session.sessions.clone(),
        logs.clone(),
        session.session_id,
    ))?;
    inventory.validate_resumable_bundle()?;
    Ok(logs)
}

/// Replays the deterministic plan and proves that the recorded prefix and context manifests match it.
fn plan_resume(
    session: &LockedSession<'_>,
    logs: &AnchoredDir,
    inspection: &ResumeSessionInspection,
    metadata: &SessionLogMetadata,
    flow_id: &str,
) -> Result<ResumePlan, RuntimeError> {
    let config = &session.authority.config;
    let registry = core_script::load_flow_registry_from_root_dir(
        &session.authority.home.dir,
        &session.authority.home.path,
        &config.registry_root,
        flow_id,
    )?;
    let flow_block = registry
        .flow_block(flow_id)
        .expect("the loaded root Flow remains in the resolved registry");
    verify_resume_definition_metadata_values(session.session_id, metadata, &registry, flow_block)?;
    let policy = core_policy::compile_policy_artifact(&registry, flow_id)?;
    let clock = resume_event_clock(config, inspection.clock)?;
    let recorded_context = read_anchored_context_manifest_signature(
        logs,
        session.sessions,
        session.session_id,
        inspection.completed_turns,
    )?;
    let mut prefix_sink =
        RuntimePrefixSink::new(inspection.event_prefix.clone(), recorded_context.clone());
    let plan = plan_flow_with_workspace(
        session.anchored,
        &registry,
        &policy,
        flow_block,
        session.session_id,
        FlowExecutionOptions::with_stub_model_fixture_profile(
            clock,
            ToolSideEffectMode::Plan,
            config.stub_model_fixture_profile,
        ),
    )?;
    for action in plan.execution.actions.iter() {
        if let FlowExecutionAction::Event(action) = action {
            prefix_sink.commit_constructed(&action.event, action.context_checkpoint.clone())?;
        }
    }
    if inspection.completed_turns > plan.execution.context_manifests.record_count
        || recorded_context.record_count > plan.execution.context_manifests.record_count
        || !prefix_sink.context_prefix_matches()
    {
        let context_path = SessionBundlePaths::contexts_in(logs, session.session_id)
            .diagnostic_path()
            .to_owned();
        return Err(RuntimeError::Protocol(format!(
            "{} context manifests do not match deterministic replay",
            context_path.display()
        )));
    }
    let resume_prefix = validate_resume_replay_prefix(
        session.path.diagnostic_path(),
        inspection,
        prefix_sink.event_prefix_matches(),
        &plan.execution,
        flow_block,
    )?;
    checked_resume_event_count(
        plan.execution.events.record_count,
        resume_prefix.resume_marker_count,
    )?;
    if let Some(tool_id) = inspection.validation.tool_without_progress() {
        return Err(RuntimeError::Protocol(format!(
            "cannot resume session {} with in-flight tool {tool_id:?} before progress or terminal event",
            session.session_id
        )));
    }
    let append = resume_append_plan(session.session_id, &inspection.validation, clock)?;
    let context_path = SessionBundlePaths::contexts_in(logs, session.session_id);
    Ok(ResumePlan {
        plan,
        clock,
        prefix: resume_prefix,
        append,
        context_path,
    })
}

/// Replays the resume against a preflight sink so that any mismatch is rejected before a write.
fn preflight_resume(session: &LockedSession<'_>, resume: &ResumePlan) -> Result<(), RuntimeError> {
    let preflight_matches = {
        let mut preflight_sink = ResumePreflightSink::open(
            session.path,
            &resume.context_path,
            session.session_id,
            resume.append.marker_stream.len(),
            resume.clock,
            resume.prefix.planned_event_count,
            resume.prefix.resume_marker_count,
        )?;
        let runtime = apply_flow_with_anchored_workspace(
            FlowApplication {
                #[cfg(test)]
                workspace: session.workspace,
                session_id: session.session_id,
                options: FlowExecutionOptions::with_stub_model_fixture_profile(
                    resume.clock,
                    ToolSideEffectMode::PreflightResume {
                        prefix_event_count: resume.prefix.planned_event_count as u64,
                    },
                    session.authority.config.stub_model_fixture_profile,
                ),
                plan: &resume.plan,
            },
            session.anchored,
            Some(&mut preflight_sink),
        )?;
        runtime.matches_plan(&resume.plan)
    };
    if !preflight_matches {
        return Err(RuntimeError::Protocol(format!(
            "{} resume preflight did not match deterministic replay",
            session.path.diagnostic_path().display()
        )));
    }
    Ok(())
}

/// Writes the resumed suffix through the serial writer and reports the resumed run.
fn run_resume(
    session: &LockedSession<'_>,
    resume: ResumePlan,
    inspection: ResumeSessionInspection,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
    finalization_result: &mut Result<(), RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    let ResumePlan {
        plan,
        clock,
        prefix,
        append,
        context_path,
    } = resume;
    let mut serial_writer = SerialSessionWriter::start_prevalidated(SerialWriterStart {
        context_path,
        path: session.path.clone(),
        session_id: session.session_id.to_owned(),
        validation: inspection.validation,
        commit_reservation: None,
        notifier,
    })?;
    if capture_jsonl {
        serial_writer.enable_jsonl_capture();
    }
    let runtime_result = {
        let mut resume_sink = ResumeEventSink {
            clock,
            marker_committed: false,
            marker_event: append.marker_event,
            marker_stream: append.marker_stream,
            planned_event_count: prefix.planned_event_count,
            resume_marker_count: prefix.resume_marker_count,
            writer: &mut serial_writer,
        };
        apply_flow_with_anchored_workspace(
            FlowApplication {
                #[cfg(test)]
                workspace: session.workspace,
                session_id: session.session_id,
                options: FlowExecutionOptions::with_stub_model_fixture_profile(
                    clock,
                    ToolSideEffectMode::Resume {
                        prefix_event_count: prefix.planned_event_count as u64,
                    },
                    session.authority.config.stub_model_fixture_profile,
                ),
                plan: &plan,
            },
            session.anchored,
            Some(&mut resume_sink),
        )
    };
    *finalization_result = serial_writer.finish();
    #[cfg(test)]
    post_writer_finish_observer(session.path);
    let captured_jsonl = serial_writer.take_captured_jsonl();
    let resumed_runtime = runtime_result?;
    let terminal_error = resumed_runtime.terminal_error;
    let resumed_failed = resumed_runtime.failed;
    let resumed_event_count = resumed_runtime.events.record_count;
    let failure_status = resumed_runtime.failure_status;
    let stdout = if capture_jsonl {
        captured_jsonl.expect("JSONL capture enabled before resumed runtime application")
    } else {
        human_run_status_from_failure(session.session_id, "resumed", failure_status.as_deref())
    };
    if let Some(err) = terminal_error {
        return Err(RuntimeError::session_failed(session.session_id, err));
    }
    let combined_event_count =
        checked_resume_event_count(resumed_event_count, prefix.resume_marker_count)?;

    Ok(RunOutput {
        event_count: combined_event_count,
        failed: resumed_failed,
        session_id: session.session_id.to_owned(),
        session_path: session.path.diagnostic_path().to_owned(),
        stdout,
    })
}
