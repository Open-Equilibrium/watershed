/// Validates public v0 event JSONL canonical bytes, envelope fields, payload
/// contracts and session lifecycle ordering.
pub fn validate_protocol_jsonl_text(
    path: &Path,
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    let text_bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    if text_bytes > MAX_SESSION_EVENT_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} session event data size {text_bytes} bytes exceeds max {MAX_SESSION_EVENT_BYTES}",
            path.display()
        )));
    }
    if !text.ends_with('\n') {
        return Err(RuntimeError::Protocol(format!(
            "{} must end with LF",
            path.display()
        )));
    }

    let events = SessionAppendValidationState::unscoped().validate_appended(path, text)?;
    if events.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "{} must contain at least one event",
            path.display()
        )));
    }
    Ok(events)
}

fn validate_event_metadata(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<(), RuntimeError> {
    event.validate_metadata().map_err(|err| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} has invalid envelope metadata: {err}",
            path.display()
        ))
    })
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
    for (field, value) in &event.additional_fields {
        validator.reject_nulls(value, field)?;
    }

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
        EventType::FlowStarted | EventType::FlowCompleted => {
            validator.require_string("flow_definition_id")?;
            validator.optional_string("flow_name")?;
        }
        EventType::FlowFailed => {
            validator.require_string("flow_definition_id")?;
            validator.optional_string("flow_name")?;
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
                        return Err(
                            validator.error("payload connection arrays must have the same length")
                        );
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
                    return Err(
                        validator.error("payload connection arrays must be present together")
                    );
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
                return Err(
                    validator.error("payload.tool_kind must be predefined-command or own-script")
                );
            }
            validator.require_string_array("read_scope")?;
            validator.require_string_array("write_scope")?;
            validator.require_string_array("allowed_parameters")?;
            let network_access = validator.require_string("network_access")?;
            if !matches!(network_access, "deny" | "declared") {
                return Err(validator.error("payload.network_access must be deny or declared"));
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

fn null_location(value: &serde_json::Value, location: &str) -> Option<String> {
    match value {
        serde_json::Value::Null => Some(location.to_owned()),
        serde_json::Value::Array(values) => values
            .iter()
            .enumerate()
            .find_map(|(index, value)| null_location(value, &format!("{location}[{index}]"))),
        serde_json::Value::Object(values) => values
            .iter()
            .find_map(|(field, value)| null_location(value, &format!("{location}.{field}"))),
        _ => None,
    }
}

impl PayloadValidator<'_> {
    fn reject_nulls(&self, value: &serde_json::Value, location: &str) -> Result<(), RuntimeError> {
        null_location(value, location)
            .map(|location| self.error(&format!("{location} must not be null in protocol v0")))
            .map_or(Ok(()), Err)
    }

    fn require_string(&self, field: &str) -> Result<&str, RuntimeError> {
        match self.payload.get(field).and_then(serde_json::Value::as_str) {
            Some(value) if !value.is_empty() => Ok(value),
            _ => Err(self.error(&format!("payload.{field} must be a non-empty string"))),
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
                    self.error(&format!("payload.{field} must contain only strings"))
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

struct SessionAppendValidationState {
    expected_session_id: Option<String>,
    stream_session_id: Option<String>,
    previous_sequence: u64,
    event_ids: BTreeSet<String>,
    flow_started_ids: BTreeSet<String>,
    terminal_line: Option<usize>,
    stream_bytes: usize,
    line_count: usize,
    lifecycle: SessionLifecycleState,
}

impl SessionAppendValidationState {
    fn unscoped() -> Self {
        Self::new(None)
    }

    fn empty(expected_session_id: &str) -> Self {
        Self::new(Some(expected_session_id))
    }

    fn new(expected_session_id: Option<&str>) -> Self {
        Self {
            expected_session_id: expected_session_id.map(str::to_owned),
            stream_session_id: None,
            previous_sequence: 0,
            event_ids: BTreeSet::new(),
            flow_started_ids: BTreeSet::new(),
            terminal_line: None,
            stream_bytes: 0,
            line_count: 0,
            lifecycle: SessionLifecycleState::default(),
        }
    }

    fn tool_without_progress(&self) -> Option<&str> {
        self.lifecycle.tool_without_progress()
    }

    fn terminal_flow_ids(&self) -> BTreeSet<String> {
        self.lifecycle.flows.terminal.keys().cloned().collect()
    }

    #[cfg(test)]
    fn from_prior_events(
        path: &Path,
        expected_session_id: &str,
        prior_events: &[EventEnvelope],
    ) -> Result<Self, RuntimeError> {
        let mut state = Self::empty(expected_session_id);
        for event in prior_events {
            let canonical_bytes = event
                .canonical_jsonl()
                .map_err(|err| {
                    RuntimeError::Protocol(format!("{} prior event stream: {err}", path.display()))
                })?
                .len();
            state.validate_constructed_event(path, event, canonical_bytes)?;
        }
        Ok(state)
    }

    fn validate_appended(
        &mut self,
        path: &Path,
        text: &str,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let mut appended_events = Vec::new();
        self.validate_appended_with(path, text, |event| {
            appended_events.push(event.clone());
            Ok(())
        })?;
        Ok(appended_events)
    }

    fn validate_appended_with(
        &mut self,
        path: &Path,
        text: &str,
        mut visit: impl FnMut(&EventEnvelope) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        if text.is_empty() {
            return Ok(());
        }
        if !text.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} appended suffix must end with LF",
                path.display()
            )));
        }

        for line in text.split_terminator('\n') {
            let line_number = self.line_count + 1;
            let canonical_bytes = line.len().saturating_add(1);
            if line.ends_with('\r') {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use LF-only line endings",
                    path.display()
                )));
            }
            validate_event_size(path, line_number, canonical_bytes)?;
            let event = parse_canonical_event(path, line_number, line)?;
            self.validate_budget(path, line_number, canonical_bytes)?;
            self.validate_event(path, line_number, &event)?;
            visit(&event)?;
        }
        Ok(())
    }

    fn validate_constructed_event(
        &mut self,
        path: &Path,
        event: &EventEnvelope,
        canonical_bytes: usize,
    ) -> Result<(), RuntimeError> {
        let line_number = self.line_count + 1;
        validate_event_size(path, line_number, canonical_bytes)?;
        self.validate_budget(path, line_number, canonical_bytes)?;
        self.validate_event(path, line_number, event)
    }

    fn validate_budget(
        &mut self,
        path: &Path,
        line_number: usize,
        canonical_bytes: usize,
    ) -> Result<(), RuntimeError> {
        if u64::try_from(line_number).unwrap_or(u64::MAX) > MAX_FLOW_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "{} runtime event budget exceeded at line {line_number}: max {MAX_FLOW_EVENTS}",
                path.display()
            )));
        }
        let stream_bytes = self.stream_bytes.saturating_add(canonical_bytes);
        if u64::try_from(stream_bytes).unwrap_or(u64::MAX) > MAX_SESSION_EVENT_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} session event data budget exceeded at line {line_number}: max {MAX_SESSION_EVENT_BYTES} bytes",
                path.display()
            )));
        }
        self.stream_bytes = stream_bytes;
        Ok(())
    }

    fn validate_event(
        &mut self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<(), RuntimeError> {
        if self
            .expected_session_id
            .as_ref()
            .is_some_and(|expected| expected != &event.session_id)
        {
            return Err(RuntimeError::Protocol(format!(
                "{} must use one session_id",
                path.display()
            )));
        }
        validate_event_metadata(path, line_number, event)?;
        if line_number == 1 && event.sequence != 1 {
            return Err(RuntimeError::Protocol(format!(
                "{} first sequence must be 1",
                path.display()
            )));
        }
        if self.previous_sequence.checked_add(1) != Some(event.sequence) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} sequence must increase by exactly 1",
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
        if event.event_type == EventType::FlowStarted {
            let flow_id = event.flow_id.as_deref().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "{} line {line_number} flow.started must include flow_id",
                    path.display()
                ))
            })?;
            if self.flow_started_ids.contains(flow_id) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a unique flow_id for flow.started",
                    path.display()
                )));
            }
            if u64::try_from(self.flow_started_ids.len()).unwrap_or(u64::MAX)
                >= MAX_FLOW_INVOCATIONS
            {
                return Err(RuntimeError::Protocol(format!(
                    "{} flow invocation budget exceeded at line {line_number}: max {MAX_FLOW_INVOCATIONS}",
                    path.display()
                )));
            }
            self.flow_started_ids.insert(flow_id.to_owned());
        }
        match &self.stream_session_id {
            Some(existing) if existing != &event.session_id => {
                return Err(RuntimeError::Protocol(format!(
                    "{} must use one session_id",
                    path.display()
                )));
            }
            None => self.stream_session_id = Some(event.session_id.clone()),
            Some(_) => {}
        }
        if line_number == 1 && event.event_type != EventType::SessionStarted {
            return Err(RuntimeError::Protocol(format!(
                "{} line 1 must start with session.started",
                path.display()
            )));
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
            self.lifecycle
                .validate_terminal_session(path, Some(event))?;
        }
        Ok(())
    }
}

fn validate_event_size(
    path: &Path,
    line_number: usize,
    canonical_bytes: usize,
) -> Result<(), RuntimeError> {
    if canonical_bytes <= MAX_CANONICAL_EVENT_BYTES {
        return Ok(());
    }
    Err(RuntimeError::Protocol(format!(
        "{} canonical event at line {line_number} is {canonical_bytes} bytes; max {MAX_CANONICAL_EVENT_BYTES}",
        path.display()
    )))
}

fn parse_canonical_event(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<EventEnvelope, RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|err| {
        RuntimeError::Protocol(format!(
            "{} line {line_number}: invalid JSON: {err}",
            path.display()
        ))
    })?;
    let canonical = proto::canonical_json(&value).map_err(|err| {
        RuntimeError::Protocol(format!("{} line {line_number}: {err}", path.display()))
    })?;
    if canonical != line {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use canonical JSONL bytes",
            path.display()
        )));
    }
    if let Some(location) = null_location(&value, "event") {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} {location} must not be null in protocol v0",
            path.display()
        )));
    }
    serde_json::from_value(value).map_err(|err| {
        RuntimeError::Protocol(format!(
            "{} line {line_number}: invalid event: {err}",
            path.display()
        ))
    })
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

#[derive(Default)]
struct SessionLifecycleState {
    flows: LifecycleTracker<String>,
    flow_definition_ids: BTreeMap<String, String>,
    flow_parents: BTreeMap<String, Option<String>>,
    terminal_steps: BTreeMap<StepLifecycleKey, usize>,
    tools: LifecycleTracker<ToolLifecycleKey>,
    terminal_messages: BTreeMap<MessageLifecycleKey, usize>,
    active_message_roles: BTreeMap<MessageLifecycleKey, String>,
    active_phases: BTreeMap<String, String>,
    active_steps: BTreeMap<String, StepLifecycleKey>,
    tools_without_progress: BTreeSet<ToolLifecycleKey>,
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

        if event.event_type != EventType::FlowStarted
            && let Some(flow_id) = &event.flow_id
        {
            if !self.flows.is_started(flow_id) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} {} must follow flow.started for flow_id {flow_id:?}",
                    path.display(),
                    event.event_type.as_str()
                )));
            }
            if let Some(terminal_line) = self.flows.terminal_line(flow_id) {
                return Err(terminal_lifecycle_error(
                    path,
                    line_number,
                    event,
                    "flow",
                    flow_id,
                    terminal_line,
                ));
            }
        }
        validate_lifecycle_parent(path, line_number, event, &self.flows, &self.flow_parents)?;

        match event.event_type {
            EventType::FlowStarted => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                self.flow_definition_ids.insert(
                    flow_id.clone(),
                    lifecycle_payload_string(event, "flow_definition_id"),
                );
                self.flow_parents
                    .insert(flow_id.clone(), event.parent_flow_id.clone());
                self.flows.start(flow_id);
            }
            EventType::FlowCompleted | EventType::FlowFailed => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if !self.flows.is_started(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow flow.started for flow_id {flow_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                let flow_definition_id = lifecycle_payload_string(event, "flow_definition_id");
                if self.flow_definition_ids.get(&flow_id) != Some(&flow_definition_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} flow_definition_id must match flow.started for flow_id {flow_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                if let Some(step) = self.active_steps.get(&flow_id) {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                    ));
                }
                if let Some(child) = self.flows.active_keys().find(|child| {
                    self.flow_parents.get(*child).and_then(Option::as_deref)
                        == Some(flow_id.as_str())
                }) {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "child flow",
                        child,
                    ));
                }
                self.flows.finish(flow_id, line_number);
            }
            EventType::PhaseEntered => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} phase.entered requires no active step for flow_id {:?}; active step_id {:?}",
                        path.display(),
                        flow_id,
                        active_step.step_id
                    )));
                }
                self.active_phases
                    .insert(flow_id, lifecycle_payload_string(event, "phase_id"));
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
                if let Some(terminal_line) = self.terminal_steps.get(&step).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started requires no active step for flow_id {:?}; active step_id {:?}",
                        path.display(),
                        flow_id,
                        active_step.step_id
                    )));
                }
                self.active_steps.insert(flow_id, step.clone());
            }
            EventType::StepCompleted => {
                let step = lifecycle_step_key(event, &self.active_phases);
                if let Some(terminal_line) = self.terminal_steps.get(&step).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if self.active_steps.get(&flow_id) != Some(&step) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.completed must follow step.started for step_id {:?}",
                        path.display(),
                        step.step_id
                    )));
                }
                if let Some(tool) = self
                    .tools
                    .active_keys()
                    .find(|tool| tool.flow_id.as_deref() == Some(flow_id.as_str()))
                {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                    ));
                }
                if let Some(message) = self
                    .active_message_roles
                    .keys()
                    .find(|message| message.flow_id == flow_id)
                {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.message_id,
                    ));
                }
                self.active_steps.remove(&flow_id);
                self.terminal_steps.insert(step, line_number);
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
                if self.tools.is_started(&tool) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} duplicate active tool.started for tool_id {:?}",
                        path.display(),
                        tool.tool_id
                    )));
                }
                self.tools_without_progress.insert(tool.clone());
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
                    self.tools.finish(tool.clone(), line_number);
                }
                self.tools_without_progress.remove(&tool);
            }
            EventType::ToolFailed => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
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
                if !self.tools.is_started(&tool) && self.active_phases.contains_key(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} tool.failed must follow tool.started after phase.entered for flow_id {flow_id:?}",
                        path.display()
                    )));
                }
                self.tools_without_progress.remove(&tool);
                self.tools.finish(tool, line_number);
            }
            EventType::MessageDelta => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.terminal_messages.get(&message).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.message_id,
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
                            message.message_id
                        )));
                    }
                    Some(_) => {}
                    None => {
                        self.active_message_roles.insert(message, role);
                    }
                }
            }
            EventType::MessageCompleted => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.terminal_messages.get(&message).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.message_id,
                        terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                let Some(active_role) = self.active_message_roles.get(&message) else {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed must follow message.delta for message_id {:?}",
                        path.display(),
                        message.message_id
                    )));
                };
                if active_role != &role {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed role {:?} must match active role {:?} for message_id {:?}",
                        path.display(),
                        role,
                        active_role,
                        message.message_id
                    )));
                }
                self.active_message_roles.remove(&message);
                self.terminal_messages.insert(message, line_number);
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
        if let Some(flow_id) = self.flows.active_keys().next() {
            return Err(open_lifecycle_error(path, "flow", flow_id));
        }
        if let Some(step) = self.active_steps.values().next() {
            return Err(open_lifecycle_error(path, "step", &step.step_id));
        }
        if let Some(tool) = self.tools.active_keys().next() {
            return Err(open_lifecycle_error(path, "tool", &tool.tool_id));
        }
        if let Some(message) = self.active_message_roles.keys().next() {
            return Err(open_lifecycle_error(path, "message", &message.message_id));
        }
        Ok(())
    }

    fn tool_without_progress(&self) -> Option<&str> {
        self.tools_without_progress
            .iter()
            .next()
            .map(|tool| tool.tool_id.as_str())
    }
}

fn open_lifecycle_error(path: &Path, kind: &str, id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} terminal session has open {kind} {id:?}",
        path.display()
    ))
}

struct LifecycleTracker<K: Ord> {
    active: BTreeSet<K>,
    terminal: BTreeMap<K, usize>,
}

impl<K: Ord> Default for LifecycleTracker<K> {
    fn default() -> Self {
        Self {
            active: BTreeSet::new(),
            terminal: BTreeMap::new(),
        }
    }
}

impl<K: Ord> LifecycleTracker<K> {
    fn start(&mut self, key: K) {
        self.active.insert(key);
    }

    fn finish(&mut self, key: K, line_number: usize) {
        self.active.remove(&key);
        self.terminal.insert(key, line_number);
    }

    fn is_started(&self, key: &K) -> bool {
        self.active.contains(key) || self.terminal.contains_key(key)
    }

    fn terminal_line(&self, key: &K) -> Option<usize> {
        self.terminal.get(key).copied()
    }

    fn active_keys(&self) -> impl Iterator<Item = &K> {
        self.active.iter()
    }
}

fn open_child_lifecycle_error(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    child_kind: &str,
    child_id: &str,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} requires no active {child_kind} {child_id:?}",
        path.display(),
        event.event_type.as_str()
    ))
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

fn require_lifecycle_flow_id(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<String, RuntimeError> {
    event.flow_id.clone().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} must include flow_id",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

/// Ensures parent flow references are already started, still active, and
/// consistent with the parent recorded by flow.started.
fn validate_lifecycle_parent(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    flows: &LifecycleTracker<String>,
    flow_parents: &BTreeMap<String, Option<String>>,
) -> Result<(), RuntimeError> {
    if event.parent_flow_id.is_some() && event.flow_id.is_none() {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} parent_flow_id requires flow_id",
            path.display()
        )));
    }

    let Some(flow_id) = &event.flow_id else {
        return Ok(());
    };

    if let Some(parent_flow_id) = &event.parent_flow_id {
        if parent_flow_id == flow_id {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_flow_id must not match flow_id {flow_id:?}",
                path.display()
            )));
        }
        if !flows.is_started(parent_flow_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_flow_id {parent_flow_id:?} must reference an already started flow",
                path.display()
            )));
        }
        if let Some(terminal_line) = flows.terminal_line(parent_flow_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_flow_id {parent_flow_id:?} references terminal flow on line {terminal_line}",
                path.display()
            )));
        }
    }

    if let Some(expected_parent) = flow_parents.get(flow_id)
        && expected_parent != &event.parent_flow_id
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} parent_flow_id for flow_id {flow_id:?} must match flow.started",
            path.display()
        )));
    }

    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MessageLifecycleKey {
    flow_id: String,
    message_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StepLifecycleKey {
    flow_id: Option<String>,
    phase_id: Option<String>,
    step_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ToolLifecycleKey {
    flow_id: Option<String>,
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
    let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
    active_phases.get(&flow_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active phase for flow_id {flow_id:?}",
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
    let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
    active_steps.get(&flow_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active step for flow_id {flow_id:?}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

fn lifecycle_step_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
) -> StepLifecycleKey {
    let flow_id = event.flow_id.clone();
    let phase_id = event
        .payload
        .get("phase_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            flow_id
                .as_ref()
                .and_then(|flow_id| active_phases.get(flow_id))
                .cloned()
        });
    StepLifecycleKey {
        flow_id,
        phase_id,
        step_id: lifecycle_payload_string(event, "step_id"),
    }
}

fn lifecycle_tool_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
    active_steps: &BTreeMap<String, StepLifecycleKey>,
) -> ToolLifecycleKey {
    let flow_id = event.flow_id.clone();
    let active_step = flow_id
        .as_ref()
        .and_then(|flow_id| active_steps.get(flow_id));
    let phase_id = active_step
        .and_then(|step| step.phase_id.clone())
        .or_else(|| {
            flow_id
                .as_ref()
                .and_then(|flow_id| active_phases.get(flow_id))
                .cloned()
        });
    let step_id = active_step.map(|step| step.step_id.clone());
    ToolLifecycleKey {
        flow_id,
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
    Ok(MessageLifecycleKey {
        flow_id: require_lifecycle_flow_id(path, line_number, event)?,
        message_id: lifecycle_payload_string(event, "message_id"),
    })
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

#[cfg(test)]
fn stream_is_completed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionCompleted)
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
