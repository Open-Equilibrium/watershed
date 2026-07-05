/// Replays a persisted terminal or partial session log without modifying it.
pub fn replay_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    read_existing_session(workspace.as_ref(), session_id, emit)
}

/// Tails a session log and captures output in the returned [`RunOutput`].
pub fn tail_session(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let mut stdout = Vec::new();
    let mut output = tail_session_to_writer(workspace, session_id, emit, &mut stdout)?;
    output.stdout = String::from_utf8(stdout)
        .map_err(|err| RuntimeError::Protocol(format!("tail output was not valid UTF-8: {err}")))?;
    Ok(output)
}

/// Tails a session log to a caller-provided writer with default follow behavior.
pub fn tail_session_to_writer(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    writer: &mut impl Write,
) -> Result<RunOutput, RuntimeError> {
    tail_session_to_writer_with_options(workspace, session_id, emit, TailOptions::follow(), writer)
}

/// Tails a session log to a caller-provided writer with explicit options.
pub fn tail_session_to_writer_with_options(
    workspace: impl AsRef<Path>,
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
    writer: &mut impl Write,
) -> Result<RunOutput, RuntimeError> {
    let workspace = workspace.as_ref();
    let path = session_path(workspace, session_id)?;
    ensure_existing_session_log_path(workspace, &path)?;
    let initial = read_session_log_to_string(&path)?;
    let mut stream = complete_jsonl_prefix(&initial).to_owned();
    let mut events = if stream.is_empty() {
        Vec::new()
    } else {
        validate_session_log_text(&path, session_id, &stream)?
    };
    let mut append_state = if events.is_empty() {
        None
    } else {
        Some(SessionAppendValidationState::from_prior_events(
            &path,
            session_id,
            &events,
            stream.len(),
        )?)
    };
    let mut pending = initial[stream.len()..].to_owned();
    let mut observed_len = initial.len();
    if initial.len() > stream.len() && (stream_is_failed(&events) || stream_is_completed(&events)) {
        return Err(RuntimeError::Protocol(format!(
            "{} contains a partial line after a terminal event",
            path.display()
        )));
    }
    if (!stream.is_empty() || emit == EmitMode::Jsonl)
        && !write_tail_chunk(writer, emit, session_id, &stream)?
    {
        return Ok(RunOutput {
            event_count: events.len(),
            failed: stream_is_failed(&events),
            session_id: session_id.to_owned(),
            session_path: path,
            stdout: String::new(),
        });
    }

    let started = Instant::now();
    while !stream_is_failed(&events) && !stream_is_completed(&events) {
        if !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            break;
        }
        thread::sleep(tail_poll_interval(&options, started));
        let current_len = tail_session_log_len(&path)?;
        if current_len < observed_len {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside append-only tail semantics",
                path.display()
            )));
        }
        if current_len == observed_len {
            continue;
        }
        let suffix = read_tail_file_suffix_to_string(&path, observed_len, current_len)?;
        observed_len = current_len;
        pending.push_str(&suffix);
        if !pending.ends_with('\n') {
            continue;
        }
        let appended = std::mem::take(&mut pending);
        let appended_events = if let Some(state) = &mut append_state {
            state.validate_appended(&path, &appended)?
        } else {
            let appended_events = validate_session_log_text(&path, session_id, &appended)?;
            append_state = Some(SessionAppendValidationState::from_prior_events(
                &path,
                session_id,
                &appended_events,
                appended.len(),
            )?);
            appended_events
        };
        events.extend(appended_events);
        if !write_tail_chunk(writer, emit, session_id, &appended)? {
            return Ok(RunOutput {
                event_count: events.len(),
                failed: stream_is_failed(&events),
                session_id: session_id.to_owned(),
                session_path: path,
                stdout: String::new(),
            });
        }
        stream.push_str(&appended);
    }

    if emit == EmitMode::Human && !write_tail_chunk(writer, emit, session_id, "")? {
        return Ok(RunOutput {
            event_count: events.len(),
            failed: stream_is_failed(&events),
            session_id: session_id.to_owned(),
            session_path: path,
            stdout: String::new(),
        });
    }

    Ok(RunOutput {
        event_count: events.len(),
        failed: stream_is_failed(&events),
        session_id: session_id.to_owned(),
        session_path: path,
        stdout: String::new(),
    })
}

fn tail_poll_interval(options: &TailOptions, started: Instant) -> Duration {
    let default = Duration::from_millis(25);
    options.timeout.map_or(default, |timeout| {
        timeout.saturating_sub(started.elapsed()).min(default)
    })
}

fn complete_jsonl_prefix(text: &str) -> &str {
    text.rfind('\n')
        .map_or("", |newline_index| &text[..=newline_index])
}
