#[derive(Clone, Default)]
struct CapturedOutput {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
}

impl Write for CapturedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .map_err(|_| io::Error::other("runtime output lock was poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Runs a loop from a workspace registry and captures its output.
pub fn run_loop(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let stdout = CapturedOutput::default();
    let captured = stdout.bytes.clone();
    let mut output = run_loop_to_writer(workspace, loop_ref, emit, stdout)?;
    let stdout = captured
        .lock()
        .map_err(|_| RuntimeError::Protocol("runtime output lock was poisoned".to_owned()))?
        .clone();
    output.stdout = String::from_utf8(stdout).map_err(|source| {
        RuntimeError::Protocol(format!("runtime emitted non-UTF-8 output: {source}"))
    })?;
    Ok(output)
}

/// Runs a loop and publishes committed output incrementally to `writer`.
///
/// JSONL events are written only after their identical canonical bytes have been appended to
/// the session log. A failed observer is detached; callers can catch up through replay by
/// `sequence` and `event_id`.
pub fn run_loop_to_writer<W>(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
    writer: W,
) -> Result<RunOutput, RuntimeError>
where
    W: Write + Send + 'static,
{
    run_loop_to_writer_internal(workspace, loop_ref, emit, writer, None)
}

#[cfg(test)]
fn run_loop_to_writer_with_timings<W>(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
    writer: W,
    timings: &mut EventWriterTimings,
) -> Result<RunOutput, RuntimeError>
where
    W: Write + Send + 'static,
{
    run_loop_to_writer_internal(workspace, loop_ref, emit, writer, Some(timings))
}

fn run_loop_to_writer_internal<W>(
    workspace: impl AsRef<Path>,
    loop_ref: &str,
    emit: EmitMode,
    writer: W,
    timings: Option<&mut EventWriterTimings>,
) -> Result<RunOutput, RuntimeError>
where
    W: Write + Send + 'static,
{
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
    if let Err(err) = write_reserved_session_metadata(
        &reservation,
        &expected_session_id,
        0,
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
    let planned_events = match preflight_session_completion_stream(
        &reservation,
        &expected_session_id,
        &planned_runtime.events,
    ) {
        Ok((_, events)) => events,
        Err(err) => {
            reservation.rollback();
            return Err(err);
        }
    };
    if let Err(err) = persist_reserved_context_manifests(
        &reservation,
        &planned_runtime.context_manifests,
    ) {
        reservation.rollback();
        return Err(err);
    }

    let result = (|| {
        let mut serial_writer =
            SerialSessionWriter::start(&reservation, emit, writer, timings)?;
        let runtime_result = execute_loop_with_sink(
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
            Some(&mut serial_writer),
        );
        let finish_result = serial_writer.finish();
        let runtime = runtime_result?;
        finish_result?;
        reservation.mark_side_effects_applied();
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
            validate_session_log_text(
                &reservation.session_path,
                &expected_session_id,
                &stream,
            )?;
        } else if runtime.events != planned_events {
            return Err(RuntimeError::Protocol(format!(
                "{} committed runtime did not match its validated plan",
                reservation.session_path.display()
            )));
        }
        write_reserved_session_metadata(
            &reservation,
            &expected_session_id,
            runtime.events.len(),
            Some(&definition_hashes),
        )?;
        reservation.release_lock()?;
        if let Some(err) = runtime.terminal_error {
            return Err(err);
        }
        let status = if runtime_failed {
            format!("loop {} failed\n", loop_block.identity.id)
        } else {
            format!("loop {} completed\n", loop_block.identity.id)
        };
        serial_writer.publish_human_status(&status);
        Ok(RunOutput {
            event_count: runtime.events.len(),
            failed: runtime_failed,
            session_id: expected_session_id.clone(),
            session_path: reservation.session_path.clone(),
            stdout: String::new(),
        })
    })();
    if result.is_err() {
        reservation.rollback();
    }
    result
}
