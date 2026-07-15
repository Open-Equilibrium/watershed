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
    let registry_path = registry_root_path(workspace, &config.registry_root)?;
    let registry = core_script::load_registry_root(registry_path)?;
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
    let planned_events = preflight_session_completion_stream(
        &reservation,
        &expected_session_id,
        &planned_runtime.events,
    )?;
    let mut serial_writer = SerialSessionWriter::start(&reservation, notifier, timings)?;
    let runtime_result = execute_loop_with_sink(
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
    let finish_result = serial_writer.finish();
    let runtime = runtime_result?;
    finish_result?;
    let runtime_failed = runtime.failed;
    if !runtime_failed
        && (runtime.events != planned_runtime.events
            || runtime.context_manifests != planned_runtime.context_manifests)
    {
        return Err(RuntimeError::Protocol(format!(
            "{} runtime did not match deterministic replay",
            reservation.session_path.display()
        )));
    }
    if runtime_failed {
        let stream = canonical_event_stream(&runtime.events)?;
        validate_session_log_text(&reservation.session_path, &expected_session_id, &stream)?;
    } else if runtime.events != planned_events {
        return Err(RuntimeError::Protocol(format!(
            "{} committed runtime did not match its validated plan",
            reservation.session_path.display()
        )));
    }
    reservation.release_lock()?;
    if let Some(err) = runtime.terminal_error {
        return Err(err);
    }
    let status = if let Some(failure) = human_failure_status(&runtime.events) {
        format!("loop {} {failure}\n", loop_block.identity.id)
    } else {
        format!("loop {} completed\n", loop_block.identity.id)
    };
    Ok(RunOutput {
        event_count: runtime.events.len(),
        failed: runtime_failed,
        session_id: expected_session_id,
        session_path: reservation.session_path.clone(),
        stdout: status,
    })
}
