/// Validates public v0 event JSONL canonical bytes, envelope fields, payload
/// contracts and session lifecycle ordering.
pub fn validate_protocol_jsonl_text(
    path: &Path,
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    if !text.ends_with('\n') {
        return Err(RuntimeError::Protocol(format!(
            "{} must end with LF",
            path.display()
        )));
    }

    let mut previous_sequence = 0;
    let mut session_id = None::<String>;
    let mut event_ids = BTreeSet::new();
    let mut loop_started_ids = BTreeSet::new();
    let mut terminal_line = None::<usize>;
    let mut events = Vec::new();
    let mut stream_bytes = 0usize;
    for (index, line) in text.split_terminator('\n').enumerate() {
        let line_number = index + 1;
        // WHY: count JSONL bytes and events before parsing payloads so oversized streams
        // fail cheaply and deterministically.
        if u64::try_from(line_number).unwrap_or(u64::MAX) > MAX_LOOP_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "{} runtime event budget exceeded at line {line_number}: max {MAX_LOOP_EVENTS}",
                path.display()
            )));
        }
        stream_bytes = stream_bytes
            .checked_add(line.len().saturating_add(1))
            .unwrap_or(usize::MAX);
        if stream_bytes > MAX_LOOP_EVENT_STREAM_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} event stream budget exceeded at line {line_number}: {stream_bytes} bytes exceeds max {MAX_LOOP_EVENT_STREAM_BYTES}",
                path.display()
            )));
        }
        if line.ends_with('\r') {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use LF-only line endings",
                path.display()
            )));
        }
        let event: EventEnvelope = serde_json::from_str(line)?;
        let canonical = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("{} line {line_number}: {err}", path.display()))
        })?;
        if canonical != format!("{line}\n") {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use canonical JSONL bytes",
                path.display()
            )));
        }
        validate_event_metadata(path, line_number, &event)?;
        if line_number == 1 && event.sequence != 1 {
            return Err(RuntimeError::Protocol(format!(
                "{} first sequence must be 1",
                path.display()
            )));
        }
        if event.sequence <= previous_sequence {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} sequence must increase",
                path.display()
            )));
        }
        previous_sequence = event.sequence;
        if !event_ids.insert(event.event_id.clone()) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a unique event_id",
                path.display()
            )));
        }
        if let Some(terminal_line) = terminal_line {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} appears after terminal session event on line {terminal_line}",
                path.display()
            )));
        }
        validate_event_payload(path, line_number, &event)?;
        if event.event_type == EventType::LoopStarted {
            let loop_id = event.loop_id.as_deref().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "{} line {line_number} loop.started must include loop_id",
                    path.display()
                ))
            })?;
            if !loop_started_ids.insert(loop_id.to_owned()) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a unique loop_id for loop.started",
                    path.display()
                )));
            }
        }
        match &session_id {
            Some(existing) if existing != &event.session_id => {
                return Err(RuntimeError::Protocol(format!(
                    "{} must use one session_id",
                    path.display()
                )));
            }
            None => session_id = Some(event.session_id.clone()),
            Some(_) => {}
        }
        if matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        ) {
            terminal_line = Some(line_number);
        }
        events.push(event);
    }
    if events.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "{} must contain at least one event",
            path.display()
        )));
    }
    validate_session_lifecycle(path, &events)?;
    Ok(events)
}

fn validate_event_metadata(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<(), RuntimeError> {
    if !validate_session_id(&event.session_id) {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use a valid session_id",
            path.display()
        )));
    }
    if event.event_id.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use a non-empty event_id",
            path.display()
        )));
    }
    if event.source.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use a non-empty source",
            path.display()
        )));
    }
    if !is_rfc3339_utc_timestamp(&event.timestamp) {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use an RFC3339 UTC timestamp",
            path.display()
        )));
    }
    if event
        .correlation_id
        .as_ref()
        .is_some_and(|correlation_id| correlation_id.is_empty())
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use a non-empty correlation_id",
            path.display()
        )));
    }
    if event
        .loop_id
        .as_ref()
        .is_some_and(|loop_id| loop_id.is_empty())
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use a non-empty loop_id",
            path.display()
        )));
    }
    if event
        .parent_loop_id
        .as_ref()
        .is_some_and(|parent_loop_id| parent_loop_id.is_empty())
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use a non-empty parent_loop_id",
            path.display()
        )));
    }
    Ok(())
}

fn validate_event_payload(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<(), RuntimeError> {
    let payload = event.payload.as_object().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} payload must be an object",
            path.display(),
            event.event_type.as_str()
        ))
    })?;
    let validator = PayloadValidator {
        path,
        line_number,
        event_type: event.event_type,
        payload,
    };
    validator.reject_nulls(&event.payload, "payload")?;

    match event.event_type {
        EventType::SessionStarted
        | EventType::SessionPaused
        | EventType::SessionResumed
        | EventType::SessionCompleted => {
            validator.optional_string("reason")?;
        }
        EventType::SessionFailed => {
            validator.require_string("reason")?;
        }
        EventType::LoopStarted | EventType::LoopCompleted => {
            validator.require_string("loop_definition_id")?;
            validator.optional_string("loop_name")?;
        }
        EventType::LoopFailed => {
            validator.require_string("loop_definition_id")?;
            validator.optional_string("loop_name")?;
            validator.require_string("error")?;
        }
        EventType::PhaseEntered => {
            validator.require_string("phase_id")?;
            validator.require_string("phase_name")?;
            validator.require_string_array("instruction_ids")?;
            validator.require_string_array("tool_ids")?;
        }
        EventType::StepStarted | EventType::StepCompleted => {
            validator.require_string("step_id")?;
            validator.require_string("step_name")?;
            validator.optional_string("phase_id")?;
            validator.optional_string("instruction_id")?;
            let connection_ids = validator.optional_string_array("connection_ids")?;
            let connection_kinds = validator.optional_string_array("connection_kinds")?;
            match (connection_ids, connection_kinds) {
                (Some(ids), Some(kinds)) => {
                    if ids.len() != kinds.len() {
                        return Err(validator.error(
                            "payload connection arrays must have the same length",
                        ));
                    }
                    for kind in kinds {
                        if !matches!(kind, "data" | "trigger" | "refresh") {
                            return Err(validator.error(
                                "payload.connection_kinds values must be data, trigger, or refresh",
                            ));
                        }
                    }
                }
                (None, None) => {}
                _ => {
                    return Err(validator.error(
                        "payload connection arrays must be present together",
                    ));
                }
            }
        }
        EventType::MessageDelta => {
            validator.require_string("message_id")?;
            validator.require_role()?;
            validator.require_string("content_delta")?;
        }
        EventType::MessageCompleted => {
            validator.require_string("message_id")?;
            validator.require_role()?;
        }
        EventType::ToolStarted => {
            validator.require_string("tool_id")?;
            validator.require_string("tool_name")?;
            let tool_kind = validator.require_string("tool_kind")?;
            if !matches!(tool_kind, "predefined-command" | "own-script") {
                return Err(validator.error(
                    "payload.tool_kind must be predefined-command or own-script",
                ));
            }
            validator.require_string_array("read_scope")?;
            validator.require_string_array("write_scope")?;
            validator.require_string_array("allowed_parameters")?;
            let network_access = validator.require_string("network_access")?;
            if !matches!(network_access, "deny" | "declared") {
                return Err(validator.error(
                    "payload.network_access must be deny or declared",
                ));
            }
        }
        EventType::ToolProgress => {
            validator.require_string("tool_id")?;
            validator.require_string("message")?;
        }
        EventType::ToolCompleted => {
            validator.require_string("tool_id")?;
            validator.optional_integer("exit_code")?;
        }
        EventType::ToolFailed | EventType::ToolTimedOut => {
            validator.require_string("tool_id")?;
            validator.require_string("error")?;
        }
        EventType::ArtifactLogged => {
            validator.require_string("artifact_id")?;
            validator.require_string("artifact_type")?;
            validator.require_string("uri")?;
        }
        EventType::AttentionRequested => {
            validator.require_string("request_id")?;
            validator.require_string("reason")?;
        }
        EventType::MetricSample => {
            validator.require_string("metric_name")?;
            validator.require_number("value")?;
        }
        EventType::Error => {
            validator.require_string("code")?;
            validator.require_string("message")?;
            validator.optional_object("data")?;
        }
    }

    Ok(())
}

struct PayloadValidator<'a> {
    path: &'a Path,
    line_number: usize,
    event_type: EventType,
    payload: &'a serde_json::Map<String, serde_json::Value>,
}

impl PayloadValidator<'_> {
    fn reject_nulls(
        &self,
        value: &serde_json::Value,
        location: &str,
    ) -> Result<(), RuntimeError> {
        match value {
            serde_json::Value::Null => {
                Err(self.error(&format!("{location} must not be null in protocol v0")))
            }
            serde_json::Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    self.reject_nulls(value, &format!("{location}[{index}]"))?;
                }
                Ok(())
            }
            serde_json::Value::Object(values) => {
                for (field, value) in values {
                    self.reject_nulls(value, &format!("{location}.{field}"))?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn require_string(&self, field: &str) -> Result<&str, RuntimeError> {
        match self
            .payload
            .get(field)
            .and_then(serde_json::Value::as_str)
        {
            Some(value) if !value.is_empty() => Ok(value),
            _ => Err(self.error(&format!(
                "payload.{field} must be a non-empty string"
            ))),
        }
    }

    fn optional_string(&self, field: &str) -> Result<(), RuntimeError> {
        if self.payload.contains_key(field) {
            self.require_string(field)?;
        }
        Ok(())
    }

    fn require_role(&self) -> Result<(), RuntimeError> {
        let role = self.require_string("role")?;
        if matches!(role, "system" | "user" | "assistant" | "tool") {
            Ok(())
        } else {
            Err(self.error("payload.role must be system, user, assistant, or tool"))
        }
    }

    fn require_string_array(&self, field: &str) -> Result<Vec<&str>, RuntimeError> {
        let Some(value) = self.payload.get(field) else {
            return Err(self.error(&format!("payload.{field} must be a string array")));
        };
        self.string_array(field, value)
    }

    fn optional_string_array(&self, field: &str) -> Result<Option<Vec<&str>>, RuntimeError> {
        self.payload
            .get(field)
            .map(|value| self.string_array(field, value))
            .transpose()
    }

    fn string_array<'a>(
        &self,
        field: &str,
        value: &'a serde_json::Value,
    ) -> Result<Vec<&'a str>, RuntimeError> {
        let Some(values) = value.as_array() else {
            return Err(self.error(&format!("payload.{field} must be a string array")));
        };
        values
            .iter()
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    self.error(&format!(
                        "payload.{field} must contain only strings"
                    ))
                })
            })
            .collect()
    }

    fn optional_integer(&self, field: &str) -> Result<(), RuntimeError> {
        if let Some(value) = self.payload.get(field) {
            let Some(number) = value.as_number() else {
                return Err(self.error(&format!("payload.{field} must be an integer")));
            };
            if number.as_i64().is_none() && number.as_u64().is_none() {
                return Err(self.error(&format!("payload.{field} must be an integer")));
            }
        }
        Ok(())
    }

    fn require_number(&self, field: &str) -> Result<(), RuntimeError> {
        if self
            .payload
            .get(field)
            .is_some_and(serde_json::Value::is_number)
        {
            Ok(())
        } else {
            Err(self.error(&format!("payload.{field} must be a number")))
        }
    }

    fn optional_object(&self, field: &str) -> Result<(), RuntimeError> {
        if self
            .payload
            .get(field)
            .is_some_and(|value| !value.is_object())
        {
            Err(self.error(&format!("payload.{field} must be an object")))
        } else {
            Ok(())
        }
    }

    fn error(&self, message: &str) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "{} line {} {} {message}",
            self.path.display(),
            self.line_number,
            self.event_type.as_str()
        ))
    }
}

#[cfg(any(test, doctest))]
fn validate_appended_session_log_text(
    path: &Path,
    expected_session_id: &str,
    prior_events: &[EventEnvelope],
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    if prior_events.is_empty() {
        return validate_session_log_text(path, expected_session_id, text);
    }
    let mut stream_bytes = 0usize;
    for event in prior_events {
        let canonical = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("{} prior event stream: {err}", path.display()))
        })?;
        stream_bytes = stream_bytes
            .checked_add(canonical.len())
            .unwrap_or(usize::MAX);
    }
    let mut state = SessionAppendValidationState::from_prior_events(
        path,
        expected_session_id,
        prior_events,
        stream_bytes,
    )?;
    state.validate_appended(path, text)
}

struct SessionAppendValidationState {
    expected_session_id: String,
    previous_sequence: u64,
    event_ids: BTreeSet<String>,
    loop_started_ids: BTreeSet<String>,
    terminal_line: Option<usize>,
    stream_bytes: usize,
    line_count: usize,
    lifecycle: SessionLifecycleState,
}

impl SessionAppendValidationState {
    fn empty(expected_session_id: &str) -> Self {
        Self {
            expected_session_id: expected_session_id.to_owned(),
            previous_sequence: 0,
            event_ids: BTreeSet::new(),
            loop_started_ids: BTreeSet::new(),
            terminal_line: None,
            stream_bytes: 0,
            line_count: 0,
            lifecycle: SessionLifecycleState::default(),
        }
    }

    fn from_prior_events(
        path: &Path,
        expected_session_id: &str,
        prior_events: &[EventEnvelope],
        stream_bytes: usize,
    ) -> Result<Self, RuntimeError> {
        let prior_session_id = &prior_events
            .first()
            .expect("prior events are non-empty")
            .session_id;
        if prior_events
            .first()
            .expect("prior events are non-empty")
            .event_type
            != EventType::SessionStarted
        {
            return Err(RuntimeError::Protocol(format!(
                "{} line 1 must start with session.started",
                path.display()
            )));
        }
        if prior_session_id != expected_session_id {
            return Err(RuntimeError::Protocol(format!(
                "{} contains session_id {prior_session_id:?}, expected {expected_session_id:?}",
                path.display()
            )));
        }

        let mut lifecycle = SessionLifecycleState::default();
        for (index, event) in prior_events.iter().enumerate() {
            lifecycle.validate_event(path, index + 1, event)?;
        }
        lifecycle.validate_terminal_session(path, prior_events.last())?;

        let terminal_line = prior_events
            .iter()
            .position(|event| {
                matches!(
                    event.event_type,
                    EventType::SessionCompleted | EventType::SessionFailed
                )
            })
            .map(|index| index + 1);

        Ok(Self {
            expected_session_id: expected_session_id.to_owned(),
            previous_sequence: prior_events
                .last()
                .expect("prior events are non-empty")
                .sequence,
            event_ids: prior_events
                .iter()
                .map(|event| event.event_id.clone())
                .collect(),
            loop_started_ids: prior_events
                .iter()
                .filter(|event| event.event_type == EventType::LoopStarted)
                .filter_map(|event| event.loop_id.clone())
                .collect(),
            terminal_line,
            stream_bytes,
            line_count: prior_events.len(),
            lifecycle,
        })
    }

    fn validate_appended(
        &mut self,
        path: &Path,
        text: &str,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        if !text.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} appended suffix must end with LF",
                path.display()
            )));
        }

        let mut appended_events = Vec::new();
        for line in text.split_terminator('\n') {
            let line_number = self.line_count + 1;
            if line.ends_with('\r') {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use LF-only line endings",
                    path.display()
                )));
            }
            let event: EventEnvelope = serde_json::from_str(line)?;
            let canonical = event.canonical_jsonl().map_err(|err| {
                RuntimeError::Protocol(format!("{} line {line_number}: {err}", path.display()))
            })?;
            if canonical != format!("{line}\n") {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use canonical JSONL bytes",
                    path.display()
                )));
            }
            self.validate_constructed_event(path, &event, line.len().saturating_add(1))?;
            appended_events.push(event);
        }
        Ok(appended_events)
    }

    fn validate_constructed_event(
        &mut self,
        path: &Path,
        event: &EventEnvelope,
        canonical_bytes: usize,
    ) -> Result<(), RuntimeError> {
        let line_number = self.line_count + 1;
        // WHY: incremental tail validation and live commits preserve the same cumulative
        // public stream budgets as full replay validation.
        if u64::try_from(line_number).unwrap_or(u64::MAX) > MAX_LOOP_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "{} runtime event budget exceeded at line {line_number}: max {MAX_LOOP_EVENTS}",
                path.display()
            )));
        }
        self.stream_bytes = self
            .stream_bytes
            .checked_add(canonical_bytes)
            .unwrap_or(usize::MAX);
        if self.stream_bytes > MAX_LOOP_EVENT_STREAM_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} event stream budget exceeded at line {line_number}: {} bytes exceeds max {MAX_LOOP_EVENT_STREAM_BYTES}",
                path.display(),
                self.stream_bytes
            )));
        }
        if event.session_id != self.expected_session_id {
            return Err(RuntimeError::Protocol(format!(
                "{} must use one session_id",
                path.display()
            )));
        }
        validate_event_metadata(path, line_number, event)?;
        if line_number == 1 {
            if event.event_type != EventType::SessionStarted {
                return Err(RuntimeError::Protocol(format!(
                    "{} line 1 must start with session.started",
                    path.display()
                )));
            }
            if event.sequence != 1 {
                return Err(RuntimeError::Protocol(format!(
                    "{} first sequence must be 1",
                    path.display()
                )));
            }
        }
        if event.sequence <= self.previous_sequence {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} sequence must increase",
                path.display()
            )));
        }
        self.previous_sequence = event.sequence;
        if !self.event_ids.insert(event.event_id.clone()) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a unique event_id",
                path.display()
            )));
        }
        if let Some(terminal_line) = self.terminal_line {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} appears after terminal session event on line {terminal_line}",
                path.display()
            )));
        }
        validate_event_payload(path, line_number, event)?;
        if event.event_type == EventType::LoopStarted {
            let loop_id = event.loop_id.as_deref().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "{} line {line_number} loop.started must include loop_id",
                    path.display()
                ))
            })?;
            if !self.loop_started_ids.insert(loop_id.to_owned()) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a unique loop_id for loop.started",
                    path.display()
                )));
            }
        }
        self.lifecycle.validate_event(path, line_number, event)?;
        if matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        ) {
            self.terminal_line = Some(line_number);
        }
        self.line_count = line_number;
        if matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        ) {
            self.lifecycle.validate_terminal_session(path, Some(event))?;
        }
        Ok(())
    }
}

fn validate_session_log_text(
    path: &Path,
    expected_session_id: &str,
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    let events = validate_protocol_jsonl_text(path, text)?;
    let actual_session_id = &events
        .first()
        .expect("validated streams contain at least one event")
        .session_id;
    if actual_session_id != expected_session_id {
        return Err(RuntimeError::Protocol(format!(
            "{} contains session_id {actual_session_id:?}, expected {expected_session_id:?}",
            path.display()
        )));
    }
    Ok(events)
}

/// Validates lifecycle invariants after envelope and payload validation:
/// every loop/step/tool/message must start before use, terminal lifecycle
/// items cannot receive later events, and terminal sessions cannot leave open
/// lifecycle items.
fn validate_session_lifecycle(path: &Path, events: &[EventEnvelope]) -> Result<(), RuntimeError> {
    if events
        .first()
        .expect("validated streams contain at least one event")
        .event_type
        != EventType::SessionStarted
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line 1 must start with session.started",
            path.display()
        )));
    }

    let mut state = SessionLifecycleState::default();

    for (index, event) in events.iter().enumerate() {
        let line_number = index + 1;
        state.validate_event(path, line_number, event)?;
    }

    state.validate_terminal_session(path, events.last())?;
    Ok(())
}

#[derive(Default)]
struct SessionLifecycleState {
    loops: LifecycleTracker<String>,
    loop_parents: BTreeMap<String, Option<String>>,
    steps: LifecycleTracker<StepLifecycleKey>,
    tools: LifecycleTracker<ToolLifecycleKey>,
    messages: LifecycleTracker<MessageLifecycleKey>,
    active_message_roles: BTreeMap<MessageLifecycleKey, String>,
    active_phases: BTreeMap<String, String>,
    active_steps: BTreeMap<String, StepLifecycleKey>,
}

impl SessionLifecycleState {
    fn validate_event(
        &mut self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<(), RuntimeError> {
        if line_number > 1 && event.event_type == EventType::SessionStarted {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} session.started is only valid as the first event",
                path.display()
            )));
        }

        if event.event_type != EventType::LoopStarted {
            if let Some(loop_id) = &event.loop_id {
                if !self.loops.is_started(loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow loop.started for loop_id {loop_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                if let Some(terminal_line) = self.loops.terminal_line(loop_id) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "loop",
                        loop_id,
                        terminal_line,
                    ));
                }
            }
        }
        validate_lifecycle_parent(path, line_number, event, &self.loops, &self.loop_parents)?;

        match event.event_type {
            EventType::LoopStarted => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                self.loop_parents
                    .insert(loop_id.clone(), event.parent_loop_id.clone());
                self.loops.start(loop_id);
            }
            EventType::LoopCompleted | EventType::LoopFailed => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if !self.loops.is_started(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow loop.started for loop_id {loop_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                self.loops.finish(loop_id, line_number);
            }
            EventType::PhaseEntered => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} phase.entered requires no active step for loop_id {:?}; active step_id {:?}",
                        path.display(),
                        loop_id,
                        active_step.step_id
                    )));
                }
                self.active_phases
                    .insert(loop_id, lifecycle_payload_string(event, "phase_id"));
            }
            EventType::StepStarted => {
                let active_phase =
                    require_active_phase(path, line_number, event, &self.active_phases)?;
                let step = lifecycle_step_key(event, &self.active_phases);
                if step.phase_id.as_deref() != Some(active_phase.as_str()) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started phase_id {:?} must match active phase {:?}",
                        path.display(),
                        step.phase_id,
                        active_phase
                    )));
                }
                if let Some(terminal_line) = self.steps.terminal_line(&step) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started requires no active step for loop_id {:?}; active step_id {:?}",
                        path.display(),
                        loop_id,
                        active_step.step_id
                    )));
                }
                self.active_steps.insert(loop_id, step.clone());
                self.steps.start(step);
            }
            EventType::StepCompleted => {
                let step = lifecycle_step_key(event, &self.active_phases);
                if let Some(terminal_line) = self.steps.terminal_line(&step) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                if !self.steps.is_started(&step) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.completed must follow step.started for step_id {:?}",
                        path.display(),
                        step.step_id
                    )));
                }
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                match self.active_steps.get(&loop_id) {
                    Some(active_step) if active_step == &step => {}
                    Some(active_step) => {
                        return Err(RuntimeError::Protocol(format!(
                            "{} line {line_number} step.completed requires active step_id {:?}, found {:?}",
                            path.display(),
                            step.step_id,
                            active_step.step_id
                        )));
                    }
                    None => {
                        return Err(RuntimeError::Protocol(format!(
                            "{} line {line_number} step.completed requires active step for step_id {:?}",
                            path.display(),
                            step.step_id
                        )));
                    }
                }
                self.active_steps.remove(&loop_id);
                self.steps.finish(step, line_number);
            }
            EventType::ToolStarted => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                self.tools.start(tool);
            }
            EventType::ToolProgress | EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                if !self.tools.is_started(&tool) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow tool.started for tool_id {:?}",
                        path.display(),
                        event.event_type.as_str(),
                        tool.tool_id
                    )));
                }
                if matches!(
                    event.event_type,
                    EventType::ToolCompleted | EventType::ToolTimedOut
                ) {
                    self.tools.finish(tool, line_number);
                }
            }
            EventType::ToolFailed => {
                let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                if !self.tools.is_started(&tool) && self.active_phases.contains_key(&loop_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} tool.failed must follow tool.started after phase.entered for loop_id {loop_id:?}",
                        path.display()
                    )));
                }
                self.tools.finish(tool, line_number);
            }
            EventType::MessageDelta => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.messages.terminal_line(&message) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.1,
                        terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                match self.active_message_roles.get(&message) {
                    Some(active_role) if active_role != &role => {
                        return Err(RuntimeError::Protocol(format!(
                            "{} line {line_number} message.delta role {:?} must match active role {:?} for message_id {:?}",
                            path.display(),
                            role,
                            active_role,
                            message.1
                        )));
                    }
                    Some(_) => {}
                    None => {
                        self.messages.start(message.clone());
                        self.active_message_roles.insert(message, role);
                    }
                }
            }
            EventType::MessageCompleted => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.messages.terminal_line(&message) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.1,
                        terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                let Some(active_role) = self.active_message_roles.get(&message) else {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed must follow message.delta for message_id {:?}",
                        path.display(),
                        message.1
                    )));
                };
                if active_role != &role {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed role {:?} must match active role {:?} for message_id {:?}",
                        path.display(),
                        role,
                        active_role,
                        message.1
                    )));
                }
                self.messages.finish(message, line_number);
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
        Ok(())
    }

    fn validate_terminal_session(
        &self,
        path: &Path,
        last_event: Option<&EventEnvelope>,
    ) -> Result<(), RuntimeError> {
        if !last_event.is_some_and(|event| {
            matches!(
                event.event_type,
                EventType::SessionCompleted | EventType::SessionFailed
            )
        }) {
            return Ok(());
        }
        for loop_id in self.loops.started_keys() {
            if !self.loops.is_terminal(loop_id) {
                return Err(open_lifecycle_error(path, "loop", loop_id));
            }
        }
        for step in self.steps.started_keys() {
            if !self.steps.is_terminal(step) {
                return Err(open_lifecycle_error(path, "step", &step.step_id));
            }
        }
        for tool in self.tools.started_keys() {
            if !self.tools.is_terminal(tool) {
                return Err(open_lifecycle_error(path, "tool", &tool.tool_id));
            }
        }
        for message in self.messages.started_keys() {
            if !self.messages.is_terminal(message) {
                return Err(open_lifecycle_error(path, "message", &message.1));
            }
        }
        Ok(())
    }
}

fn open_lifecycle_error(path: &Path, kind: &str, id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} terminal session has open {kind} {id:?}",
        path.display()
    ))
}

struct LifecycleTracker<K: Ord> {
    started: BTreeSet<K>,
    terminal: BTreeMap<K, usize>,
}

impl<K: Ord> Default for LifecycleTracker<K> {
    fn default() -> Self {
        Self {
            started: BTreeSet::new(),
            terminal: BTreeMap::new(),
        }
    }
}

impl<K: Ord> LifecycleTracker<K> {
    fn start(&mut self, key: K) {
        self.started.insert(key);
    }

    fn finish(&mut self, key: K, line_number: usize) {
        self.terminal.insert(key, line_number);
    }

    fn is_started(&self, key: &K) -> bool {
        self.started.contains(key)
    }

    fn is_terminal(&self, key: &K) -> bool {
        self.terminal.contains_key(key)
    }

    fn terminal_line(&self, key: &K) -> Option<usize> {
        self.terminal.get(key).copied()
    }

    fn started_keys(&self) -> impl Iterator<Item = &K> {
        self.started.iter()
    }
}

fn started_tool_without_progress(events: &[EventEnvelope]) -> Option<String> {
    let mut active_phases = BTreeMap::new();
    let mut active_steps = BTreeMap::new();
    let mut started_without_progress = BTreeMap::new();

    for event in events {
        match event.event_type {
            EventType::PhaseEntered => {
                if let Some(loop_id) = &event.loop_id {
                    active_phases
                        .insert(loop_id.clone(), lifecycle_payload_string(event, "phase_id"));
                    active_steps.remove(loop_id);
                }
            }
            EventType::StepStarted => {
                if let Some(loop_id) = &event.loop_id {
                    active_steps.insert(loop_id.clone(), lifecycle_step_key(event, &active_phases));
                }
            }
            EventType::StepCompleted => {
                if let Some(loop_id) = &event.loop_id {
                    active_steps.remove(loop_id);
                }
            }
            EventType::ToolStarted => {
                let tool = lifecycle_tool_key(event, &active_phases, &active_steps);
                started_without_progress.insert(tool.clone(), tool.tool_id);
            }
            EventType::ToolProgress
            | EventType::ToolCompleted
            | EventType::ToolFailed
            | EventType::ToolTimedOut => {
                let tool = lifecycle_tool_key(event, &active_phases, &active_steps);
                started_without_progress.remove(&tool);
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::LoopStarted
            | EventType::LoopCompleted
            | EventType::LoopFailed
            | EventType::MessageDelta
            | EventType::MessageCompleted
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
    }

    started_without_progress.into_values().next()
}

fn terminal_lifecycle_error(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    kind: &str,
    id: &str,
    terminal_line: usize,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} appears after terminal {kind} {id:?} on line {terminal_line}",
        path.display(),
        event.event_type.as_str()
    ))
}

fn require_lifecycle_loop_id(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<String, RuntimeError> {
    event.loop_id.clone().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} must include loop_id",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

/// Ensures parent loop references are already started, still active, and
/// consistent with the parent recorded by loop.started.
fn validate_lifecycle_parent(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    loops: &LifecycleTracker<String>,
    loop_parents: &BTreeMap<String, Option<String>>,
) -> Result<(), RuntimeError> {
    if event.parent_loop_id.is_some() && event.loop_id.is_none() {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} parent_loop_id requires loop_id",
            path.display()
        )));
    }

    let Some(loop_id) = &event.loop_id else {
        return Ok(());
    };

    if let Some(parent_loop_id) = &event.parent_loop_id {
        if parent_loop_id == loop_id {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id must not match loop_id {loop_id:?}",
                path.display()
            )));
        }
        if !loops.is_started(parent_loop_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id {parent_loop_id:?} must reference an already started loop",
                path.display()
            )));
        }
        if let Some(terminal_line) = loops.terminal_line(parent_loop_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id {parent_loop_id:?} references terminal loop on line {terminal_line}",
                path.display()
            )));
        }
    }

    if let Some(expected_parent) = loop_parents.get(loop_id) {
        if expected_parent != &event.parent_loop_id {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_loop_id for loop_id {loop_id:?} must match loop.started",
                path.display()
            )));
        }
    }

    Ok(())
}

type MessageLifecycleKey = (String, String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StepLifecycleKey {
    loop_id: Option<String>,
    phase_id: Option<String>,
    step_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ToolLifecycleKey {
    loop_id: Option<String>,
    phase_id: Option<String>,
    step_id: Option<String>,
    tool_id: String,
}

fn require_active_phase(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
) -> Result<String, RuntimeError> {
    let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
    active_phases.get(&loop_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active phase for loop_id {loop_id:?}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

fn require_active_step(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    active_steps: &BTreeMap<String, StepLifecycleKey>,
) -> Result<StepLifecycleKey, RuntimeError> {
    let loop_id = require_lifecycle_loop_id(path, line_number, event)?;
    active_steps.get(&loop_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active step for loop_id {loop_id:?}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

fn lifecycle_step_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
) -> StepLifecycleKey {
    let loop_id = event.loop_id.clone();
    let phase_id = event
        .payload
        .get("phase_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            loop_id
                .as_ref()
                .and_then(|loop_id| active_phases.get(loop_id))
                .cloned()
        });
    StepLifecycleKey {
        loop_id,
        phase_id,
        step_id: lifecycle_payload_string(event, "step_id"),
    }
}

fn lifecycle_tool_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
    active_steps: &BTreeMap<String, StepLifecycleKey>,
) -> ToolLifecycleKey {
    let loop_id = event.loop_id.clone();
    let active_step = loop_id
        .as_ref()
        .and_then(|loop_id| active_steps.get(loop_id));
    let phase_id = active_step
        .and_then(|step| step.phase_id.clone())
        .or_else(|| {
            loop_id
                .as_ref()
                .and_then(|loop_id| active_phases.get(loop_id))
                .cloned()
        });
    let step_id = active_step.map(|step| step.step_id.clone());
    ToolLifecycleKey {
        loop_id,
        phase_id,
        step_id,
        tool_id: lifecycle_payload_string(event, "tool_id"),
    }
}

fn lifecycle_message_key(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<MessageLifecycleKey, RuntimeError> {
    Ok((
        require_lifecycle_loop_id(path, line_number, event)?,
        lifecycle_payload_string(event, "message_id"),
    ))
}

fn lifecycle_payload_string(event: &EventEnvelope, field: &str) -> String {
    event
        .payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .expect("payload contract validation ensures lifecycle key fields are strings")
        .to_owned()
}

fn stream_is_failed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionFailed)
}

fn stream_is_completed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionCompleted)
}

fn next_event_id(sequence: u64, events: &[EventEnvelope]) -> String {
    let existing = events
        .iter()
        .map(|event| event.event_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut candidate_sequence = sequence;
    loop {
        let candidate = format!("evt-{candidate_sequence:03}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
        candidate_sequence += 1;
    }
}

fn is_rfc3339_utc_timestamp(value: &str) -> bool {
    parse_rfc3339_utc_timestamp(value).is_some()
}

fn parse_rfc3339_utc_timestamp(value: &str) -> Option<i64> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;

    let mut date_parts = date.split('-');
    let year = date_parts.next().and_then(|part| parse_digits(part, 4))?;
    let month = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    let day = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let minute = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let second_part = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }

    let (second, fraction) = second_part
        .split_once('.')
        .map_or((second_part, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let second = parse_digits(second, 2)?;
    if fraction
        .is_some_and(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }

    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    Some(
        days.saturating_mul(86_400)
            .saturating_add(i64::from(hour) * 3_600)
            .saturating_add(i64::from(minute) * 60)
            .saturating_add(i64::from(second)),
    )
}

fn format_unix_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn parse_digits(value: &str, len: usize) -> Option<u16> {
    if value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
