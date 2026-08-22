use crate::{
    runtime::{
        context::ContextManifestSourceRecord,
        context_persistence::{
            read_anchored_context_manifest_signature, verify_context_manifest_objects,
        },
        execution_plan::{FlowExecutionOptions, ToolSideEffectMode},
        fs_guards::ensure_runtime_dirs,
        planning::plan_flow,
        resume::resume_session,
        session::run_flow,
        types::{
            EmitMode, EventClock, MAX_SESSION_OBJECT_TOTAL_BYTES, MAX_SESSION_OBJECTS, RuntimeError,
        },
        validate::validate_session_log_text,
    },
    tests::{
        helpers::{
            canonical_test_path, empty_workspace, fixture_runtime_policy,
            prefix_before_tool_started, write_definition_hash_metadata,
        },
        test_support::workspace_copy,
    },
};
use proto::EventType;
use std::{collections::BTreeSet, fs};

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
fn recorded_context_profile_is_verified_before_object_io_and_resume_replay() {
    let workspace = workspace_copy("hello-flow");
    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("fixture flow completes");
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("runtime stream validates");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow_block = registry.flow_block("hello-flow").expect("flow exists");
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        flow_block,
        &output.session_id,
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("deterministic replay plans");
    let completed_turns = events
        .iter()
        .filter(|event| event.event_type == EventType::MessageCompleted)
        .count();
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime directories open");
    let recorded = read_anchored_context_manifest_signature(
        &dirs.logs,
        &dirs.sessions,
        &output.session_id,
        completed_turns,
    )
    .expect("recorded manifests match replay");
    assert_eq!(recorded, plan.execution.context_manifests);

    let path = crate::tests::helpers::workspace_log_dir(&workspace)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let text = fs::read_to_string(&path).expect("manifests read");
    let mut lines = text.lines().collect::<Vec<_>>();
    let mut first: serde_json::Value =
        serde_json::from_str(lines[0]).expect("first manifest parses");
    let digest = first["ordered_sources"][0]["object_uri"]
        .as_str()
        .and_then(|uri| uri.strip_prefix("session-object:sha256:"))
        .expect("context object URI")
        .to_owned();
    first["context_profile_id"] = serde_json::json!("different-profile");
    let mut tampered = proto::canonical_json(&first).expect("tampered manifest canonicalizes");
    tampered.push('\n');
    for line in lines.drain(1..) {
        tampered.push_str(line);
        tampered.push('\n');
    }
    fs::write(&path, tampered).expect("manifest tampered");
    fs::remove_file(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join(format!("{}.object.sha256-{digest}", output.session_id)),
    )
    .expect("context object removed");

    let err = read_anchored_context_manifest_signature(
        &dirs.logs,
        &dirs.sessions,
        &output.session_id,
        completed_turns,
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
        ("malformed-json", "line 4: invalid context manifest JSON"),
        ("missing-field", "invalid context manifest record"),
        (
            "projection-mismatch",
            "projection_hash does not match object_uri",
        ),
        ("whitespace", "context manifest is not canonical JSONL"),
    ] {
        let workspace = workspace_copy("hello-flow");
        let output =
            run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("fixture flow completes");
        let before = prefix_before_tool_started(&output.stdout, "write-summary");
        fs::write(&output.session_path, &before).expect("partial session prefix written");
        write_definition_hash_metadata(&workspace, &output.session_id, "hello-flow");
        let context_path = crate::tests::helpers::workspace_log_dir(&workspace)
            .join(format!("{}.contexts.jsonl", output.session_id));
        let diagnostic_path = canonical_test_path(&context_path);
        let context_stream = fs::read_to_string(&context_path).expect("context manifests read");
        match tamper {
            "missing" => fs::remove_file(&context_path).expect("context stream removed"),
            "missing-lf" => fs::write(&context_path, context_stream.trim_end_matches('\n'))
                .expect("unframed context stream written"),
            "malformed-json" => fs::write(&context_path, format!("{context_stream}{{\n"))
                .expect("malformed context stream written"),
            "missing-field" => {
                let mut lines = context_stream
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let mut first: serde_json::Value =
                    serde_json::from_str(&lines[0]).expect("context manifest parses");
                first
                    .as_object_mut()
                    .expect("context manifest is an object")
                    .remove("context_hash");
                lines[0] = proto::canonical_json(&first).expect("manifest canonicalizes");
                fs::write(&context_path, format!("{}\n", lines.join("\n")))
                    .expect("incomplete context manifest written");
            }
            "projection-mismatch" => {
                let mut lines = context_stream
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let mut first: serde_json::Value =
                    serde_json::from_str(&lines[0]).expect("context manifest parses");
                first["ordered_sources"][0]["projection_hash"] = serde_json::json!("0".repeat(64));
                lines[0] = proto::canonical_json(&first).expect("manifest canonicalizes");
                fs::write(&context_path, format!("{}\n", lines.join("\n")))
                    .expect("mismatched context manifest written");
            }
            "whitespace" => fs::write(&context_path, context_stream.replacen('{', "{ ", 1))
                .expect("noncanonical context stream written"),
            _ => unreachable!(),
        }
        fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");

        let err = resume_session(&workspace, &output.session_id, EmitMode::Jsonl)
            .expect_err("invalid context audit evidence must block resume");

        assert!(
            matches!(
            &err,
            RuntimeError::Protocol(message)
                if message.contains(expected)
                    && message.contains(&diagnostic_path.display().to_string())
            ),
            "{tamper} context manifest returned {err:?}"
        );
        assert_eq!(
            fs::read_to_string(&output.session_path).expect("session remains readable"),
            before
        );
        assert!(!workspace.join("out/summary.txt").exists());
    }
}

#[test]
fn resume_rejects_missing_modified_or_invalid_session_context_objects() {
    for (tamper, expected) in [
        ("missing", "referenced context object is unavailable"),
        ("modified", "referenced context object hash does not match"),
        ("invalid-digest", "context manifest object_uri is invalid"),
    ] {
        let workspace = workspace_copy("hello-flow");
        let output =
            run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("fixture flow completes");
        let before = prefix_before_tool_started(&output.stdout, "write-summary");
        fs::write(&output.session_path, &before).expect("partial session prefix written");
        write_definition_hash_metadata(&workspace, &output.session_id, "hello-flow");
        let context_path = crate::tests::helpers::workspace_log_dir(&workspace)
            .join(format!("{}.contexts.jsonl", output.session_id));
        let mut first: serde_json::Value = serde_json::from_str(
            fs::read_to_string(&context_path)
                .expect("context manifests read")
                .lines()
                .next()
                .expect("context manifest exists"),
        )
        .expect("context manifest parses");
        let digest = first["ordered_sources"][0]["object_uri"]
            .as_str()
            .and_then(|uri| uri.strip_prefix("session-object:sha256:"))
            .expect("context object URI")
            .to_owned();
        let object_path = crate::tests::helpers::workspace_session_dir(&workspace)
            .join(format!("{}.object.sha256-{digest}", output.session_id));
        match tamper {
            "missing" => fs::remove_file(&object_path).expect("context object removed"),
            "modified" => fs::write(&object_path, b"modified").expect("context object modified"),
            "invalid-digest" => {
                let invalid = format!("A{}", &digest[1..]);
                first["ordered_sources"][0]["object_uri"] =
                    serde_json::json!(format!("session-object:sha256:{invalid}"));
                first["ordered_sources"][0]["projection_hash"] = serde_json::json!(invalid);
                let mut lines = fs::read_to_string(&context_path)
                    .expect("context manifests read")
                    .lines()
                    .skip(1)
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                lines.insert(
                    0,
                    proto::canonical_json(&first).expect("tampered manifest canonicalizes"),
                );
                fs::write(&context_path, format!("{}\n", lines.join("\n")))
                    .expect("invalid object URI written");
            }
            _ => unreachable!(),
        }
        fs::remove_file(workspace.join("out/summary.txt")).expect("side effect removed");

        let err = resume_session(&workspace, &output.session_id, EmitMode::Jsonl)
            .expect_err("invalid context object must block resume");

        assert!(matches!(
            err,
            RuntimeError::Protocol(message) if message.contains(expected)
        ));
        assert!(!workspace.join("out/summary.txt").exists());
    }
}

#[test]
fn context_object_verification_checks_the_aggregate_before_hashing() {
    let workspace = empty_workspace("context-object-aggregate");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_id = "contextaggregate001";
    let digest = "0".repeat(64);
    fs::write(
        sessions
            .file(format!("{session_id}.object.sha256-{digest}"))
            .diagnostic_path(),
        b"x",
    )
    .expect("context object written");
    let source = ContextManifestSourceRecord {
        object_uri: format!("session-object:sha256:{digest}"),
        projection_hash: digest,
        source_id: String::new(),
    };
    let mut verified = BTreeSet::new();
    let mut verified_bytes = MAX_SESSION_OBJECT_TOTAL_BYTES;

    let err = verify_context_manifest_objects(
        &sessions,
        session_id,
        std::slice::from_ref(&source),
        &mut verified,
        &mut verified_bytes,
    )
    .expect_err("aggregate overflow must precede hash validation");
    assert!(err.to_string().contains("object data size"), "{err}");
}

#[test]
fn context_object_verification_bounds_unique_digests_before_opening_the_excess() {
    let workspace = empty_workspace("context-object-count");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_id = "contextcount001";
    let mut verified = (0..MAX_SESSION_OBJECTS)
        .map(|index| format!("{index:064x}"))
        .collect::<BTreeSet<_>>();
    let mut verified_bytes = 0;
    let duplicate = format!("{:064x}", 0);
    let source = |digest: &str| ContextManifestSourceRecord {
        object_uri: format!("session-object:sha256:{digest}"),
        projection_hash: digest.to_owned(),
        source_id: String::new(),
    };

    let duplicate_source = source(&duplicate);
    verify_context_manifest_objects(
        &sessions,
        session_id,
        std::slice::from_ref(&duplicate_source),
        &mut verified,
        &mut verified_bytes,
    )
    .expect("a duplicate digest does not consume the object-count budget");

    let novel = "f".repeat(64);
    let novel_source = source(&novel);
    let err = verify_context_manifest_objects(
        &sessions,
        session_id,
        std::slice::from_ref(&novel_source),
        &mut verified,
        &mut verified_bytes,
    )
    .expect_err("a novel digest beyond the object-count budget must be rejected");

    assert!(
        matches!(
            err,
            RuntimeError::Protocol(message)
                if message.ends_with("session object count exceeds max 131072")
        ),
        "the excess digest must be rejected before its missing object is opened"
    );
    assert_eq!(verified.len(), MAX_SESSION_OBJECTS);
}

#[test]
fn resume_recovers_one_deterministic_inflight_context_manifest() {
    let workspace = workspace_copy("smoke-flow");
    let output =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("fixture flow completes");
    let context_path = crate::tests::helpers::workspace_log_dir(&workspace)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let context_stream = fs::read_to_string(&context_path).expect("context manifest reads");
    let inflight_manifest = format!(
        "{}\n",
        context_stream
            .lines()
            .next()
            .expect("fixture has a first context manifest")
    );
    let prefix = prefix_before_message_completed(&output.stdout);
    fs::write(&output.session_path, &prefix).expect("incomplete event prefix written");
    write_definition_hash_metadata(&workspace, &output.session_id, "smoke-flow");
    fs::write(&context_path, inflight_manifest).expect("in-flight manifest restored");

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
        2
    );
    assert_eq!(
        events.last().map(|event| &event.event_type),
        Some(&EventType::SessionCompleted)
    );
}

#[test]
fn resume_rejects_more_than_one_future_context_manifest() {
    let workspace = workspace_copy("hello-flow");
    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("fixture flow completes");
    let context_path = crate::tests::helpers::workspace_log_dir(&workspace)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let context_stream = fs::read_to_string(&context_path).expect("context manifests read");
    assert!(context_stream.lines().count() > 1);
    let prefix = prefix_before_message_completed(&output.stdout);
    fs::write(&output.session_path, &prefix).expect("incomplete event prefix written");
    write_definition_hash_metadata(&workspace, &output.session_id, "hello-flow");
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
