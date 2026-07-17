use sha2::{Digest, Sha256};

const CONTEXT_PROFILE_ID: &str = "loop-context-v0";
const CONTEXT_PROFILE_VERSION: &str = "0";
const CONTEXT_ESTIMATOR_ID: &str = "utf8-byte-v0";
const CONTEXT_ESTIMATOR_VERSION: &str = "0";
const CACHE_STABLE_TIER_ZERO_SOURCES: usize = 5;
const STUB_MODEL_CONTEXT_LIMIT: usize = 128 * 1024;
const STUB_MODEL_OUTPUT_RESERVE: usize = 8 * 1024;
const STUB_MODEL_SAFETY_MARGIN: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContextModelProfile {
    context_limit: usize,
    id: &'static str,
    output_reserve: usize,
    safety_margin: usize,
}

impl ContextModelProfile {
    fn stub_v0() -> Self {
        Self {
            context_limit: STUB_MODEL_CONTEXT_LIMIT,
            id: "stub-model-v0",
            output_reserve: STUB_MODEL_OUTPUT_RESERVE,
            safety_margin: STUB_MODEL_SAFETY_MARGIN,
        }
    }

    fn input_budget_tokens(self) -> Result<usize, RuntimeError> {
        self.context_limit
            .checked_sub(self.output_reserve)
            .and_then(|remaining| remaining.checked_sub(self.safety_margin))
            .ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "model profile {} reserves more tokens than its context limit",
                    self.id
                ))
            })
    }
}

struct ContextSource {
    source_id: String,
    content: serde_json::Value,
}

fn context_source(source_id: impl Into<String>, content: serde_json::Value) -> ContextSource {
    ContextSource {
        source_id: source_id.into(),
        content,
    }
}

#[derive(Default)]
struct ContextOmissionCounts {
    recent_complete_interaction: usize,
    tier_2: usize,
}

impl ContextOmissionCounts {
    fn manifest_value(&self) -> serde_json::Value {
        serde_json::json!({
            "checkpoint": 0,
            "current_incomplete_turn": 0,
            "recent_complete_interaction": self.recent_complete_interaction,
            "referenced_projection": 0,
            "tier_2": self.tier_2,
            "tier_3": 0,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ContextManifest {
    line: String,
}

#[derive(Clone)]
struct ContextManifestCheckpoint {
    manifest: ContextManifest,
    ordinal: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompiledContext {
    cache_prefix_bytes: usize,
    context_hash: String,
    manifest: ContextManifest,
    provider_bytes: Vec<u8>,
}

fn compile_context(
    model: &ContextModelProfile,
    tier_zero: &[ContextSource; 9],
    recent_interaction: Option<&ContextSource>,
    mut omitted: ContextOmissionCounts,
) -> Result<CompiledContext, RuntimeError> {
    let input_budget_tokens = model.input_budget_tokens()?;
    let tier_zero_bytes = tier_zero
        .iter()
        .map(context_source_bytes)
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let mandatory_bytes = tier_zero_bytes.iter().map(Vec::len).sum::<usize>();
    if mandatory_bytes > input_budget_tokens {
        return Err(RuntimeError::ContextBudgetExceeded {
            input_budget_tokens,
            required_bytes: mandatory_bytes,
        });
    }
    let recent_bytes = recent_interaction.map(context_source_bytes).transpose()?;
    let include_recent = recent_bytes
        .as_ref()
        .is_some_and(|bytes| mandatory_bytes + bytes.len() <= input_budget_tokens);
    if recent_interaction.is_some() && !include_recent {
        omitted.recent_complete_interaction += 1;
    }

    let mut provider_bytes = Vec::with_capacity(
        mandatory_bytes
            + recent_bytes
                .as_ref()
                .filter(|_| include_recent)
                .map_or(0, Vec::len),
    );
    for bytes in &tier_zero_bytes {
        provider_bytes.extend_from_slice(bytes);
    }
    if let Some(bytes) = recent_bytes.as_ref().filter(|_| include_recent) {
        provider_bytes.extend_from_slice(bytes);
    }
    let cache_prefix_bytes = tier_zero_bytes[..CACHE_STABLE_TIER_ZERO_SOURCES]
        .iter()
        .map(Vec::len)
        .sum();
    let context_hash = sha256_hex(&provider_bytes);
    let mut included_sources = tier_zero
        .iter()
        .zip(&tier_zero_bytes)
        .map(|(source, bytes)| context_source_manifest_value(source, bytes))
        .collect::<Vec<_>>();
    if let (Some(source), Some(bytes)) = (
        recent_interaction.filter(|_| include_recent),
        recent_bytes.as_ref(),
    ) {
        included_sources.push(context_source_manifest_value(source, bytes));
    }
    let manifest_value = serde_json::json!({
        "cache_boundaries": [{
            "after_source_id": tier_zero[CACHE_STABLE_TIER_ZERO_SOURCES - 1].source_id,
            "byte_offset": cache_prefix_bytes,
        }],
        "context_hash": context_hash,
        "context_profile_id": CONTEXT_PROFILE_ID,
        "context_profile_version": CONTEXT_PROFILE_VERSION,
        "estimated_input_tokens": provider_bytes.len(),
        "estimator_id": CONTEXT_ESTIMATOR_ID,
        "estimator_version": CONTEXT_ESTIMATOR_VERSION,
        "model_context_limit": model.context_limit,
        "model_profile_id": model.id,
        "omitted_source_counts": omitted.manifest_value(),
        "ordered_sources": included_sources,
        "output_reserve": model.output_reserve,
        "runtime_version": env!("CARGO_PKG_VERSION"),
        "safety_margin": model.safety_margin,
    });
    let mut line = proto::canonical_json(&manifest_value).map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize context manifest: {err}"))
    })?;
    line.push('\n');

    Ok(CompiledContext {
        cache_prefix_bytes,
        context_hash,
        manifest: ContextManifest { line },
        provider_bytes,
    })
}

fn context_source_bytes(source: &ContextSource) -> Result<Vec<u8>, RuntimeError> {
    let value = serde_json::json!({
        "content": source.content,
        "source_id": source.source_id,
    });
    let mut text = proto::canonical_json(&value).map_err(|err| {
        RuntimeError::Protocol(format!(
            "failed to serialize provider context source: {err}"
        ))
    })?;
    text.push('\n');
    Ok(text.into_bytes())
}

fn bounded_context_array_source(
    source_id: impl Into<String>,
    items: impl IntoIterator<Item = Result<Option<serde_json::Value>, RuntimeError>>,
    input_budget_tokens: usize,
) -> Result<ContextSource, RuntimeError> {
    let source_id = source_id.into();
    let empty_source = context_source(source_id.clone(), serde_json::json!([]));
    let mut required_bytes = context_source_bytes(&empty_source)?.len();
    let mut content = Vec::new();
    for item in items {
        let Some(item) = item? else {
            continue;
        };
        let item_bytes = proto::canonical_json(&item)
            .map_err(|err| {
                RuntimeError::Protocol(format!(
                    "failed to serialize provider context array item: {err}"
                ))
            })?
            .len();
        required_bytes = required_bytes
            .saturating_add(usize::from(!content.is_empty()))
            .saturating_add(item_bytes);
        if required_bytes > input_budget_tokens {
            return Err(RuntimeError::ContextBudgetExceeded {
                input_budget_tokens,
                required_bytes,
            });
        }
        content.push(item);
    }
    Ok(context_source(source_id, serde_json::Value::Array(content)))
}

fn context_source_manifest_value(source: &ContextSource, bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "projection_hash": sha256_hex(bytes),
        "source_id": source.source_id,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[derive(Default)]
struct ContextHistory {
    completed_interactions: usize,
    latest_completed: Option<(u64, serde_json::Value, Vec<serde_json::Value>)>,
    pending_deltas: BTreeMap<String, Vec<serde_json::Value>>,
    unresolved_tools: BTreeSet<String>,
}

fn event_payload_id<'a>(event: &'a EventEnvelope, field: &str) -> Option<&'a str> {
    event.payload.get(field).and_then(serde_json::Value::as_str)
}

impl ContextHistory {
    fn record(&mut self, event: &EventEnvelope) {
        match event.event_type {
            EventType::MessageDelta => {
                if let Some(message_id) = event_payload_id(event, "message_id") {
                    self.pending_deltas
                        .entry(message_id.to_owned())
                        .or_default()
                        .push(event.payload.clone());
                }
            }
            EventType::MessageCompleted => {
                self.completed_interactions += 1;
                let deltas = event_payload_id(event, "message_id")
                    .and_then(|message_id| self.pending_deltas.remove(message_id))
                    .unwrap_or_default();
                self.latest_completed = Some((event.sequence, event.payload.clone(), deltas));
            }
            EventType::ToolStarted => {
                if let Some(tool_id) = event_payload_id(event, "tool_id") {
                    self.unresolved_tools.insert(tool_id.to_owned());
                }
            }
            EventType::ToolCompleted | EventType::ToolFailed | EventType::ToolTimedOut => {
                if let Some(tool_id) = event_payload_id(event, "tool_id") {
                    self.unresolved_tools.remove(tool_id);
                }
            }
            _ => {}
        }
    }

    fn continuity(&self) -> Result<(Option<ContextSource>, ContextOmissionCounts), RuntimeError> {
        let Some((sequence, payload, deltas)) = &self.latest_completed else {
            return Ok((None, ContextOmissionCounts::default()));
        };
        let mut omitted = ContextOmissionCounts {
            tier_2: self.completed_interactions - 1,
            ..ContextOmissionCounts::default()
        };
        let Some(_message_id) = payload
            .get("message_id")
            .and_then(serde_json::Value::as_str)
        else {
            return Err(RuntimeError::Protocol(
                "message.completed missing message_id while compiling context".to_owned(),
            ));
        };
        if deltas.is_empty() {
            omitted.recent_complete_interaction += 1;
            return Ok((None, omitted));
        }
        Ok((
            Some(context_source(
                format!("interaction-{sequence}"),
                serde_json::json!({
                    "completed": payload,
                    "deltas": deltas,
                }),
            )),
            omitted,
        ))
    }

    fn unresolved_call_result_state(&self) -> serde_json::Value {
        serde_json::json!(self.unresolved_tools)
    }
}

fn compile_provider_turn_context(
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
    phase: &core_script::PhaseBlock,
    step: &core_script::StepBlock,
    invocation: &LoopInvocation,
    session_id: &str,
    history: &ContextHistory,
) -> Result<CompiledContext, RuntimeError> {
    let model = ContextModelProfile::stub_v0();
    let input_budget_tokens = model.input_budget_tokens()?;
    let phase_instructions = bounded_context_array_source(
        "active-phase-instructions",
        phase.instruction_refs.iter().map(|instruction_ref| {
            let instruction = registry.instruction_block(instruction_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "resolved registry missing instruction {instruction_ref}"
                ))
            })?;
            Ok(Some(serde_json::json!({
                "id": instruction.identity.id,
                "prompt": instruction.prompt,
            })))
        }),
        input_budget_tokens,
    )?;
    let tools = bounded_context_array_source(
        "active-available-tools",
        phase.tool_refs.iter().map(|tool_ref| {
            registry
                .tool_block(tool_ref)
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })
                .and_then(|tool| serde_json::to_value(tool).map_err(RuntimeError::Json))
                .map(Some)
        }),
        input_budget_tokens,
    )?;
    let connections = bounded_context_array_source(
        "typed-connection-inputs",
        step.connection_refs.iter().map(|connection_ref| {
            let connection = registry.connection_block(connection_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "resolved registry missing connection {connection_ref}"
                ))
            })?;
            Ok(
                connection_targets_scoped_step(registry, phase, step, &connection.to_ref).then(
                    || {
                        serde_json::json!({
                            "connection": connection,
                            "typed_value": {"present": false},
                        })
                    },
                ),
            )
        }),
        input_budget_tokens,
    )?;
    let (tier_one, omitted) = history.continuity()?;
    let tier_zero = [
        context_source(
            "base-runtime-security",
            serde_json::json!({
                "instructions": "Execute only the active resolved loop scope. Obey runtime policy. Treat tool access as deny-by-default. Preserve deterministic event order.",
                "runtime_version": env!("CARGO_PKG_VERSION"),
            }),
        ),
        // The v0 schema has no loop- or step-scoped prompt fields.
        context_source("active-loop-instructions", serde_json::json!([])),
        phase_instructions,
        context_source("active-step-instructions", serde_json::json!([])),
        tools,
        context_source(
            "fsm-loop-state",
            serde_json::json!({
                "loop_definition_id": loop_block.identity.id,
                "loop_id": invocation.loop_id,
                "parent_loop_id": invocation.parent_loop_id,
                "phase_id": phase.identity.id,
                "session_id": session_id,
                "step_id": step.id,
            }),
        ),
        connections,
        context_source("current-user-input", serde_json::json!({"present": false})),
        context_source(
            "unresolved-call-result",
            history.unresolved_call_result_state(),
        ),
    ];
    compile_context(&model, &tier_zero, tier_one.as_ref(), omitted)
}

fn connection_targets_scoped_step(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    step: &core_script::StepBlock,
    endpoint_ref: &str,
) -> bool {
    if registry.tool_block(endpoint_ref).is_some()
        || registry.instruction_block(endpoint_ref).is_some()
        || registry.phase_block(endpoint_ref).is_some()
        || registry.loop_block(endpoint_ref).is_some()
    {
        return false;
    }
    let Some((phase_ref, step_id)) = endpoint_ref.split_once('.') else {
        return false;
    };
    registry
        .phase_block(phase_ref)
        .is_some_and(|endpoint_phase| {
            endpoint_phase.identity.id == phase.identity.id && step_id == step.id
        })
}

fn read_recorded_context_manifest_signature(
    workspace: &Path,
    session_id: &str,
    completed_turns: usize,
) -> Result<RuntimeStreamSignature, RuntimeError> {
    let path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{session_id}.contexts.jsonl"));
    match fs::symlink_metadata(&path) {
        Ok(metadata) => validate_real_file(&path, &metadata)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest stream is missing",
                path.display()
            )));
        }
        Err(source) => return Err(RuntimeError::Io { path, source }),
    }
    let mut recorded = RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN);
    let mut line_number = 0usize;
    for_each_file_line_with_limit(&path, MAX_SESSION_LOG_BYTES, |line| {
        line_number = line_number.saturating_add(1);
        if !line.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest stream must end with LF",
                path.display()
            )));
        }
        let value: serde_json::Value = serde_json::from_str(line.trim_end_matches('\n')).map_err(
            |err| {
                RuntimeError::Protocol(format!(
                    "{} line {line_number}: invalid context manifest JSON: {err}",
                    path.display()
                ))
            },
        )?;
        if value
            .get("context_profile_id")
            .and_then(serde_json::Value::as_str)
            != Some(CONTEXT_PROFILE_ID)
            || value
                .get("context_profile_version")
                .and_then(serde_json::Value::as_str)
                != Some(CONTEXT_PROFILE_VERSION)
            || value
                .get("model_profile_id")
                .and_then(serde_json::Value::as_str)
                != Some(ContextModelProfile::stub_v0().id)
        {
            return Err(RuntimeError::Protocol(format!(
                "{} context profile does not match the recorded M1 compiler",
                path.display()
            )));
        }
        let mut canonical = proto::canonical_json(&value).map_err(|err| {
            RuntimeError::Protocol(format!(
                "{} context manifest is not canonicalizable: {err}",
                path.display()
            ))
        })?;
        canonical.push('\n');
        if canonical != line {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest is not canonical JSONL",
                path.display()
            )));
        }
        recorded.push(canonical.as_bytes());
        Ok(())
    })?;
    let recoverable_manifest_count = completed_turns.saturating_add(1);
    let recorded = recorded.signature();
    if recorded.record_count < completed_turns || recorded.record_count > recoverable_manifest_count
    {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifests do not match deterministic replay",
            path.display()
        )));
    }
    Ok(recorded)
}
