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
        context_source(
            "base-runtime-security",
            serde_json::json!({"policy":"deny"}),
        ),
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
    loop_id: &str,
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
            loop_id: loop_id.to_owned(),
            parent_loop_id: None,
        },
        "contextdirection001",
        &ContextHistory::default(),
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

fn replace_registry_text(workspace: &Path, path: &str, before: &str, after: &str) {
    let path = workspace.join("registry").join(path);
    let text = fs::read_to_string(&path).expect("registry fixture reads");
    assert!(
        text.contains(before),
        "registry fixture contains target text"
    );
    fs::write(path, text.replacen(before, after, 1)).expect("registry fixture updates");
}

fn exceed_context_budget_with_valid_instructions(workspace: &Path) {
    const PROMPT_BYTES: usize = core_script::MAX_REGISTRY_DEFINITION_BYTES - 4 * 1024;
    let instructions = [
        ("context-load-a", "ContextLoadA"),
        ("context-load-b", "ContextLoadB"),
    ];
    for (id, name) in instructions {
        fs::write(
            workspace
                .join("registry/instructions")
                .join(format!("{id}.yaml")),
            format!(
                "instruction:\n  id: {id}\n  name: {name}\n  prompt: \"{}\"\n",
                "x".repeat(PROMPT_BYTES)
            ),
        )
        .expect("valid large instruction writes");
    }
    replace_registry_text(
        workspace,
        "phases/inspect.yaml",
        "instruction_refs: [inspect-input]",
        "instruction_refs: [context-load-a, context-load-b]",
    );
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
fn provider_context_preserves_tier_zero_order_scope_and_cache_prefix() {
    let (registry, _) = fixture_runtime_policy("hello-loop", "hello-loop");
    let first = compile_summarize_turn_context(&registry, "hello-loop#1");
    let second = compile_summarize_turn_context(&registry, "hello-loop#2");
    let source_lines = std::str::from_utf8(&first.provider_bytes)
        .expect("provider context is UTF-8")
        .lines()
        .collect::<Vec<_>>();
    let sources = source_lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("source parses"))
        .collect::<Vec<_>>();
    let source_ids = sources
        .iter()
        .map(|source| source["source_id"].as_str().expect("source id"))
        .collect::<Vec<_>>();
    let expected_ids = [
        "base-runtime-security",
        "active-loop-instructions",
        "active-phase-instructions",
        "active-step-instructions",
        "active-available-tools",
        "fsm-loop-state",
        "typed-connection-inputs",
        "current-user-input",
        "unresolved-call-result",
    ];
    assert_eq!(source_ids, expected_ids);
    let content_ids = |index: usize| {
        sources[index]["content"]
            .as_array()
            .expect("source content array")
            .iter()
            .map(|item| item["id"].as_str().expect("content id"))
            .collect::<Vec<_>>()
    };
    assert_eq!(sources[1]["content"], serde_json::json!([]));
    assert_eq!(content_ids(2), ["write-output"]);
    assert_eq!(sources[3]["content"], serde_json::json!([]));
    assert_eq!(content_ids(4), ["write-summary"]);
    assert_eq!(sources[7]["content"], serde_json::json!({"present": false}));
    assert_eq!(sources[8]["content"], serde_json::json!([]));
    let provider_text = source_lines.join("\n");
    assert!(!provider_text.contains("inspect-input"));
    assert!(!provider_text.contains("read-file"));

    let expected_prefix: usize = source_lines[..CACHE_STABLE_TIER_ZERO_SOURCES]
        .iter()
        .map(|line| line.len() + 1)
        .sum();
    assert_eq!(first.cache_prefix_bytes, expected_prefix);
    assert_eq!(
        &first.provider_bytes[..first.cache_prefix_bytes],
        &second.provider_bytes[..second.cache_prefix_bytes]
    );
    assert_eq!(first.cache_prefix_bytes, second.cache_prefix_bytes);
    assert_ne!(first.context_hash, second.context_hash);

    let manifest: serde_json::Value =
        serde_json::from_str(first.manifest.line.trim()).expect("manifest parses");
    assert_eq!(
        manifest["cache_boundaries"][0]["after_source_id"],
        expected_ids[4]
    );
    assert_eq!(
        manifest["cache_boundaries"][0]["byte_offset"],
        serde_json::json!(expected_prefix)
    );
    assert_eq!(manifest["context_hash"], sha256_hex(&first.provider_bytes));
    let expected_sources = source_lines
        .iter()
        .zip(expected_ids)
        .map(|(line, source_id)| {
            serde_json::json!({
                "projection_hash": sha256_hex(format!("{line}\n").as_bytes()),
                "source_id": source_id,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        manifest["ordered_sources"],
        serde_json::json!(expected_sources)
    );
}

#[test]
fn typed_connection_inputs_exclude_outbound_step_connections() {
    let workspace = workspace_copy("hello-loop");
    let registry = load_test_registry(&workspace, "hello-loop");
    let with_outbound_reference = compile_summarize_turn_context(&registry, "hello-loop#1");
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "connection_refs: [inspect-trigger, summary-refresh]",
        "connection_refs: [inspect-trigger]",
    );
    let inbound_reference_only = load_test_registry(&workspace, "hello-loop");
    let without_outbound_reference =
        compile_summarize_turn_context(&inbound_reference_only, "hello-loop#1");

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
    let workspace = workspace_copy("hello-loop");
    replace_registry_text(
        &workspace,
        "connections/inspect-trigger.yaml",
        "to_ref: summarize.write",
        "to_ref: Summarize.write",
    );
    replace_registry_text(
        &workspace,
        "connections/inspect-data.yaml",
        "to_ref: inspect.gather",
        "to_ref: summarize.write",
    );
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "connection_refs: [inspect-trigger, summary-refresh]",
        "connection_refs: [InspectTrigger, inspect-data, SummaryRefresh]",
    );
    let registry = load_test_registry(&workspace, "hello-loop");
    let declared = compile_summarize_turn_context(&registry, "hello-loop#1");

    let inputs = context_source_content(&declared, "typed-connection-inputs");
    assert_eq!(inputs.as_array().map(Vec::len), Some(2));
    assert_eq!(inputs[0]["connection"]["id"], "inspect-trigger");
    assert_eq!(inputs[1]["connection"]["id"], "inspect-data");

    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "connection_refs: [InspectTrigger, inspect-data, SummaryRefresh]",
        "connection_refs: [inspect-data, InspectTrigger, SummaryRefresh]",
    );
    let registry = load_test_registry(&workspace, "hello-loop");
    let reordered = compile_summarize_turn_context(&registry, "hello-loop#1");
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
        .map(|source| {
            context_source_bytes(source)
                .expect("mandatory source serializes")
                .len()
        })
        .sum::<usize>();
    let err = compile_context(
        &test_profile(required - 1),
        &mandatory,
        None,
        ContextOmissionCounts::default(),
    )
    .expect_err("mandatory context must not be truncated");

    let message = err.to_string();
    assert!(
        message.contains("canonical bytes (one estimated token per byte)"),
        "{message}"
    );
    assert!(matches!(
        err,
        RuntimeError::ContextBudgetExceeded {
            required_bytes,
            input_budget_tokens
        } if required_bytes == required && input_budget_tokens == required - 1
    ));
}

#[test]
fn context_array_source_stops_materializing_repeated_content_at_its_budget() {
    let item = "x".repeat(64 * 1024);
    let source_id = "active-phase-instructions";
    let one_item_source = context_source(
        source_id,
        serde_json::json!([{"id":"repeated","prompt":item.as_str()}]),
    );
    let input_budget_tokens = context_source_bytes(&one_item_source)
        .expect("one-item source serializes")
        .len();
    let mut materialized = 0;

    let result = bounded_context_array_source(
        source_id,
        (0..4_096).map(|_| {
            materialized += 1;
            Ok(Some(serde_json::json!({
                "id": "repeated",
                "prompt": item.as_str(),
            })))
        }),
        input_budget_tokens,
    );
    let err = match result {
        Err(err) => err,
        Ok(_) => panic!("the second repeated item must exceed the source budget"),
    };

    assert_eq!(materialized, 2);
    assert!(matches!(
        err,
        RuntimeError::ContextBudgetExceeded {
            input_budget_tokens: actual_budget,
            required_bytes,
        } if actual_budget == input_budget_tokens && required_bytes > input_budget_tokens
    ));
}

#[test]
fn context_history_selects_the_latest_interaction_and_omits_it_whole() {
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
    let mut history = ContextHistory::default();
    for event in &events {
        history.record(event);
    }
    let (recent, omitted) = history.continuity().expect("continuity compiles");
    let recent = recent.expect("latest complete interaction is selected");
    assert_eq!(recent.source_id, "interaction-4");
    assert_eq!(recent.content["deltas"][0]["content_delta"], "recent");
    assert_eq!(omitted.tier_2, 1);

    let mandatory = tier_zero("turn");
    let mandatory_bytes = mandatory
        .iter()
        .map(|source| {
            context_source_bytes(source)
                .expect("mandatory source serializes")
                .len()
        })
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
    assert!(
        std::str::from_utf8(&fitting.provider_bytes)
            .expect("context is UTF-8")
            .contains("interaction-4")
    );
    let compiled = compile_context(
        &test_profile(mandatory_bytes + recent_bytes - 1),
        &mandatory,
        Some(&recent),
        omitted,
    )
    .expect("bounded context compiles");
    let text = std::str::from_utf8(&compiled.provider_bytes).expect("context is UTF-8");

    assert!(!text.contains("interaction-4"));
    let manifest: serde_json::Value =
        serde_json::from_str(compiled.manifest.line.trim_end()).expect("manifest parses");
    assert_eq!(
        manifest["omitted_source_counts"]["recent_complete_interaction"],
        1
    );
    assert_eq!(manifest["omitted_source_counts"]["tier_2"], 1);
}

#[test]
fn run_persists_one_canonical_context_manifest_per_stub_model_turn() {
    let workspace = workspace_copy("hello-loop");
    let output =
        run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("fixture loop completes");
    let manifest_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let text = fs::read_to_string(&manifest_path).expect("context manifests persist");
    let manifests = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("manifest parses"))
        .collect::<Vec<_>>();
    let model_turns =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
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
        assert_eq!(manifest["context_hash"].as_str().map(str::len), Some(64));
        assert_eq!(
            proto::canonical_json(&manifest).expect("manifest canonicalizes"),
            serde_json::to_string(&manifest).expect("manifest serializes canonically")
        );
    }
}

#[test]
fn unhandled_errors_map_to_typed_sanitized_runtime_failures() {
    let failure = runtime_failure_for_unhandled_error(&RuntimeError::ContextBudgetExceeded {
        input_budget_tokens: 5,
        required_bytes: 6,
    });

    assert_eq!(failure.reason, "context_budget_exceeded");
    assert_eq!(
        failure.message,
        "mandatory context exceeds the model input budget"
    );
    assert_eq!(
        failure.data,
        serde_json::Map::from_iter([
            ("input_budget_tokens".to_owned(), serde_json::json!(5)),
            ("required_bytes".to_owned(), serde_json::json!(6)),
        ])
    );

    let failure = runtime_failure_for_unhandled_error(&RuntimeError::Io {
        path: PathBuf::from("private/workspace/secret"),
        source: io::Error::new(io::ErrorKind::StorageFull, "private failure detail"),
    });
    assert_eq!(
        failure.data,
        serde_json::Map::from_iter([("io_kind".to_owned(), serde_json::json!("storage_full"))])
    );
}

#[test]
fn dry_run_terminalizes_context_budget_failure_as_typed_events() {
    let workspace = workspace_copy("hello-loop");
    let (_, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    exceed_context_budget_with_valid_instructions(&workspace);
    let registry = load_test_registry(&workspace, "hello-loop");
    let loop_block = registry
        .loop_block("hello-loop")
        .expect("loop exists")
        .clone();

    let mut captured = CapturedRuntime::default();
    let runtime = execute_loop_with_sink(
        &workspace,
        &registry,
        &policy,
        &loop_block,
        "contextbudget001",
        LoopExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::DryRun),
        Some(&mut captured),
    )
    .expect("budget failure becomes a deterministic failed stream");

    assert!(runtime.failed);
    assert!(matches!(
        runtime.terminal_error,
        Some(RuntimeError::ContextBudgetExceeded { .. })
    ));
    assert!(captured.events.iter().any(|event| {
        event.event_type == EventType::Error && event.payload["code"] == "context_budget_exceeded"
    }));
    assert_eq!(
        captured.events.last().map(|event| &event.event_type),
        Some(&EventType::SessionFailed)
    );
}

#[test]
fn persisted_terminal_error_identifies_its_session_and_typed_cause() {
    let workspace = workspace_copy("hello-loop");
    exceed_context_budget_with_valid_instructions(&workspace);

    let err = run_loop(&workspace, "hello-loop", EmitMode::Human)
        .expect_err("the committed context failure must be returned");
    let RuntimeError::SessionFailed { session_id, source } = &err else {
        panic!("expected identified session failure, got {err:?}");
    };
    assert_eq!(session_id, "hello001");
    let RuntimeError::ContextBudgetExceeded {
        input_budget_tokens,
        required_bytes,
    } = source.as_ref()
    else {
        panic!("expected typed context budget cause, got {source:?}");
    };
    assert!(
        err.to_string()
            .starts_with("session hello001 failed: context_budget_exceeded:"),
        "{err}"
    );
    let path = workspace.join(".loop/sessions/hello001.jsonl");
    let stream = read_session_log_to_string(&path).expect("failed session log is readable");
    let events = validate_session_log_text(&path, "hello001", &stream)
        .expect("failed session log remains authoritative");
    let error = events
        .iter()
        .find(|event| event.event_type == EventType::Error)
        .expect("persisted failure includes an error event");
    assert_eq!(
        error.payload["data"],
        serde_json::json!({
            "input_budget_tokens": input_budget_tokens,
            "required_bytes": required_bytes,
        })
    );
    assert_eq!(
        events.last().map(|event| &event.event_type),
        Some(&EventType::SessionFailed)
    );
}

#[test]
fn recorded_context_profile_is_verified_before_resume_replay() {
    let workspace = workspace_copy("hello-loop");
    let output =
        run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("fixture loop completes");
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("runtime stream validates");
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let loop_block = registry.loop_block("hello-loop").expect("loop exists");
    let planned = execute_loop(
        &workspace,
        &registry,
        &policy,
        loop_block,
        &output.session_id,
        LoopExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::DryRun),
    )
    .expect("deterministic replay plans");
    let recorded = read_recorded_context_manifest_signature(
        &workspace,
        &output.session_id,
        events
            .iter()
            .filter(|event| event.event_type == EventType::MessageCompleted)
            .count(),
    )
    .expect("recorded manifests match replay");
    assert_eq!(recorded, planned.context_manifests);

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

    let err = read_recorded_context_manifest_signature(
        &workspace,
        &output.session_id,
        events
            .iter()
            .filter(|event| event.event_type == EventType::MessageCompleted)
            .count(),
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
        ("malformed-json", "line 2: invalid context manifest JSON"),
        ("whitespace", "context manifest is not canonical JSONL"),
    ] {
        let workspace = workspace_copy("hello-loop");
        let output =
            run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("fixture loop completes");
        let before = prefix_before_tool_started(&output.stdout, "write-summary");
        fs::write(&output.session_path, &before).expect("partial session prefix written");
        write_definition_hash_metadata(&workspace, &output.session_id, "hello-loop");
        let context_path = workspace
            .join(LOCAL_LOG_DIR)
            .join(format!("{}.contexts.jsonl", output.session_id));
        let context_stream = fs::read_to_string(&context_path).expect("context manifests read");
        match tamper {
            "missing" => fs::remove_file(&context_path).expect("context stream removed"),
            "missing-lf" => fs::write(&context_path, context_stream.trim_end_matches('\n'))
                .expect("unframed context stream written"),
            "malformed-json" => {
                let mut records = context_stream.lines();
                let first = records.next().expect("first context manifest exists");
                records.next().expect("second context manifest exists");
                let suffix = records.map(|record| format!("{record}\n")).collect::<String>();
                fs::write(&context_path, format!("{first}\n{{\n{suffix}"))
                    .expect("malformed context stream written");
            }
            "whitespace" => fs::write(&context_path, context_stream.replacen('{', "{ ", 1))
                .expect("noncanonical context stream written"),
            _ => unreachable!(),
        }
        fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");

        let err = resume_session(&workspace, &output.session_id, EmitMode::Jsonl)
            .expect_err("invalid context audit evidence must block resume");

        assert!(matches!(
            err,
            RuntimeError::Protocol(message)
                if message.contains(expected)
                    && message.contains(&context_path.display().to_string())
        ));
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
    let output =
        run_loop(&workspace, "smoke-loop", EmitMode::Jsonl).expect("fixture loop completes");
    let context_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let context_stream = fs::read_to_string(&context_path).expect("context manifest reads");
    let prefix = prefix_before_message_completed(&output.stdout);
    fs::write(&output.session_path, &prefix).expect("incomplete event prefix written");
    write_definition_hash_metadata(&workspace, &output.session_id, "smoke-loop");
    fs::write(&context_path, &context_stream).expect("in-flight manifest restored");

    let resumed = resume_session(&workspace, &output.session_id, EmitMode::Jsonl)
        .expect("one in-flight deterministic manifest is recoverable");

    assert!(!resumed.failed);
    assert_eq!(
        fs::read_to_string(&context_path).expect("recovered manifest reads"),
        context_stream
    );
    let committed = fs::read_to_string(&output.session_path).expect("recovered session reads");
    let events = validate_session_log_text(&output.session_path, &output.session_id, &committed)
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
    let output =
        run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("fixture loop completes");
    let context_path = workspace
        .join(LOCAL_LOG_DIR)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let context_stream = fs::read_to_string(&context_path).expect("context manifests read");
    assert!(context_stream.lines().count() > 1);
    let prefix = prefix_before_message_completed(&output.stdout);
    fs::write(&output.session_path, &prefix).expect("incomplete event prefix written");
    write_definition_hash_metadata(&workspace, &output.session_id, "hello-loop");
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
