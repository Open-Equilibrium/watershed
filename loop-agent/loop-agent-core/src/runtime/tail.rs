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
    let mut capture = TailCapture::new(emit);
    let mut poll_interval = TAIL_POLL_INITIAL;
    loop {
        let appended = reader.read_incremental_after(capture.cursor)?;
        if !appended.is_empty() {
            poll_interval = TAIL_POLL_INITIAL;
        }
        capture.extend(appended)?;
        if capture.failed
            || capture.completed
            || !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            capture.extend(reader.read_after(capture.cursor)?)?;
            break;
        }
        wait(tail_poll_interval(&options, started, poll_interval));
        poll_interval = poll_interval.saturating_mul(2).min(TAIL_POLL_MAX);
    }
    Ok(RunOutput {
        event_count: capture.event_count,
        failed: capture.failed,
        session_id: session_id.to_owned(),
        session_path: reader.path.diagnostic_path().to_owned(),
        stdout: match emit {
            EmitMode::Jsonl => capture.jsonl,
            EmitMode::Human => human_session_status_from_failure(
                session_id,
                "tailed",
                capture.human_failure().as_deref(),
            ),
        },
    })
}

struct TailCapture {
    completed: bool,
    cursor: u64,
    error_messages: BTreeMap<String, String>,
    event_count: usize,
    failed: bool,
    failure_reason: Option<String>,
    jsonl: String,
    retain_jsonl: bool,
}

impl TailCapture {
    fn new(emit: EmitMode) -> Self {
        Self {
            completed: false,
            cursor: 0,
            error_messages: BTreeMap::new(),
            event_count: 0,
            failed: false,
            failure_reason: None,
            jsonl: String::new(),
            retain_jsonl: emit == EmitMode::Jsonl,
        }
    }

    fn extend(&mut self, events: Vec<EventEnvelope>) -> Result<(), RuntimeError> {
        for event in events {
            if self.retain_jsonl {
                self.jsonl.push_str(&event.canonical_jsonl().map_err(|err| {
                    RuntimeError::Protocol(format!(
                        "failed to serialize validated tail event: {err}"
                    ))
                })?);
            }
            if event.event_type == EventType::Error
                && let (Some(code), Some(message)) = (
                    event
                        .payload
                        .get("code")
                        .and_then(serde_json::Value::as_str),
                    event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str),
                )
            {
                self.error_messages
                    .insert(code.to_owned(), message.to_owned());
            }
            if event.event_type == EventType::SessionFailed {
                self.failed = true;
                self.failure_reason = event
                    .payload
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
            self.completed |= event.event_type == EventType::SessionCompleted;
            self.cursor = event.sequence;
            self.event_count = self.event_count.saturating_add(1);
        }
        Ok(())
    }

    fn human_failure(&self) -> Option<String> {
        let reason = self.failure_reason.as_deref()?;
        Some(render_human_failure_status(
            reason,
            self.error_messages.get(reason).map(String::as_str),
        ))
    }
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
