fn source(id: &str, content: serde_json::Value) -> ContextSource {
    ContextSource {
        source_id: id.to_owned(),
        content,
    }
}

fn test_profile(input_budget: usize) -> ContextModelProfile {
    ContextModelProfile {
        context_limit: input_budget + 20,
        id: "test-model-v0",
        output_reserve: 10,
        safety_margin: 10,
    }
}

fn tier_zero(turn_value: &str) -> Vec<ContextSource> {
    vec![
        source("base-runtime-security", serde_json::json!({"policy":"deny"})),
        source("active-loop-instructions", serde_json::json!([])),
        source("active-phase-instructions", serde_json::json!(["phase"])),
        source("active-step-instructions", serde_json::json!([])),
        source("active-available-tools", serde_json::json!({"z":1,"a":2})),
        source("fsm-loop-state", serde_json::json!({"turn":turn_value})),
        source("typed-connection-inputs", serde_json::json!([])),
        source("current-user-input", serde_json::json!({"present":false})),
        source("unresolved-call-result", serde_json::json!([])),
    ]
}

#[test]
fn context_compiler_is_deterministic_and_preserves_the_cache_prefix() {
    let first = compile_context(
        &test_profile(16 * 1024),
        tier_zero("first"),
        Vec::new(),
        ContextOmissionCounts::default(),
    )
    .expect("first context compiles");
    let second = compile_context(
        &test_profile(16 * 1024),
        tier_zero("second"),
        Vec::new(),
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
fn context_compiler_rejects_mandatory_content_over_budget() {
    let mandatory = tier_zero("large");
    let required = context_sources_bytes(&mandatory).expect("mandatory context serializes");
    let err = compile_context(
        &test_profile(required.len() - 1),
        mandatory,
        Vec::new(),
        ContextOmissionCounts::default(),
    )
    .expect_err("mandatory context must not be truncated");

    assert!(matches!(
        err,
        RuntimeError::ContextBudgetExceeded {
            required_bytes,
            input_budget
        } if required_bytes == required.len() && input_budget == required.len() - 1
    ));
}

#[test]
fn context_compiler_omits_oldest_optional_units_whole() {
    let mandatory = tier_zero("turn");
    let mandatory_bytes = context_sources_bytes(&mandatory)
        .expect("mandatory context serializes")
        .len();
    let old = ContextOptionalUnit {
        category: ContextOptionalCategory::RecentCompleteInteraction,
        source: source("interaction-2", serde_json::json!({"message":"old"})),
        source_sequence: 2,
    };
    let recent = ContextOptionalUnit {
        category: ContextOptionalCategory::RecentCompleteInteraction,
        source: source("interaction-8", serde_json::json!({"message":"recent"})),
        source_sequence: 8,
    };
    let recent_bytes = context_sources_bytes(std::slice::from_ref(&recent.source))
        .expect("recent interaction serializes")
        .len();
    let compiled = compile_context(
        &test_profile(mandatory_bytes + recent_bytes),
        mandatory,
        vec![old, recent],
        ContextOmissionCounts::default(),
    )
    .expect("bounded context compiles");
    let text = std::str::from_utf8(&compiled.provider_bytes).expect("context is UTF-8");

    assert!(!text.contains("interaction-2"));
    assert!(text.contains("interaction-8"));
    let manifest: serde_json::Value = serde_json::from_str(compiled.manifest.line.trim_end())
        .expect("manifest parses");
    assert_eq!(
        manifest["omitted_source_counts"]["recent_complete_interaction"],
        1
    );
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
