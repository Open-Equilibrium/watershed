use super::{
    helpers::{
        empty_workspace, load_test_registry, prefix_before_tool_started, replace_registry_text,
        workspace_at_write_summary_progress_with_existing_output, write_definition_hash_metadata,
    },
    test_support::workspace_copy,
};
use crate::runtime::{
    fs_guards::ensure_runtime_dirs,
    resume::resume_session,
    session::run_flow,
    session_definition::{
        parse_session_log_metadata, require_anchored_session_log_metadata,
        verify_resume_definition_metadata,
    },
    types::{EmitMode, RuntimeError},
    validate::{stream_is_completed, validate_session_log_text},
};
use std::fs;

#[test]
fn resume_uses_canonical_registry_strings_and_equivalent_references() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "name: HelloFlow",
        "name: Cafe\u{301}Flow",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\"",
        "printf 'Cafe\u{301}\\n' \"$SUMMARY\"",
    );

    let completed =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("initial run completes");
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is readable"),
        "Café\n"
    );
    let prefix = prefix_before_tool_started(&completed.stdout, "write-summary");
    fs::write(&completed.session_path, &prefix).expect("partial canonical prefix written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-flow");
    fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [Inspect, Summarize]",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf 'Cafe\u{301}\\n' \"$SUMMARY\"",
        "printf 'Café\\n' \"$SUMMARY\"",
    );

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("canonical names and equivalent references preserve resume hashes");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written on resume"),
        "Café\n"
    );
    let resumed = fs::read_to_string(&completed.session_path).expect("resumed log readable");
    let events =
        validate_session_log_text(&completed.session_path, &completed.session_id, &resumed)
            .expect("resumed log validates");
    assert!(stream_is_completed(&events));
}

#[test]
fn session_metadata_rejects_case_aliased_names() {
    let workspace = empty_workspace("session-metadata-case-alias");
    let logs = ensure_runtime_dirs(&workspace).expect("runtime dirs").logs;
    let session_id = "metadataalias001";
    let canonical = logs.file(format!("{session_id}.log"));
    fs::write(canonical.diagnostic_path(), b"").expect("canonical metadata written");
    let alias = canonical
        .diagnostic_path()
        .with_file_name(format!("{session_id}.log").to_ascii_uppercase());
    if cfg!(any(windows, target_os = "macos")) {
        fs::rename(canonical.diagnostic_path(), alias).expect("case-aliased metadata renamed");
    } else {
        fs::write(alias, b"").expect("case-aliased metadata written");
    }

    let err = require_anchored_session_log_metadata(&logs, session_id)
        .expect_err("case-aliased metadata must be rejected");
    assert!(err.to_string().contains("non-canonical"), "{err}");
}

#[test]
fn resume_ignores_unrelated_registry_additions() {
    let (workspace, _) = workspace_at_write_summary_progress_with_existing_output();
    fs::write(
        crate::tests::test_support::session_home_path()
            .join("registry/instructions/unrelated.yaml"),
        "instruction:\n  id: unrelated\n  name: Unrelated\n  prompt: Not used by hello-flow\n",
    )
    .expect("unrelated definition written");

    let output = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("unrelated definition does not change the closure hash");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_rejects_registry_drift_before_side_effects() {
    let (workspace, _) = workspace_at_write_summary_progress_with_existing_output();

    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'drift\\n' > out/summary.txt",
    );

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("registry drift must reject resume");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("registry drift")
    ));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_definition_metadata_rejects_partial_hashes_and_missing_directory() {
    let workspace = workspace_copy("hello-flow");
    let registry = load_test_registry(&workspace, "hello-flow");
    let flow_block = registry.flow_block("hello-flow").expect("flow exists");
    let metadata_path =
        crate::tests::helpers::ensure_workspace_log_dir(&workspace).join("partial001.log");

    fs::write(&metadata_path, "").expect("empty metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without registry hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing registry_hash")
    ));

    fs::write(
        &metadata_path,
        "flow_definition_id=hello-flow\nregistry_hash=sha256:partial\n",
    )
    .expect("partial metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without flow hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing flow_definition_hash")
    ));

    fs::write(
        &metadata_path,
        "registry_hash=sha256:partial\nflow_definition_hash=sha256:partial\n",
    )
    .expect("metadata without flow id writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without flow id must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing flow_definition_id")
    ));

    fs::remove_file(&metadata_path).expect("metadata removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("absent metadata must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));

    fs::remove_dir_all(crate::tests::helpers::workspace_log_dir(&workspace))
        .expect("metadata directory removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("missing metadata directory must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));
}

#[test]
fn session_metadata_and_resume_paths_reject_malformed_inputs() {
    assert!(matches!(
        parse_session_log_metadata("not key value\n"),
        Err(RuntimeError::Protocol(message)) if message.contains("key=value")
    ));
    let workspace = empty_workspace("resume-unsafe-session-id");
    assert!(matches!(
        resume_session(&workspace, "../outside", EmitMode::Jsonl),
        Err(RuntimeError::Usage(message)) if message.contains("invalid session_id")
    ));
    assert!(!workspace.join(".flow").exists());
}
