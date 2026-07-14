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
    let mut initial = read_file_range(&path, 0, MAX_SESSION_LOG_BYTES)?;
    let complete_len = complete_jsonl_prefix_len(&initial);
    let mut pending = initial.split_off(complete_len);
    let stream = decode_jsonl_bytes(&path, initial)?;
    let mut validated_len = stream.len();
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
            validated_len,
        )?)
    };
    let mut observed_len = complete_len + pending.len();
    if !pending.is_empty() && (stream_is_failed(&events) || stream_is_completed(&events)) {
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
    drop(stream);

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
            if current_len < validated_len {
                return Err(RuntimeError::Protocol(format!(
                    "{} changed outside append-only tail semantics",
                    path.display()
                )));
            }
            // WHY: a failed append may roll back only bytes that have never formed a complete
            // event. The validated prefix remains immutable and authoritative.
            pending.truncate(current_len - validated_len);
            observed_len = current_len;
            continue;
        }
        if current_len == observed_len {
            continue;
        }
        let suffix = match read_tail_file_suffix(&path, observed_len, current_len)? {
            TailSuffixRead::Appended(suffix) => suffix,
            TailSuffixRead::RolledBack(actual_len) => {
                if actual_len < validated_len {
                    return Err(RuntimeError::Protocol(format!(
                        "{} changed outside append-only tail semantics",
                        path.display()
                    )));
                }
                pending.truncate(actual_len - validated_len);
                observed_len = actual_len;
                continue;
            }
        };
        observed_len = current_len;
        pending.extend_from_slice(&suffix);
        let complete_len = complete_jsonl_prefix_len(&pending);
        if complete_len == 0 {
            continue;
        }
        let remainder = pending.split_off(complete_len);
        let appended_bytes = std::mem::replace(&mut pending, remainder);
        let appended_len = appended_bytes.len();
        let appended = decode_jsonl_bytes(&path, appended_bytes)?;
        let appended_events = if let Some(state) = &mut append_state {
            state.validate_appended(&path, &appended)?
        } else {
            let appended_events = validate_session_log_text(&path, session_id, &appended)?;
            append_state = Some(SessionAppendValidationState::from_prior_events(
                &path,
                session_id,
                &appended_events,
                appended_len,
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
        validated_len += appended_len;
    }

    if emit == EmitMode::Human
        && !write_tail_bytes(
            writer,
            human_session_status(session_id, "tailed", &events).as_bytes(),
        )?
    {
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

fn complete_jsonl_prefix_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline_index| newline_index + 1)
}

fn decode_jsonl_bytes(path: &Path, bytes: Vec<u8>) -> Result<String, RuntimeError> {
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}
