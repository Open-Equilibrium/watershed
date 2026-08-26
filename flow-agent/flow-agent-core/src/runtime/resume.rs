use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_anchored_workspace},
    config_io::{
        load_global_config_authority, require_fixture_execution_backend, resume_event_clock,
    },
    context_persistence::read_anchored_context_manifest_signature,
    event_writer::{
        ResumeEventSink, ResumePreflightSink, RuntimeEventSink, RuntimePrefixSink,
        SerialSessionWriter, SerialWriterStart,
    },
    execution_plan::{
        FlowExecutionAction, FlowExecutionOptions, ToolSideEffectMode, runtime_policy_target,
    },
    fs_guards::{
        AnchoredFile, AnchoredWorkspace, ensure_anchored_non_hardlinked_file,
        open_anchored_runtime_dir,
    },
    live_events::LiveEventNotifier,
    planning::plan_flow_with_workspace,
    resume_inspection::{
        checked_resume_event_count, inspect_resume_session, resume_append_plan,
        validate_resume_replay_prefix,
    },
    segmented_appender::{EventLogAppender, SessionLogAppender},
    session_bundle::{SessionBundleInventory, SessionBundlePaths},
    session_definition::{
        missing_definition_metadata, require_anchored_session_log_metadata, resumable_flow_id,
        verify_resume_definition_metadata_values,
    },
    session_reservation::acquire_anchored_session_lock,
    session_store::workspace_store_path,
    stage_results::reconcile_controlled_stages,
    types::{
        LOG_STORAGE_DIR, RunOutput, RuntimeError, SESSION_STORAGE_DIR,
        human_run_status_from_failure,
    },
};
#[cfg(test)]
use crate::runtime::{
    conversations::legacy_flat_compatibility_is_available,
    event_writer::{EventWriterTimings, post_writer_finish_observer},
    types::{EmitMode, human_session_status_from_failure},
};
use proto::EventType;
use std::{io, path::Path};

/// Resumes one compatible persisted legacy flat session without repeating completed work.
#[cfg(test)]
pub(crate) fn resume_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let _ = legacy_flat_compatibility_is_available(workspace, session_id)?;
    resume_session_internal(workspace, session_id, None, None, emit == EmitMode::Jsonl)
}

/// Resumes a session with bounded, non-blocking committed-event notifications.
///
/// The caller owns the receiver and any blocking transport. Notifications cover only newly
/// committed events; read their payloads from [`crate::SessionEventReader`] by sequence.
#[cfg(test)]
pub(crate) fn resume_session_with_live_events(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let _ = legacy_flat_compatibility_is_available(workspace, session_id)?;
    let mut output = resume_session_internal(workspace, session_id, Some(notifier), None, false)?;
    output.stdout.clear();
    Ok(output)
}

#[cfg(test)]
pub(crate) fn resume_session_internal(
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
        human_session_status_from_failure,
        |_| {},
    )
}

pub(crate) fn resume_migrating_conversation_run_internal(
    workspace: impl AsRef<Path>,
    run_session_id: &str,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    resume_session_internal_with_cleanup_observer_impl(
        workspace,
        run_session_id,
        notifier,
        #[cfg(test)]
        None,
        capture_jsonl,
        human_run_status_from_failure,
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
        human_session_status_from_failure,
        before_cleanup,
    )
}

fn resume_session_internal_with_cleanup_observer_impl(
    workspace: impl AsRef<Path>,
    session_id: &str,
    notifier: Option<LiveEventNotifier>,
    #[cfg(test)] timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
    human_status: fn(&str, &str, Option<&str>) -> String,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    if !proto::is_valid_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    let execution_workspace = AnchoredWorkspace::open(workspace)?;
    let authority = load_global_config_authority()?;
    let config = &authority.config;
    require_fixture_execution_backend(config)?;
    let session_dir_path = workspace_store_path(&execution_workspace)?.join(SESSION_STORAGE_DIR);
    let sessions = open_anchored_runtime_dir(&execution_workspace, SESSION_STORAGE_DIR)?
        .ok_or_else(|| RuntimeError::Io {
            path: session_dir_path,
            source: io::Error::from(io::ErrorKind::NotFound),
        })?;
    let path = SessionBundlePaths::events_in(&sessions, session_id);
    ensure_anchored_non_hardlinked_file(&path)?;
    let lock = acquire_anchored_session_lock(&execution_workspace, &sessions, session_id)?;
    let mut finalization_result = Ok(());
    let operation_result = (|| {
        SessionLogAppender::open(&path)?.sync(path.diagnostic_path())?;
        let inspection = inspect_resume_session(&path, session_id)?;
        if matches!(
            inspection.last_event_type,
            EventType::SessionFailed | EventType::SessionCompleted
        ) {
            return Err(RuntimeError::TerminalSession(session_id.to_owned()));
        }
        let logs = open_anchored_runtime_dir(&execution_workspace, LOG_STORAGE_DIR)?
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

        let registry = core_script::load_flow_registry_from_root_dir(
            &authority.home.dir,
            &authority.home.path,
            &config.registry_root,
            &flow_id,
        )?;
        let flow_block = registry
            .flow_block(&flow_id)
            .expect("the loaded root Flow remains in the resolved registry");
        verify_resume_definition_metadata_values(session_id, &metadata, &registry, flow_block)?;
        let policy =
            core_policy::compile_policy_artifact(&registry, &flow_id, runtime_policy_target())?;
        let clock = resume_event_clock(config, inspection.clock)?;
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
        for action in plan.actions.iter() {
            if let FlowExecutionAction::Event(action) = action {
                prefix_sink.commit(
                    &action.event,
                    &action.canonical_jsonl,
                    action.context_checkpoint.clone(),
                    #[cfg(test)]
                    None,
                )?;
            }
        }
        if inspection.completed_turns > plan.execution.context_manifests.record_count
            || recorded_context.record_count > plan.execution.context_manifests.record_count
            || !prefix_sink.context_prefix_matches()
        {
            let context_path = SessionBundlePaths::contexts_in(&logs, session_id)
                .diagnostic_path()
                .to_owned();
            return Err(RuntimeError::Protocol(format!(
                "{} context manifests do not match deterministic replay",
                context_path.display()
            )));
        }
        let resume_prefix = validate_resume_replay_prefix(
            path.diagnostic_path(),
            &inspection,
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
            #[cfg(test)]
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
            human_status(session_id, "resumed", failure_status.as_deref())
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
