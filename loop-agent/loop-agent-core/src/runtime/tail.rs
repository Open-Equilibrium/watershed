/// Replays a persisted terminal or partial session log without modifying it.
pub fn replay_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    read_existing_session(workspace.as_ref(), session_id, emit)
}

/// Waits for a session log and captures its validated output.
pub fn tail_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    tail_session_with_options(workspace, session_id, emit, TailOptions::follow())
}

/// Waits for a session log with explicit follow behavior and captures its validated output.
pub fn tail_session_with_options(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
) -> Result<RunOutput, RuntimeError> {
    let mut reader = SessionEventReader::open(workspace, session_id)?;
    let started = Instant::now();
    let mut events = Vec::new();
    loop {
        let cursor = events.last().map_or(0, |event: &EventEnvelope| event.sequence);
        events.extend(reader.read_after(cursor)?);
        if stream_is_failed(&events)
            || stream_is_completed(&events)
            || !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            break;
        }
        thread::sleep(tail_poll_interval(&options, started));
    }
    Ok(RunOutput {
        event_count: events.len(),
        failed: stream_is_failed(&events),
        session_id: session_id.to_owned(),
        session_path: reader.path,
        stdout: match emit {
            EmitMode::Jsonl => canonical_event_stream(&events)?,
            EmitMode::Human => human_session_status(session_id, "tailed", &events),
        },
    })
}

fn tail_poll_interval(options: &TailOptions, started: Instant) -> Duration {
    let default = Duration::from_millis(25);
    options.timeout.map_or(default, |timeout| {
        timeout.saturating_sub(started.elapsed()).min(default)
    })
}
