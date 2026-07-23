/// Runs a loop from a workspace registry and captures its output.
pub fn run_loop(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    run_loop_internal(workspace, loop_ref, None, None, emit == EmitMode::Jsonl)
}

/// Runs a loop with bounded, non-blocking committed-event notifications.
///
/// The caller owns the receiver and any blocking transport. Notifications carry only a
/// high-watermark wake-up; read event payloads from [`SessionEventReader`] by sequence.
pub fn run_loop_with_live_events(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut output = run_loop_internal(workspace, loop_ref, Some(notifier), None, false)?;
    output.stdout.clear();
    Ok(output)
}

fn run_loop_internal(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let config = load_workspace_config(workspace)?;
    let registry =
        core_script::load_loop_registry_from_workspace(workspace, &config.registry_root, loop_ref)?;
    let loop_block = registry
        .loop_block(loop_ref)
        .ok_or_else(|| RuntimeError::Usage(format!("unknown loop {loop_ref}")))?;
    let definition_metadata = session_definition_metadata(&registry, loop_block)?;
    let policy =
        core_policy::compile_policy_artifact(&registry, loop_ref, runtime_policy_target())?;
    preflight_loop_tools(workspace, &registry, &policy, loop_block)?;
    let base_session_id = &loop_block.identity.id;
    let reservation = reserve_unique_session_log(workspace, base_session_id)?;
    let expected_session_id = reservation.session_id.clone();
    write_reserved_session_metadata(&reservation, Some(&definition_metadata))?;
    let planned_runtime = execute_loop(
        workspace,
        &registry,
        &policy,
        loop_block,
        &expected_session_id,
        LoopExecutionOptions::with_stub_model_fixture_profile(
            config.event_clock,
            ToolSideEffectMode::DryRun,
            config.stub_model_fixture_profile,
        ),
    )?;
    let mut serial_writer = SerialSessionWriter::start(&reservation, notifier, timings)?;
    let (runtime_result, replay_matches) = {
        let result = execute_loop_with_sink(
            workspace,
            &registry,
            &policy,
            loop_block,
            &expected_session_id,
            LoopExecutionOptions::with_stub_model_fixture_profile(
                config.event_clock,
                ToolSideEffectMode::ApplyAll,
                config.stub_model_fixture_profile,
            ),
            Some(&mut serial_writer),
        );
        let matches = result
            .as_ref()
            .is_ok_and(|runtime| runtime.matches_plan(&planned_runtime));
        (result, matches)
    };
    let finish_result = serial_writer.finish();
    let runtime = runtime_result?;
    finish_result?;
    let runtime_failed = runtime.failed;
    if !runtime_failed && !replay_matches {
        return Err(RuntimeError::Protocol(format!(
            "{} runtime did not match deterministic replay",
            reservation.session_path.diagnostic_path().display()
        )));
    }
    let event_count = runtime.events.record_count;
    let outcome = runtime.failure_status.unwrap_or_else(|| {
        if runtime_failed {
            "failed"
        } else {
            "completed"
        }
        .to_owned()
    });
    let terminal_error = runtime.terminal_error;
    reservation.release_lock()?;
    if let Some(err) = terminal_error {
        return Err(RuntimeError::session_failed(&expected_session_id, err));
    }
    let stdout = if capture_jsonl {
        read_segmented_jsonl(&reservation.session_path, EVENT_STREAM_LIMITS)?
    } else {
        format!(
            "loop {} (session {expected_session_id}) {outcome}\n",
            loop_block.identity.id
        )
    };
    Ok(RunOutput {
        event_count,
        failed: runtime_failed,
        session_id: expected_session_id,
        session_path: reservation.session_path.diagnostic_path().to_owned(),
        stdout,
    })
}
