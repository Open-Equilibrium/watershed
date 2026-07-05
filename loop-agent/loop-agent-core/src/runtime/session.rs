/// Runs a loop from a workspace registry and persists a new session log.
pub fn run_loop(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let config = load_workspace_config(workspace)?;
    let registry_path = registry_root_path(workspace, &config.registry_root)?;
    let registry = core_script::load_registry_root(registry_path)?;
    let loop_block = registry
        .loop_block(loop_ref)
        .ok_or_else(|| RuntimeError::Usage(format!("unknown loop {loop_ref}")))?;
    let definition_hashes = session_definition_hashes(&registry, loop_block)?;
    let artifacts =
        core_policy::compile_policy_artifacts(&loop_block.identity.id, &registry, loop_ref)?;
    let policy = runtime_policy_artifact(&artifacts)?;
    preflight_loop_tools(workspace, &registry, policy, loop_block)?;
    let base_session_id = session_id_for_loop(&loop_block.identity.id);
    let reservation = reserve_unique_session_log(workspace, &base_session_id)?;
    let expected_session_id = reservation.session_id.clone();
    if let Err(err) =
        write_initial_session_log_with_clock(&reservation, &expected_session_id, config.event_clock)
    {
        reservation.rollback();
        return Err(err);
    }
    if let Err(err) = write_reserved_session_metadata(
        &reservation,
        &expected_session_id,
        1,
        Some(&definition_hashes),
    ) {
        reservation.rollback();
        return Err(err);
    }
    let planned_runtime = match execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        &expected_session_id,
        LoopExecutionOptions::with_stub_model_fixture_profile(
            config.event_clock,
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
            config.stub_model_fixture_profile,
        ),
    ) {
        Ok(runtime) => runtime,
        Err(err) => {
            reservation.rollback();
            return Err(err);
        }
    };
    let (planned_stream, planned_events) = match preflight_session_completion_stream(
        &reservation,
        &expected_session_id,
        &planned_runtime.events,
    ) {
        Ok(planned) => planned,
        Err(err) => {
            reservation.rollback();
            return Err(err);
        }
    };
    let durable_prefix_event_count = durable_run_prefix_event_count(&planned_events);
    if let Err(err) = persist_reserved_session_prefix(
        &reservation,
        &expected_session_id,
        &planned_events,
        durable_prefix_event_count,
        Some(&definition_hashes),
    ) {
        reservation.rollback();
        return Err(err);
    }
    let result = (|| {
        let session_id = planned_events
            .first()
            .expect("validated streams contain at least one event")
            .session_id
            .clone();
        let runtime = execute_loop(
            workspace,
            &registry,
            policy,
            loop_block,
            &expected_session_id,
            LoopExecutionOptions::with_stub_model_fixture_profile(
                config.event_clock,
                ToolSideEffectMode::ApplyAll,
                SideEffectRecorder::for_reservation(&reservation),
                config.stub_model_fixture_profile,
            ),
        )?;
        reservation.mark_side_effects_applied();
        let runtime_failed = runtime.failed;
        let terminal_error = runtime.terminal_error;
        if !runtime_failed && runtime.events != planned_runtime.events {
            return Err(RuntimeError::Protocol(format!(
                "{} runtime did not match deterministic replay",
                reservation.session_path.display()
            )));
        }
        let (final_stream, final_events) = if runtime_failed {
            preflight_session_completion_stream_from_prefix(
                &reservation,
                &expected_session_id,
                &runtime.events,
                durable_prefix_event_count,
            )?
        } else {
            (planned_stream, planned_events)
        };
        commit_reserved_session_log_from_prefix(
            &reservation,
            &session_id,
            &final_stream,
            final_events.len(),
            Some(&definition_hashes),
            durable_prefix_event_count,
        )?;
        reservation.release_lock()?;
        if let Some(err) = terminal_error {
            return Err(err);
        }

        Ok(RunOutput {
            event_count: final_events.len(),
            failed: runtime_failed,
            session_id,
            session_path: reservation.session_path.clone(),
            stdout: match emit {
                EmitMode::Jsonl => final_stream,
                EmitMode::Human if runtime_failed => {
                    format!("loop {} failed\n", loop_block.identity.id)
                }
                EmitMode::Human => format!("loop {} completed\n", loop_block.identity.id),
            },
        })
    })();
    if result.is_err() {
        reservation.rollback();
    }
    result
}
