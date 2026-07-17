/// Runs a loop from a workspace registry and captures its output.
pub fn run_loop(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let mut output = run_loop_internal(workspace, loop_ref, None, None)?;
    if emit == EmitMode::Jsonl {
        output.stdout = read_session_log_to_string(&output.session_path)?;
    }
    Ok(output)
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
    let mut output = run_loop_internal(workspace, loop_ref, Some(notifier), None)?;
    output.stdout.clear();
    Ok(output)
}

fn run_loop_internal(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let config = load_workspace_config(workspace)?;
    let registry = core_script::load_loop_registry_from_workspace(
        workspace,
        &config.registry_root,
        loop_ref,
    )?;
    let loop_block = registry
        .loop_block(loop_ref)
        .ok_or_else(|| RuntimeError::Usage(format!("unknown loop {loop_ref}")))?;
    let definition_hashes = session_definition_hashes(&registry, loop_block)?;
    let policy = core_policy::compile_policy_artifact(
        &loop_block.identity.id,
        &registry,
        loop_ref,
        runtime_policy_target(),
    )?;
    preflight_loop_tools(workspace, &registry, &policy, loop_block)?;
    let base_session_id = session_id_for_loop(&loop_block.identity.id);
    let reservation = reserve_unique_session_log(workspace, &base_session_id)?;
    let expected_session_id = reservation.session_id.clone();
    write_reserved_session_metadata(&reservation, Some(&definition_hashes))?;
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
    preflight_session_completion_stream(
        &reservation,
        &expected_session_id,
        &planned_runtime.events,
    )?;
    let mut serial_writer = SerialSessionWriter::start(&reservation, notifier, timings)?;
    let (runtime_result, replay_matches) = {
        let mut matcher = PlannedRuntimeSink::new(&planned_runtime, Some(&mut serial_writer));
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
            )
            .without_captured_output(),
            Some(&mut matcher),
        );
        let matches = result
            .as_ref()
            .is_ok_and(|runtime| matcher.matches_execution(runtime));
        (result, matches)
    };
    let finish_result = serial_writer.finish();
    let runtime = runtime_result?;
    finish_result?;
    let runtime_failed = runtime.failed;
    if !runtime_failed && !replay_matches {
        return Err(RuntimeError::Protocol(format!(
            "{} runtime did not match deterministic replay",
            reservation.session_path.display()
        )));
    }
    let terminal_error = runtime.terminal_error;
    let (event_count, outcome) = if runtime_failed {
        drop(planned_runtime);
        let stream = read_session_log_to_string(&reservation.session_path)?;
        let events =
            validate_session_log_text(&reservation.session_path, &expected_session_id, &stream)?;
        (
            events.len(),
            human_failure_status(&events).unwrap_or_else(|| "failed".to_owned()),
        )
    } else {
        (planned_runtime.events.len(), "completed".to_owned())
    };
    reservation.release_lock()?;
    if let Some(err) = terminal_error {
        return Err(RuntimeError::session_failed(&expected_session_id, err));
    }
    let status = format!(
        "loop {} (session {expected_session_id}) {outcome}\n",
        loop_block.identity.id
    );
    Ok(RunOutput {
        event_count,
        failed: runtime_failed,
        session_id: expected_session_id,
        session_path: reservation.session_path.clone(),
        stdout: status,
    })
}
