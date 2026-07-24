use super::*;

pub(crate) fn reconcile_controlled_stages<T>(
    operation: Result<T, RuntimeError>,
    finalization: Result<(), RuntimeError>,
    cleanup: Result<(), RuntimeError>,
) -> Result<T, RuntimeError> {
    match (operation, finalization, cleanup) {
        (Ok(value), Ok(()), Ok(())) => Ok(value),
        (Err(error), Ok(()), Ok(()))
        | (Ok(_), Err(error), Ok(()))
        | (Ok(_), Ok(()), Err(error)) => Err(error),
        (operation, finalization, cleanup) => Err(RuntimeError::ControlledStageFailures {
            operation: operation.err().map(Box::new),
            finalization: finalization.err().map(Box::new),
            cleanup: cleanup.err().map(Box::new),
        }),
    }
}

/// Runs a flow from a workspace registry and captures its output.
pub fn run_flow(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal(workspace, flow_ref, None, None, emit == EmitMode::Jsonl)
}

/// Runs a flow with bounded, non-blocking committed-event notifications.
///
/// The caller owns the receiver and any blocking transport. Notifications carry only a
/// high-watermark wake-up; read event payloads from [`SessionEventReader`] by sequence.
pub fn run_flow_with_live_events(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut output = run_flow_internal(workspace, flow_ref, Some(notifier), None, false)?;
    output.stdout.clear();
    Ok(output)
}

pub fn run_flow_internal(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_cleanup_observer_impl(
        workspace,
        flow_ref,
        notifier,
        timings,
        capture_jsonl,
        |result| result,
        |_| {},
    )
}

#[cfg(test)]
pub(crate) fn run_flow_internal_with_cleanup_observer(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    capture_jsonl: bool,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_cleanup_observer_impl(
        workspace,
        flow_ref,
        None,
        None,
        capture_jsonl,
        |result| result,
        before_cleanup,
    )
}

#[cfg(test)]
pub(crate) fn run_flow_internal_with_stage_observers(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    capture_jsonl: bool,
    after_finalization: impl FnOnce(Result<(), RuntimeError>) -> Result<(), RuntimeError>,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_cleanup_observer_impl(
        workspace,
        flow_ref,
        None,
        None,
        capture_jsonl,
        after_finalization,
        before_cleanup,
    )
}

fn run_flow_internal_with_cleanup_observer_impl(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
    after_finalization: impl FnOnce(Result<(), RuntimeError>) -> Result<(), RuntimeError>,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let config = load_workspace_config(workspace)?;
    require_fixture_execution_backend(&config)?;
    let registry =
        core_script::load_flow_registry_from_workspace(workspace, &config.registry_root, flow_ref)?;
    let flow_block = registry
        .flow_block(flow_ref)
        .ok_or_else(|| RuntimeError::Usage(format!("unknown flow {flow_ref}")))?;
    let definition_metadata = session_definition_metadata(&registry, flow_block)?;
    let policy =
        core_policy::compile_policy_artifact(&registry, flow_ref, runtime_policy_target())?;
    preflight_flow_tools(workspace, &registry, &policy, flow_block)?;
    let base_session_id = &flow_block.identity.id;
    let reservation = reserve_unique_session_log(workspace, base_session_id)?;
    let expected_session_id = reservation.session_id.clone();
    let mut finalization_result = Ok(());
    let operation_result = (|| {
        write_reserved_session_metadata(&reservation, Some(&definition_metadata))?;
        let plan = plan_flow(
            workspace,
            &registry,
            &policy,
            flow_block,
            &expected_session_id,
            FlowExecutionOptions::with_stub_model_fixture_profile(
                config.event_clock,
                ToolSideEffectMode::Plan,
                config.stub_model_fixture_profile,
            ),
        )?;
        let mut serial_writer = SerialSessionWriter::start(&reservation, notifier, timings)?;
        let runtime_result = apply_flow_with_sink(
            FlowApplication {
                workspace,
                registry: &registry,
                policy: &policy,
                root_flow: flow_block,
                session_id: &expected_session_id,
                options: FlowExecutionOptions::with_stub_model_fixture_profile(
                    config.event_clock,
                    ToolSideEffectMode::Apply,
                    config.stub_model_fixture_profile,
                ),
                plan: &plan,
            },
            Some(&mut serial_writer),
        );
        finalization_result = serial_writer.finish();
        let runtime = runtime_result?;
        let runtime_failed = runtime.failed;
        let event_count = runtime.events.record_count;
        let outcome = runtime.failure_status.unwrap_or_else(|| {
            if runtime_failed {
                "failed"
            } else {
                "completed"
            }
            .to_owned()
        });
        if let Some(err) = runtime.terminal_error {
            return Err(RuntimeError::session_failed(&expected_session_id, err));
        }
        let stdout = if capture_jsonl {
            read_segmented_jsonl(&reservation.session_path, EVENT_STREAM_LIMITS)?
        } else {
            format!(
                "flow {} (session {expected_session_id}) {outcome}\n",
                flow_block.identity.id
            )
        };
        Ok(RunOutput {
            event_count,
            failed: runtime_failed,
            session_id: expected_session_id,
            session_path: reservation.session_path.diagnostic_path().to_owned(),
            stdout,
        })
    })();
    finalization_result = after_finalization(finalization_result);
    before_cleanup(&reservation.lock_path);
    let cleanup_result = reservation.cleanup();
    reconcile_controlled_stages(operation_result, finalization_result, cleanup_result)
}
