const TAIL_POLL_INITIAL: Duration = Duration::from_millis(25);
const TAIL_POLL_MAX: Duration = Duration::from_secs(1);

/// Replays a persisted terminal or non-terminal session log without modifying it.
pub fn replay_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    read_existing_session(workspace.as_ref(), session_id, emit)
}

/// Follows an existing session log and captures its validated output.
pub fn tail_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    tail_session_with_options(workspace, session_id, emit, TailOptions::follow())
}

/// Reads an existing session log with explicit follow behavior and captures its validated output.
pub fn tail_session_with_options(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
) -> Result<RunOutput, RuntimeError> {
    tail_session_with_wait(workspace.as_ref(), session_id, emit, options, thread::sleep)
}

fn tail_session_with_wait(
    workspace: &Path,
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
    mut wait: impl FnMut(Duration),
) -> Result<RunOutput, RuntimeError> {
    let mut reader = SessionEventReader::open(workspace, session_id)?;
    let started = Instant::now();
    let mut events = Vec::new();
    let mut poll_interval = TAIL_POLL_INITIAL;
    loop {
        let cursor = events
            .last()
            .map_or(0, |event: &EventEnvelope| event.sequence);
        let appended = reader.read_after(cursor)?;
        if !appended.is_empty() {
            poll_interval = TAIL_POLL_INITIAL;
        }
        events.extend(appended);
        if stream_is_failed(&events)
            || stream_is_completed(&events)
            || !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            break;
        }
        wait(tail_poll_interval(&options, started, poll_interval));
        poll_interval = poll_interval.saturating_mul(2).min(TAIL_POLL_MAX);
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

fn tail_poll_interval(
    options: &TailOptions,
    started: Instant,
    poll_interval: Duration,
) -> Duration {
    options.timeout.map_or(poll_interval, |timeout| {
        timeout.saturating_sub(started.elapsed()).min(poll_interval)
    })
}
