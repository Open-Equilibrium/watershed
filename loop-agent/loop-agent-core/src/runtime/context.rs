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

    fn input_budget(self) -> Result<usize, RuntimeError> {
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ContextSource {
    source_id: String,
    content: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ContextOptionalCategory {
    RecentCompleteInteraction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ContextOptionalUnit {
    category: ContextOptionalCategory,
    source: ContextSource,
    source_sequence: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ContextOmissionCounts {
    checkpoint: usize,
    current_incomplete_turn: usize,
    recent_complete_interaction: usize,
    referenced_projection: usize,
    tier_2: usize,
    tier_3: usize,
}

impl ContextOmissionCounts {
    fn increment(&mut self, category: ContextOptionalCategory) {
        match category {
            ContextOptionalCategory::RecentCompleteInteraction => {
                self.recent_complete_interaction += 1;
            }
        }
    }

    fn manifest_value(self) -> serde_json::Value {
        serde_json::json!({
            "checkpoint": self.checkpoint,
            "current_incomplete_turn": self.current_incomplete_turn,
            "recent_complete_interaction": self.recent_complete_interaction,
            "referenced_projection": self.referenced_projection,
            "tier_2": self.tier_2,
            "tier_3": self.tier_3,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ContextManifest {
    line: String,
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
    tier_zero: Vec<ContextSource>,
    mut tier_one: Vec<ContextOptionalUnit>,
    mut omitted: ContextOmissionCounts,
) -> Result<CompiledContext, RuntimeError> {
    if tier_zero.len() != 9 {
        return Err(RuntimeError::Protocol(format!(
            "{CONTEXT_PROFILE_ID} requires exactly nine Tier 0 sources"
        )));
    }
    let input_budget = model.input_budget()?;
    let mandatory_bytes = context_sources_bytes(&tier_zero)?;
    if mandatory_bytes.len() > input_budget {
        return Err(RuntimeError::ContextBudgetExceeded {
            input_budget,
            required_bytes: mandatory_bytes.len(),
        });
    }

    tier_one.sort_by_key(|unit| (unit.source_sequence, unit.category));
    let mut optional_bytes = tier_one
        .iter()
        .map(|unit| context_source_bytes(&unit.source))
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let mut total_bytes = mandatory_bytes.len()
        + optional_bytes
            .iter()
            .map(Vec::len)
            .sum::<usize>();
    let mut first_included = 0usize;
    while total_bytes > input_budget {
        let omitted_unit = &tier_one[first_included];
        omitted.increment(omitted_unit.category);
        total_bytes -= optional_bytes[first_included].len();
        first_included += 1;
    }

    let mut provider_bytes = mandatory_bytes;
    for bytes in optional_bytes.drain(first_included..) {
        provider_bytes.extend_from_slice(&bytes);
    }
    let cache_prefix_bytes = context_sources_bytes(
        &tier_zero[..CACHE_STABLE_TIER_ZERO_SOURCES],
    )?
    .len();
    let context_hash = sha256_hex(&provider_bytes);
    let included_sources = tier_zero
        .iter()
        .chain(tier_one[first_included..].iter().map(|unit| &unit.source))
        .map(context_source_manifest_value)
        .collect::<Result<Vec<_>, RuntimeError>>()?;
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

fn context_sources_bytes(sources: &[ContextSource]) -> Result<Vec<u8>, RuntimeError> {
    let mut bytes = Vec::new();
    for source in sources {
        bytes.extend_from_slice(&context_source_bytes(source)?);
    }
    Ok(bytes)
}

fn context_source_bytes(source: &ContextSource) -> Result<Vec<u8>, RuntimeError> {
    let value = serde_json::json!({
        "content": source.content,
        "source_id": source.source_id,
    });
    let mut text = proto::canonical_json(&value).map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize provider context source: {err}"))
    })?;
    text.push('\n');
    Ok(text.into_bytes())
}

fn context_source_manifest_value(
    source: &ContextSource,
) -> Result<serde_json::Value, RuntimeError> {
    let bytes = context_source_bytes(source)?;
    Ok(serde_json::json!({
        "projection_hash": sha256_hex(&bytes),
        "source_id": source.source_id,
    }))
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

fn compile_provider_turn_context(
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
    phase: &core_script::PhaseBlock,
    step: &core_script::StepBlock,
    invocation: &LoopInvocation,
    session_id: &str,
    prior_events: &[EventEnvelope],
) -> Result<CompiledContext, RuntimeError> {
    let phase_instructions = phase
        .instruction_refs
        .iter()
        .map(|instruction_ref| {
            let instruction = registry.instruction_block(instruction_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "resolved registry missing instruction {instruction_ref}"
                ))
            })?;
            Ok(serde_json::json!({
                "id": instruction.identity.id,
                "prompt": instruction.prompt,
            }))
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let tools = phase
        .tool_refs
        .iter()
        .map(|tool_ref| {
            registry
                .tool_block(tool_ref)
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })
                .and_then(|tool| serde_json::to_value(tool).map_err(RuntimeError::Json))
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let connections = step
        .connection_refs
        .iter()
        .map(|connection_ref| {
            let connection = registry.connection_block(connection_ref).ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "resolved registry missing connection {connection_ref}"
                ))
            })?;
            Ok(serde_json::json!({
                "connection": connection,
                "typed_value": {"present": false},
            }))
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let (tier_one, omitted) = context_continuity(prior_events)?;
    let tier_zero = vec![
        ContextSource {
            source_id: "base-runtime-security".to_owned(),
            content: serde_json::json!({
                "instructions": "Execute only the active resolved loop scope. Obey runtime policy. Treat tool access as deny-by-default. Preserve deterministic event order.",
                "runtime_version": env!("CARGO_PKG_VERSION"),
            }),
        },
        ContextSource {
            source_id: "active-loop-instructions".to_owned(),
            // The v0 script schema has no loop-scoped prompt field. Preserve the mandatory
            // section as an explicit empty declaration instead of borrowing inactive prompts.
            content: serde_json::json!([]),
        },
        ContextSource {
            source_id: "active-phase-instructions".to_owned(),
            content: serde_json::Value::Array(phase_instructions),
        },
        ContextSource {
            source_id: "active-step-instructions".to_owned(),
            // The v0 script schema has no step-scoped prompt field.
            content: serde_json::json!([]),
        },
        ContextSource {
            source_id: "active-available-tools".to_owned(),
            content: serde_json::Value::Array(tools),
        },
        ContextSource {
            source_id: "fsm-loop-state".to_owned(),
            content: serde_json::json!({
                "loop_definition_id": loop_block.identity.id,
                "loop_id": invocation.loop_id,
                "parent_loop_id": invocation.parent_loop_id,
                "phase_id": phase.identity.id,
                "session_id": session_id,
                "step_id": step.id,
            }),
        },
        ContextSource {
            source_id: "typed-connection-inputs".to_owned(),
            content: serde_json::Value::Array(connections),
        },
        ContextSource {
            source_id: "current-user-input".to_owned(),
            content: serde_json::json!({"present": false}),
        },
        ContextSource {
            source_id: "unresolved-call-result".to_owned(),
            content: unresolved_call_result_state(prior_events),
        },
    ];
    compile_context(&ContextModelProfile::stub_v0(), tier_zero, tier_one, omitted)
}

fn context_continuity(
    events: &[EventEnvelope],
) -> Result<(Vec<ContextOptionalUnit>, ContextOmissionCounts), RuntimeError> {
    let completed = events
        .iter()
        .filter(|event| event.event_type == EventType::MessageCompleted)
        .collect::<Vec<_>>();
    let mut omitted = ContextOmissionCounts {
        tier_2: completed.len().saturating_sub(1),
        ..ContextOmissionCounts::default()
    };
    let Some(last_completed) = completed.last() else {
        return Ok((Vec::new(), omitted));
    };
    let Some(message_id) = last_completed
        .payload
        .get("message_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Err(RuntimeError::Protocol(
            "message.completed missing message_id while compiling context".to_owned(),
        ));
    };
    let deltas = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::MessageDelta
                && event
                    .payload
                    .get("message_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(message_id)
        })
        .map(|event| event.payload.clone())
        .collect::<Vec<_>>();
    if deltas.is_empty() {
        omitted.recent_complete_interaction += 1;
        return Ok((Vec::new(), omitted));
    }
    Ok((
        vec![ContextOptionalUnit {
            category: ContextOptionalCategory::RecentCompleteInteraction,
            source: ContextSource {
                source_id: format!("interaction-{}", last_completed.sequence),
                content: serde_json::json!({
                    "completed": last_completed.payload,
                    "deltas": deltas,
                }),
            },
            source_sequence: last_completed.sequence,
        }],
        omitted,
    ))
}

fn unresolved_call_result_state(events: &[EventEnvelope]) -> serde_json::Value {
    let mut unresolved = BTreeSet::new();
    for event in events {
        let tool_id = event
            .payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str);
        match event.event_type {
            EventType::ToolStarted => {
                if let Some(tool_id) = tool_id {
                    unresolved.insert(tool_id.to_owned());
                }
            }
            EventType::ToolCompleted
            | EventType::ToolFailed
            | EventType::ToolTimedOut => {
                if let Some(tool_id) = tool_id {
                    unresolved.remove(tool_id);
                }
            }
            _ => {}
        }
    }
    serde_json::json!(unresolved)
}

fn verify_recorded_context_manifests(
    workspace: &Path,
    session_id: &str,
    events: &[EventEnvelope],
    planned: &[ContextManifest],
) -> Result<(), RuntimeError> {
    let path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{session_id}.contexts.jsonl"));
    match fs::symlink_metadata(&path) {
        Ok(metadata) => validate_real_file(&path, &metadata)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(RuntimeError::Io { path, source }),
    }
    let text = read_to_string_with_limit(&path, MAX_SESSION_LOG_BYTES)?;
    if !text.is_empty() && !text.ends_with('\n') {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest stream must end with LF",
            path.display()
        )));
    }
    let mut recorded = Vec::new();
    for line in text.split_inclusive('\n') {
        let value: serde_json::Value = serde_json::from_str(line.trim_end_matches('\n'))?;
        if value.get("context_profile_id").and_then(serde_json::Value::as_str)
            != Some(CONTEXT_PROFILE_ID)
            || value
                .get("context_profile_version")
                .and_then(serde_json::Value::as_str)
                != Some(CONTEXT_PROFILE_VERSION)
            || value.get("model_profile_id").and_then(serde_json::Value::as_str)
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
        recorded.push(ContextManifest { line: canonical });
    }
    let completed_turns = events
        .iter()
        .filter(|event| event.event_type == EventType::MessageCompleted)
        .count();
    if recorded.len() < completed_turns
        || recorded.len() > planned.len()
        || recorded != planned[..recorded.len()]
    {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifests do not match deterministic replay",
            path.display()
        )));
    }
    Ok(())
}
