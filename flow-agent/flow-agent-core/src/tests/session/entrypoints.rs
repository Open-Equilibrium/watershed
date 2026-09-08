use crate::{
    runtime::{
        session::run_flow,
        types::{EmitMode, render_human_failure_status},
        validate::validate_protocol_jsonl_text,
    },
    tests::test_support::workspace_copy,
};
use std::{fs, path::Path};

#[test]
fn run_flow_allocates_unique_session_id_for_repeated_valid_runs() {
    let workspace = workspace_copy("smoke-flow");

    let first =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("first flow run succeeds");
    let second = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("second flow run gets a unique session id");

    assert_eq!(first.session_id, "smoke-flow");
    assert_eq!(second.session_id, "smoke-flow-2");
    assert!(second.stdout.contains("\"session_id\":\"smoke-flow-2\""));
    assert_eq!(
        validate_protocol_jsonl_text(Path::new("second-run.jsonl"), &second.stdout)
            .expect("second run stream remains protocol-valid")
            .len(),
        first.event_count
    );
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("smoke-flow.jsonl")
            .is_file()
    );
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("smoke-flow-2.jsonl")
            .is_file()
    );
    for session_id in [&first.session_id, &second.session_id] {
        let metadata = fs::read_to_string(
            crate::tests::helpers::workspace_log_dir(&workspace).join(format!("{session_id}.log")),
        )
        .expect("definition metadata reads");
        assert!(metadata.starts_with("registry_hash=sha256:"));
        assert_eq!(
            metadata
                .lines()
                .map(|line| line.split_once('=').unwrap().0)
                .collect::<Vec<_>>(),
            [
                "registry_hash",
                "flow_definition_hash",
                "flow_definition_id",
            ]
        );
    }
}

#[test]
fn human_run_reports_terminal_status() {
    let workspace = workspace_copy("smoke-flow");

    let run = run_flow(&workspace, "smoke-flow", EmitMode::Human).expect("flow runs");
    assert!(!run.failed);
    assert_eq!(
        run.stdout,
        "flow smoke-flow (session smoke-flow) completed\n"
    );

    let failed_workspace = workspace_copy("sandbox-negative");
    let failed = run_flow(&failed_workspace, "sandbox-negative-write", EmitMode::Human)
        .expect("negative fixture reaches its deterministic terminal state");
    assert!(failed.failed);
    assert_eq!(
        failed.stdout,
        "flow sandbox-negative-write (session sandbox-negative-write) failed (write_denied): write outside declared roots denied\n"
    );
}

#[test]
fn human_failure_status_escapes_control_characters() {
    assert_eq!(
        render_human_failure_status("line\nbreak\u{1b}[31m", None),
        "failed (line\\nbreak\\u{1b}[31m)"
    );
}
