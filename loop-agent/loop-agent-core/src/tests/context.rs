fn test_profile(input_budget: usize) -> ContextModelProfile {
    ContextModelProfile {
        context_limit: input_budget + 20,
        id: "test-model-v0",
        output_reserve: 10,
        safety_margin: 10,
    }
}

fn tier_zero(turn_value: &str) -> [ContextSource; 9] {
    [
        context_source("base-runtime-security", serde_json::json!({"policy":"deny"})),
        context_source("active-loop-instructions", serde_json::json!([])),
        context_source("active-phase-instructions", serde_json::json!(["phase"])),
        context_source("active-step-instructions", serde_json::json!([])),
        context_source("active-available-tools", serde_json::json!({"z":1,"a":2})),
        context_source("fsm-loop-state", serde_json::json!({"turn":turn_value})),
        context_source("typed-connection-inputs", serde_json::json!([])),
        context_source("current-user-input", serde_json::json!({"present":false})),
        context_source("unresolved-call-result", serde_json::json!([])),
    ]
}

fn compile_summarize_turn_context(
    registry: &core_script::ResolvedRegistry,
) -> CompiledContext {
    let loop_block = registry.loop_block("hello-loop").expect("loop exists");
    let phase = registry.phase_block("summarize").expect("phase exists");
    let step = phase
        .steps
        .iter()
        .find(|step| step.id == "write")
        .expect("step exists");
    compile_provider_turn_context(
        registry,
        loop_block,
        phase,
        step,
        &LoopInvocation {
            loop_id: "hello-loop#1".to_owned(),
            parent_loop_id: None,
        },
        "contextdirection001",
        &[],
    )
    .expect("context compiles")
}

fn context_source_content(compiled: &CompiledContext, source_id: &str) -> serde_json::Value {
    std::str::from_utf8(&compiled.provider_bytes)
        .expect("provider context is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("source parses"))
        .find(|source| source["source_id"] == source_id)
        .unwrap_or_else(|| panic!("missing {source_id} context source"))["content"]
        .clone()
}

fn prefix_before_message_completed(stream: &str) -> String {
    let mut prefix = String::new();
    for line in stream.lines() {
        if line.contains("\"event_type\":\"message.completed\"") {
            break;
        }
        prefix.push_str(line);
        prefix.push('\n');
    }
    prefix
}

#[test]
fn context_compiler_is_deterministic_and_preserves_the_cache_prefix() {
    let first = compile_context(
        &test_profile(16 * 1024),
        &tier_zero("first"),
        None,
        ContextOmissionCounts::default(),
    )
    .expect("first context compiles");
    let second = compile_context(
        &test_profile(16 * 1024),
        &tier_zero("second"),
        None,
        ContextOmissionCounts::default(),
    )
    .expect("second context compiles");

    assert_eq!(
        &first.provider_bytes[..first.cache_prefix_bytes],
        &second.provider_bytes[..second.cache_prefix_bytes]
    );
    assert_eq!(first.cache_prefix_bytes, second.cache_prefix_bytes);
    assert_ne!(first.context_hash, second.context_hash);
    assert!(std::str::from_utf8(&first.provider_bytes)
        .expect("provider context is UTF-8")
        .starts_with("{\"content\":{\"policy\":\"deny\"},\"source_id\":\"base-runtime-security\"}\n"));
    assert!(std::str::from_utf8(&first.provider_bytes)
        .expect("provider context is UTF-8")
        .contains("\"active-available-tools\""));
}

#[test]
fn typed_connection_inputs_exclude_outbound_step_connections() {
    let (registry, _) = fixture_runtime_policy("hello-loop", "hello-loop");
    let with_outbound_reference = compile_summarize_turn_context(&registry);
    let mut inbound_reference_only = registry.clone();
    inbound_reference_only
        .phases
        .get_mut("summarize")
        .expect("phase exists")
        .steps[0]
        .connection_refs = vec!["inspect-trigger".to_owned()];
    let without_outbound_reference = compile_summarize_turn_context(&inbound_reference_only);

    let inputs = context_source_content(&with_outbound_reference, "typed-connection-inputs");
    assert_eq!(inputs.as_array().map(Vec::len), Some(1));
    assert_eq!(inputs[0]["connection"]["id"], "inspect-trigger");
    assert_eq!(
        with_outbound_reference.provider_bytes,
        without_outbound_reference.provider_bytes
    );
    assert_eq!(
        with_outbound_reference.context_hash,
        without_outbound_reference.context_hash
    );
    assert_eq!(
        with_outbound_reference.manifest,
        without_outbound_reference.manifest
    );
}

#[test]
fn typed_connection_inputs_resolve_phase_id_or_name_and_preserve_reference_order() {
    let (mut registry, _) = fixture_runtime_policy("hello-loop", "hello-loop");
    registry
        .connections
        .get_mut("inspect-trigger")
        .expect("trigger exists")
        .to_ref = "Summarize.write".to_owned();
    registry
        .connections
        .get_mut("inspect-data")
        .expect("data connection exists")
        .to_ref = "summarize.write".to_owned();
    registry
        .phases
        .get_mut("summarize")
        .expect("phase exists")
        .steps[0]
        .connection_refs = vec![
            "InspectTrigger".to_owned(),
            "inspect-data".to_owned(),
            "SummaryRefresh".to_owned(),
        ];
    let declared = compile_summarize_turn_context(&registry);

    let inputs = context_source_content(&declared, "typed-connection-inputs");
    assert_eq!(inputs.as_array().map(Vec::len), Some(2));
    assert_eq!(inputs[0]["connection"]["id"], "inspect-trigger");
    assert_eq!(inputs[1]["connection"]["id"], "inspect-data");

    registry
        .phases
        .get_mut("summarize")
        .expect("phase exists")
        .steps[0]
        .connection_refs
        .swap(0, 1);
    let reordered = compile_summarize_turn_context(&registry);
    let reordered_inputs = context_source_content(&reordered, "typed-connection-inputs");
    assert_eq!(reordered_inputs[0]["connection"]["id"], "inspect-data");
    assert_eq!(reordered_inputs[1]["connection"]["id"], "inspect-trigger");
    assert_eq!(declared.cache_prefix_bytes, reordered.cache_prefix_bytes);
    assert_eq!(
        &declared.provider_bytes[..declared.cache_prefix_bytes],
        &reordered.provider_bytes[..reordered.cache_prefix_bytes]
    );
    assert_ne!(declared.provider_bytes, reordered.provider_bytes);
    assert_ne!(declared.context_hash, reordered.context_hash);
    assert_ne!(declared.manifest, reordered.manifest);
}

#[test]
fn context_direction_filter_does_not_change_step_event_connections() {
    let (registry, _) = fixture_runtime_policy("hello-loop", "hello-loop");
    let phase = registry.phase_block("summarize").expect("phase exists");
    let step = phase
        .steps
        .iter()
        .find(|step| step.id == "write")
        .expect("step exists");
    let payload = step_payload(&registry, phase, step).expect("step payload compiles");

    assert_eq!(
        payload["connection_ids"],
        serde_json::json!(["inspect-trigger", "summary-refresh"])
    );
    assert_eq!(
        payload["connection_kinds"],
        serde_json::json!(["trigger", "refresh"])
    );
}

#[test]
fn context_compiler_rejects_mandatory_content_over_budget() {
    let mandatory = tier_zero("large");
    let required = mandatory
        .iter()
        .map(|source| context_source_bytes(source).expect("mandatory source serializes").len())
        .sum::<usize>();
    let err = compile_context(
        &test_profile(required - 1),
        &mandatory,
        None,
        ContextOmissionCounts::default(),
    )
    .expect_err("mandatory context must not be truncated");

    assert!(matches!(
        err,
        RuntimeError::ContextBudgetExceeded {
            required_bytes,
            input_budget
        } if required_bytes == required && input_budget == required - 1
    ));
}

#[test]
fn context_compiler_selects_the_latest_interaction_and_omits_it_whole() {
    let events = [
        (EventType::MessageDelta, "old"),
        (EventType::MessageCompleted, "old"),
        (EventType::MessageDelta, "recent"),
        (EventType::MessageCompleted, "recent"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (event_type, message_id))| {
        let sequence = index as u64 + 1;
        let mut payload = serde_json::json!({"message_id":message_id,"role":"assistant"});
        if event_type == EventType::MessageDelta {
            payload["content_delta"] = serde_json::json!(message_id);
        }
        EventEnvelope::new(
            format!("evt-{sequence}"),
            event_type,
            "context001",
            sequence,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            payload,
        )
    })
    .collect::<Vec<_>>();
    let (recent, omitted) = context_continuity(&events).expect("continuity compiles");
    let recent = recent.expect("latest complete interaction is selected");
    assert_eq!(recent.source_id, "interaction-4");
    assert_eq!(recent.content["deltas"][0]["content_delta"], "recent");
    assert_eq!(omitted.tier_2, 1);

    let mandatory = tier_zero("turn");
    let mandatory_bytes = mandatory
        .iter()
        .map(|source| context_source_bytes(source).expect("mandatory source serializes").len())
        .sum::<usize>();
    let recent_bytes = context_source_bytes(&recent)
        .expect("recent interaction serializes")
        .len();
    let fitting = compile_context(
        &test_profile(mandatory_bytes + recent_bytes),
        &mandatory,
        Some(&recent),
        ContextOmissionCounts::default(),
    )
    .expect("fitting interaction compiles");
    assert!(std::str::from_utf8(&fitting.provider_bytes)
        .expect("context is UTF-8")
        .contains("interaction-4"));
    let compiled = compile_context(
        &test_profile(mandatory_bytes + recent_bytes - 1),
        &mandatory,
        Some(&recent),
        omitted,
    )
    .expect("bounded context compiles");
    let text = std::str::from_utf8(&compiled.provider_bytes).expect("context is UTF-8");

    assert!(!text.contains("interaction-4"));
    let manifest: serde_json::Value = serde_json::from_str(compiled.manifest.line.trim_end())
        .expect("manifest parses");
    assert_eq!(
        manifest["omitted_source_counts"]["recent_complete_interaction"],
        1
    );
    assert_eq!(manifest["omitted_source_counts"]["tier_2"], 1);
}

#[test]
fn run_persists_one_canonical_context_manifest_per_stub_model_turn() {
    let workspace = workspace_copy("hello-loop");
    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("fixture loop completes");
    let manifest_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let text = fs::read_to_string(&manifest_path).expect("context manifests persist");
    let manifests = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("manifest parses"))
        .collect::<Vec<_>>();
    let model_turns = validate_session_log_text(
        &output.session_path,
        &output.session_id,
        &output.stdout,
    )
    .expect("runtime stream validates")
    .iter()
    .filter(|event| event.event_type == EventType::MessageCompleted)
    .count();

    assert_eq!(manifests.len(), model_turns);
    assert!(!manifests.is_empty());
    for manifest in manifests {
        assert_eq!(manifest["context_profile_id"], CONTEXT_PROFILE_ID);
        assert_eq!(manifest["model_profile_id"], "stub-model-v0");
        assert_eq!(manifest["estimator_id"], CONTEXT_ESTIMATOR_ID);
        assert_eq!(
            manifest["context_hash"].as_str().map(str::len),
            Some(64)
        );
        assert_eq!(
            proto::canonical_json(&manifest).expect("manifest canonicalizes"),
            serde_json::to_string(&manifest).expect("manifest serializes canonically")
        );
    }
}

#[test]
fn context_budget_error_maps_to_the_typed_runtime_failure_code() {
    let failure = runtime_failure_for_unhandled_error(&RuntimeError::ContextBudgetExceeded {
        input_budget: 5,
        required_bytes: 6,
    });

    assert_eq!(failure.reason, "context_budget_exceeded");
    assert_eq!(
        failure.message,
        "mandatory context exceeds the model input budget"
    );
}

#[test]
fn dry_run_terminalizes_context_budget_failure_as_typed_events() {
    let (mut registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    registry
        .instructions
        .get_mut("inspect-input")
        .expect("instruction exists")
        .prompt = "x".repeat(STUB_MODEL_CONTEXT_LIMIT);
    let loop_block = registry
        .loop_block("hello-loop")
        .expect("loop exists")
        .clone();

    let runtime = execute_loop(
        Path::new("."),
        &registry,
        &policy,
        &loop_block,
        "contextbudget001",
        LoopExecutionOptions::new(
            EventClock::fixed_fixture(),
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
        ),
    )
    .expect("budget failure becomes a deterministic failed stream");

    assert!(runtime.failed);
    assert!(matches!(
        runtime.terminal_error,
        Some(RuntimeError::ContextBudgetExceeded { .. })
    ));
    assert!(runtime.events.iter().any(|event| {
        event.event_type == EventType::Error
            && event.payload["code"] == "context_budget_exceeded"
    }));
    assert_eq!(
        runtime.events.last().map(|event| &event.event_type),
        Some(&EventType::SessionFailed)
    );
}

#[test]
fn recorded_context_profile_is_verified_before_resume_replay() {
    let workspace = workspace_copy("hello-loop");
    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("fixture loop completes");
    let events = validate_session_log_text(
        &output.session_path,
        &output.session_id,
        &output.stdout,
    )
    .expect("runtime stream validates");
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let loop_block = registry.loop_block("hello-loop").expect("loop exists");
    let planned = execute_loop(
        &workspace,
        &registry,
        &policy,
        loop_block,
        &output.session_id,
        LoopExecutionOptions::new(
            EventClock::fixed_fixture(),
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
        ),
    )
    .expect("deterministic replay plans");
    verify_recorded_context_manifests(
        &workspace,
        &output.session_id,
        &events,
        &planned.context_manifests,
    )
    .expect("recorded manifests match replay");

    let path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let text = fs::read_to_string(&path).expect("manifests read");
    let mut lines = text.lines().collect::<Vec<_>>();
    let mut first: serde_json::Value =
        serde_json::from_str(lines[0]).expect("first manifest parses");
    first["context_profile_id"] = serde_json::json!("different-profile");
    let mut tampered = proto::canonical_json(&first).expect("tampered manifest canonicalizes");
    tampered.push('\n');
    for line in lines.drain(1..) {
        tampered.push_str(line);
        tampered.push('\n');
    }
    fs::write(&path, tampered).expect("manifest tampered");

    let err = verify_recorded_context_manifests(
        &workspace,
        &output.session_id,
        &events,
        &planned.context_manifests,
    )
    .expect_err("profile drift must block resume");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("context profile")
    ));
}

#[test]
fn resume_rejects_invalid_context_manifest_streams_before_side_effects() {
    for (tamper, expected) in [
        ("missing", "context manifest stream is missing"),
        ("missing-lf", "context manifest stream must end with LF"),
        ("whitespace", "context manifest is not canonical JSONL"),
    ] {
        let workspace = workspace_copy("hello-loop");
        let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
            .expect("fixture loop completes");
        let before = prefix_before_tool_started(&output.stdout, "write-summary");
        fs::write(&output.session_path, &before).expect("partial session prefix written");
        write_definition_hash_metadata(
            &workspace,
            &output.session_id,
            "hello-loop",
            before.lines().count(),
        );
        let context_path = workspace
            .join(LOCAL_LOG_DIR)
            .join(format!("{}.contexts.jsonl", output.session_id));
        let context_stream = fs::read_to_string(&context_path).expect("context manifests read");
        match tamper {
            "missing" => fs::remove_file(&context_path).expect("context stream removed"),
            "missing-lf" => fs::write(&context_path, context_stream.trim_end_matches('\n'))
                .expect("unframed context stream written"),
            "whitespace" => fs::write(&context_path, context_stream.replacen('{', "{ ", 1))
                .expect("noncanonical context stream written"),
            _ => unreachable!(),
        }
        fs::remove_file(workspace.join("out/summary.txt"))
            .expect("completed side effect removed");

        let err = resume_session(&workspace, &output.session_id, EmitMode::Jsonl)
            .expect_err("invalid context audit evidence must block resume");

        assert!(matches!(err, RuntimeError::Protocol(message) if message.contains(expected)));
        assert_eq!(
            fs::read_to_string(&output.session_path).expect("session remains readable"),
            before
        );
        assert!(!workspace.join("out/summary.txt").exists());
    }
}

#[test]
fn resume_recovers_one_deterministic_inflight_context_manifest() {
    let workspace = workspace_copy("smoke-loop");
    let output = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect("fixture loop completes");
    let context_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let context_stream = fs::read_to_string(&context_path).expect("context manifest reads");
    let prefix = prefix_before_message_completed(&output.stdout);
    fs::write(&output.session_path, &prefix).expect("incomplete event prefix written");
    write_definition_hash_metadata(
        &workspace,
        &output.session_id,
        "smoke-loop",
        prefix.lines().count(),
    );
    fs::write(&context_path, &context_stream).expect("in-flight manifest restored");

    let resumed = resume_session(&workspace, &output.session_id, EmitMode::Jsonl)
        .expect("one in-flight deterministic manifest is recoverable");

    assert!(!resumed.failed);
    assert_eq!(
        fs::read_to_string(&context_path).expect("recovered manifest reads"),
        context_stream
    );
    let committed = fs::read_to_string(&output.session_path).expect("recovered session reads");
    let events = validate_session_log_text(
        &output.session_path,
        &output.session_id,
        &committed,
    )
    .expect("recovered session validates");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::MessageCompleted)
            .count(),
        1
    );
    assert_eq!(
        events.last().map(|event| &event.event_type),
        Some(&EventType::SessionCompleted)
    );
}

#[test]
fn resume_rejects_more_than_one_future_context_manifest() {
    let workspace = workspace_copy("hello-loop");
    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("fixture loop completes");
    let context_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let context_stream = fs::read_to_string(&context_path).expect("context manifests read");
    assert!(context_stream.lines().count() > 1);
    let prefix = prefix_before_message_completed(&output.stdout);
    fs::write(&output.session_path, &prefix).expect("incomplete event prefix written");
    write_definition_hash_metadata(
        &workspace,
        &output.session_id,
        "hello-loop",
        prefix.lines().count(),
    );
    fs::write(&context_path, context_stream).expect("future manifests restored");
    let before = fs::read_to_string(&output.session_path).expect("event prefix reads");

    let err = resume_session(&workspace, &output.session_id, EmitMode::Jsonl)
        .expect_err("arbitrary future context suffix must remain invalid");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message)
            if message.contains("context manifests do not match deterministic replay")
    ));
    assert_eq!(
        fs::read_to_string(&output.session_path).expect("event prefix remains readable"),
        before
    );
}
