use crate::{
    runtime::{
        context::{CONTEXT_ESTIMATOR_ID, CONTEXT_PROFILE_ID},
        digest::sha256_hex,
        failures::runtime_failure_for_unhandled_error,
        session::run_flow,
        session_reading::SessionEventReader,
        types::{EmitMode, MAX_SESSION_OBJECT_BYTES, RuntimeError},
        validate::validate_session_log_text,
    },
    tests::{
        helpers::replace_registry_text,
        test_support::{session_home_path, workspace_copy},
    },
};
use proto::EventType;
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

fn exceed_context_budget_with_valid_instructions(workspace: &Path) {
    const PROMPT_BYTES: usize = core_script::MAX_REGISTRY_DEFINITION_BYTES - 4 * 1024;
    let instructions = [
        ("context-load-a", "ContextLoadA"),
        ("context-load-b", "ContextLoadB"),
    ];
    for (id, name) in instructions {
        let definition = format!(
            "instruction:\n  id: {id}\n  name: {name}\n  prompt: \"{}\"\n",
            "x".repeat(PROMPT_BYTES)
        );
        for registry in [
            workspace.join("registry"),
            session_home_path().join("registry"),
        ] {
            fs::write(
                registry.join("instructions").join(format!("{id}.yaml")),
                &definition,
            )
            .expect("valid large instruction writes");
        }
    }
    replace_registry_text(
        workspace,
        "phases/inspect.yaml",
        "instruction_refs: [inspect-input]",
        "instruction_refs: [context-load-a, context-load-b]",
    );
}

#[test]
fn run_persists_canonical_manifests_objects_and_summarize_model_turn() {
    let workspace = workspace_copy("hello-flow");
    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("fixture flow completes");
    let manifest_path = crate::tests::helpers::workspace_log_dir(&workspace)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let text = fs::read_to_string(&manifest_path).expect("context manifests persist");
    let manifests = text
        .split_inclusive('\n')
        .map(|line| {
            let manifest = serde_json::from_str::<serde_json::Value>(line.trim_end_matches('\n'))
                .expect("manifest parses");
            let canonical = proto::canonical_json(&manifest).expect("manifest canonicalizes");
            assert_eq!(
                line,
                format!("{canonical}\n"),
                "manifest line is canonical JSONL"
            );
            manifest
        })
        .collect::<Vec<_>>();
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("runtime stream validates");
    let model_turns = events
        .iter()
        .filter(|event| event.event_type == EventType::MessageCompleted)
        .count();

    assert_eq!(manifests.len(), model_turns);
    assert!(!manifests.is_empty());
    let mut referenced = 0usize;
    let mut digests = BTreeSet::new();
    for manifest in &manifests {
        assert_eq!(manifest["context_profile_id"], CONTEXT_PROFILE_ID);
        assert_eq!(manifest["model_profile_id"], "stub-model-v0");
        assert_eq!(manifest["estimator_id"], CONTEXT_ESTIMATOR_ID);
        assert_eq!(manifest["context_hash"].as_str().map(str::len), Some(64));
        for source in manifest["ordered_sources"]
            .as_array()
            .expect("ordered sources")
        {
            referenced += 1;
            let digest = source["object_uri"]
                .as_str()
                .and_then(|uri| uri.strip_prefix("session-object:sha256:"))
                .expect("session object URI");
            digests.insert(digest.to_owned());
            let object_path = crate::tests::helpers::workspace_session_dir(&workspace)
                .join(format!("{}.object.sha256-{digest}", output.session_id));
            let bytes = fs::read(object_path).expect("referenced object exists");
            assert_eq!(sha256_hex(&bytes), digest);
            assert!(
                u64::try_from(bytes.len()).unwrap() <= MAX_SESSION_OBJECT_BYTES,
                "context object is independently bounded"
            );
        }
    }
    assert!(
        referenced > digests.len(),
        "repeated context sources deduplicate"
    );

    let start = events
        .iter()
        .position(|event| {
            event.event_type == EventType::PhaseEntered && event.payload["phase_id"] == "summarize"
        })
        .expect("summarize phase starts");
    let end = events[start + 1..]
        .iter()
        .position(|event| {
            event.event_type == EventType::PhaseCompleted
                && event.payload["phase_id"] == "summarize"
        })
        .map(|index| start + 1 + index)
        .expect("summarize phase completes");

    assert!(
        events[start + 1..end]
            .iter()
            .any(|event| event.event_type == EventType::MessageCompleted),
        "an instruction-bearing leaf Phase must run the model regardless of Tool kind"
    );
}

#[test]
fn unhandled_errors_map_to_typed_sanitized_runtime_failures() {
    let denied = runtime_failure_for_unhandled_error(&RuntimeError::Denied {
        reason: core_policy::DenyReasonCode::WriteDenied,
        message: "private policy detail".to_owned(),
    });
    assert_eq!(denied.reason, "write_denied");
    assert_eq!(denied.message, "write outside declared roots denied");
    assert!(denied.data.is_empty());

    let io_kinds = [
        (io::ErrorKind::NotFound, "not_found"),
        (io::ErrorKind::PermissionDenied, "permission_denied"),
        (io::ErrorKind::AlreadyExists, "already_exists"),
        (io::ErrorKind::InvalidInput, "invalid_input"),
        (io::ErrorKind::InvalidData, "invalid_data"),
        (io::ErrorKind::TimedOut, "timed_out"),
        (io::ErrorKind::WriteZero, "write_zero"),
        (io::ErrorKind::StorageFull, "storage_full"),
        (io::ErrorKind::ReadOnlyFilesystem, "read_only_filesystem"),
        (io::ErrorKind::FileTooLarge, "file_too_large"),
        (io::ErrorKind::ResourceBusy, "resource_busy"),
        (io::ErrorKind::Interrupted, "interrupted"),
        (io::ErrorKind::UnexpectedEof, "unexpected_eof"),
        (io::ErrorKind::OutOfMemory, "out_of_memory"),
        (io::ErrorKind::Other, "other"),
    ];
    for (kind, expected) in io_kinds {
        let failure = runtime_failure_for_unhandled_error(&RuntimeError::Io {
            path: PathBuf::from("wire-contract"),
            source: io::Error::from(kind),
        });
        assert_eq!(failure.data["io_kind"], expected);
    }
    let failure = runtime_failure_for_unhandled_error(&RuntimeError::Io {
        path: PathBuf::from("private/workspace/secret"),
        source: io::Error::other("private failure detail"),
    });
    assert!(
        !serde_json::Value::Object(failure.data)
            .to_string()
            .contains("private")
    );
}

#[test]
fn persisted_terminal_error_identifies_its_session_and_typed_cause() {
    let workspace = workspace_copy("hello-flow");
    exceed_context_budget_with_valid_instructions(&workspace);

    let err = run_flow(&workspace, "hello-flow", EmitMode::Human)
        .expect_err("the committed context failure must be returned");
    let RuntimeError::SessionFailed { session_id, source } = &err else {
        panic!("expected identified session failure, got {err:?}");
    };
    assert_eq!(session_id, "hello-flow");
    let RuntimeError::ContextBudgetExceeded {
        input_budget_tokens,
        required_bytes,
    } = source.as_ref()
    else {
        panic!("expected typed context budget cause, got {source:?}");
    };
    assert!(
        err.to_string()
            .starts_with("session hello-flow failed: context_budget_exceeded:"),
        "{err}"
    );
    let mut reader =
        SessionEventReader::open(&workspace, "hello-flow").expect("failed session reader opens");
    let events = reader
        .read_after(0)
        .expect("failed session log remains authoritative");
    let error = events
        .iter()
        .find(|event| event.event_type == EventType::Error)
        .expect("persisted failure includes an error event");
    assert_eq!(error.payload["code"], "context_budget_exceeded");
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
