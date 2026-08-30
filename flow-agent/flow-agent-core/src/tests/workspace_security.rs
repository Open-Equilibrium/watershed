mod root_binding;

#[cfg(windows)]
use super::helpers::create_windows_junction;
use super::{
    helpers::{
        assert_no_session_artifacts, create_directory_alias, empty_workspace,
        fixture_runtime_policy, replace_registry_text,
    },
    support::assert_denied,
    test_support::{session_home_path, workspace_copy},
};
use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_sink},
    context::ContextManifestCheckpoint,
    event_writer::RuntimeEventSink,
    execution_plan::{FlowExecutionOptions, ToolSideEffectMode, runtime_protected_path_match_mode},
    fixture_tools::{plan_own_script, preflight_own_script_outputs, write_script_output},
    fs_guards::{AnchoredDir, replacement_temp_path},
    planning::plan_flow,
    session::run_flow,
    types::{EmitMode, EventClock, RuntimeError, terminal_failure_reason},
    validate::validate_session_log_text,
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

#[test]
fn shared_workspace_tool_write_parents_are_concurrent_safe() {
    let workspace = workspace_copy("hello-flow");
    fs::remove_dir_all(workspace.join("out")).expect("fixture output dir removed");

    for index in 0..10 {
        let tool = format!(
            "tool:\n  id: write-summary-{index}\n  name: WriteSummary{index}\n  tool_kind: own-script\n  command: script:write-summary-{index}\n  script_runtime: posix-sh\n  script_body: |\n    printf 'hello {index}\\n' > out/summary-{index}.txt\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: [\"workspace/out\"]\n  protected_path_grants: []\n  network: deny\n"
        );
        let phase = format!(
            "phase:\n  id: summarize-{index}\n  name: Summarize{index}\n  instruction_refs: [write-output]\n  tool_refs: [write-summary-{index}]\n  output:\n    type: string\n"
        );
        let flow = format!(
            "flow:\n  id: hello-flow-{index}\n  name: HelloFlow{index}\n  phase_refs: [inspect, summarize-{index}]\n  subflow_refs: []\n"
        );
        for registry in [
            workspace.join("registry"),
            session_home_path().join("registry"),
        ] {
            fs::write(
                registry.join(format!("tools/write-summary-{index}.yaml")),
                &tool,
            )
            .expect("tool fixture written");
            fs::write(
                registry.join(format!("phases/summarize-{index}.yaml")),
                &phase,
            )
            .expect("phase fixture written");
            fs::write(
                registry.join(format!("flows/hello-flow-{index}.yaml")),
                &flow,
            )
            .expect("flow fixture written");
        }
    }

    let barrier = Arc::new(Barrier::new(10));
    let handles = (0..10)
        .map(|index| {
            let workspace = workspace.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                run_flow(
                    workspace.as_ref(),
                    &format!("hello-flow-{index}"),
                    EmitMode::Jsonl,
                )
                .expect("shared workspace flow runs")
            })
        })
        .collect::<Vec<_>>();

    for (index, handle) in handles.into_iter().enumerate() {
        let output = handle.join().expect("thread joins");
        assert!(!output.failed);
        assert_eq!(
            fs::read_to_string(workspace.join(format!("out/summary-{index}.txt")))
                .expect("summary output readable"),
            format!("hello {index}\n")
        );
    }
}

#[cfg(unix)]
#[test]
fn run_flow_rejects_symlinked_log_dir_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-log");
    crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    symlink(
        &outside,
        crate::tests::helpers::workspace_log_dir(&workspace),
    )
    .expect("log dir symlink");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("symlinked log dir must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside.join("smoke-flow.log").exists());
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("smoke-flow.jsonl")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn run_flow_rejects_symlinked_session_leaf_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-session");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let outside_target = outside.join("victim.jsonl");
    symlink(&outside_target, session_dir.join("smoke-flow.jsonl")).expect("session leaf symlink");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("symlinked session leaf must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside_target.exists());
    assert!(
        !crate::tests::helpers::workspace_log_dir(&workspace)
            .join("smoke-flow.log")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn run_flow_rejects_symlinked_summary_leaf_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-summary");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    symlink(&outside_target, workspace.join("out/summary.txt")).expect("summary leaf symlink");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("symlinked summary leaf must fail");

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "symlink",
    );
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_writes_portable_near_limit_output_leaf() {
    let workspace = workspace_copy("hello-flow");
    let leaf = "a".repeat(240);
    let target = format!("out/{leaf}");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "out/summary.txt",
        &target,
    );

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("portable near-limit output leaf runs");

    assert!(!output.failed, "{}", output.stdout);
    assert_eq!(
        fs::read_to_string(workspace.join(target)).expect("long output leaf readable"),
        "hello\n"
    );
}

#[test]
fn run_flow_rejects_multi_write_own_script_before_side_effects() {
    let workspace = workspace_copy("hello-flow");
    fs::write(
        crate::tests::test_support::session_home_path().join("registry/tools/write-summary.yaml"),
        r#"tool:
  id: write-summary
  name: WriteSummary
  tool_kind: own-script
  command: script:write-summary
  script_runtime: posix-sh
  script_body: |
    printf 'partial\n' > out/partial.txt
    printf '%s\n' "$SUMMARY" > out/summary.txt
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("write-summary fixture mutated");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("multi-write own-script must fail before execution");

    assert!(
        matches!(err, RuntimeError::Protocol(ref message) if message.contains("multiple write operations")),
        "{err:?}"
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert!(!workspace.join("out/summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_rejects_non_file_declared_write_paths_before_side_effects() {
    for (leaf_is_directory, expected) in [(true, "must be a file"), (false, "must be a directory")]
    {
        let workspace = workspace_copy("hello-flow");
        let output_parent = workspace.join("out");
        if leaf_is_directory {
            fs::create_dir_all(output_parent.join("summary.txt"))
                .expect("directory created at write leaf");
        } else {
            if output_parent.exists() {
                fs::remove_dir_all(&output_parent).expect("fixture output directory removed");
            }
            fs::write(&output_parent, "not a directory\n").expect("file created in write ancestor");
        }

        let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
            .expect_err("non-file declared write path must fail preflight");

        assert_denied(err, core_policy::DenyReasonCode::WriteDenied, expected);
        assert_no_session_artifacts(&workspace, "hello-flow");
    }
}

#[test]
fn run_flow_commits_failure_stream_when_apply_side_effects_fail() {
    let workspace = workspace_copy("hello-flow");
    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("apply-time side effect failure is recorded as a failed run");

    assert!(output.failed);
    assert!(
        output.stdout.contains("\"reason\":\"write_denied\""),
        "{}",
        output.stdout
    );
    assert!(!summary_path.exists());
    let events = validate_session_log_text(
        Path::new("apply-denial-temp-collision.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("failed apply stream validates");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::ToolFailed)
    );
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert_eq!(
        fs::read_to_string(&output.session_path).expect("session log readable"),
        output.stdout
    );
    assert!(
        crate::tests::helpers::workspace_log_dir(&workspace)
            .join("hello-flow.log")
            .exists()
    );
}

#[test]
fn tool_started_commit_failure_prevents_own_script_side_effect() {
    struct RejectWriteStart;

    impl RuntimeEventSink for RejectWriteStart {
        fn commit(
            &mut self,
            event: &EventEnvelope,
            _canonical_jsonl: &str,
            _context_manifest: Option<ContextManifestCheckpoint>,
        ) -> Result<(), RuntimeError> {
            if event.event_type == EventType::ToolStarted
                && event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("write-summary")
            {
                return Err(RuntimeError::EventWriter(Box::new(RuntimeError::Protocol(
                    "injected tool.started commit failure".to_owned(),
                ))));
            }
            Ok(())
        }
    }

    let workspace = workspace_copy("hello-flow");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow_block = registry
        .flow_block("hello-flow")
        .expect("hello flow exists");
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        flow_block,
        "commitfail001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("flow planning succeeds");
    let err = match apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "commitfail001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        Some(&mut RejectWriteStart),
    ) {
        Err(err) => err,
        Ok(_) => panic!("tool.started commit failure must stop dispatch"),
    };

    assert!(matches!(
        err,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Protocol(message)
                if message.contains("tool.started"))
    ));
    assert!(!workspace.join("out/summary.txt").exists());
}

#[cfg(unix)]
#[test]
fn run_flow_rejects_symlinked_summary_ancestor_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-summary-ancestor");
    fs::remove_dir_all(workspace.join("out")).expect("fixture out directory removed");
    symlink(&outside, workspace.join("out")).expect("summary ancestor symlink");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("symlinked summary ancestor must fail");

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "symlink",
    );
    assert!(!outside.join("summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[cfg(windows)]
#[test]
fn run_flow_rejects_junction_summary_ancestor_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-summary-junction");
    fs::remove_dir_all(workspace.join("out")).expect("fixture out directory removed");
    create_windows_junction(&workspace.join("out"), &outside);

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("junction summary ancestor must fail");

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "reparse",
    );
    assert!(!outside.join("summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[cfg(any(unix, windows))]
#[test]
fn own_script_internal_directory_alias_cannot_escape_write_scope() {
    let workspace = workspace_copy("hello-flow");
    fs::remove_dir_all(workspace.join("out")).expect("fixture out directory removed");
    fs::create_dir(workspace.join("private")).expect("ungranted directory created");
    create_directory_alias(&workspace.join("out"), &workspace.join("private"));

    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let tool = registry
        .tool_block("write-summary")
        .expect("write-summary tool exists");
    let write_policy = policy
        .commands
        .iter()
        .find(|command| command.tool_id == "write-summary")
        .expect("write-summary policy exists");
    let match_mode = runtime_protected_path_match_mode(&policy.target);
    let write = plan_own_script(tool, match_mode, write_policy)
        .expect("own-script plan compiles")
        .expect("own-script plan writes output");
    let anchored_workspace = AnchoredDir::workspace(&workspace).expect("workspace anchors");
    let expected_alias = if cfg!(windows) { "reparse" } else { "symlink" };

    let preflight_err =
        preflight_own_script_outputs(&anchored_workspace, Some(&write), match_mode, write_policy)
            .expect_err("preflight rejects the aliased write ancestor");
    assert_denied(
        preflight_err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        expected_alias,
    );

    let apply_err = write_script_output(
        &anchored_workspace,
        &write.target,
        &write.contents,
        match_mode,
        write_policy,
    )
    .expect_err("apply rejects the aliased write ancestor");
    assert_denied(
        apply_err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        expected_alias,
    );
    assert!(!workspace.join("private/summary.txt").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn run_flow_rejects_hardlinked_summary_leaf_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-summary-hardlink");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    fs::hard_link(&outside_target, workspace.join("out/summary.txt")).expect("summary hard link");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("hard-linked summary leaf must fail");

    assert_denied(err, core_policy::DenyReasonCode::WriteDenied, "hard-linked");
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[cfg(not(any(unix, windows)))]
#[test]
fn run_flow_replaces_hardlinked_summary_leaf_without_modifying_link_target_when_link_count_unverified()
 {
    let workspace = workspace_copy("hello-flow");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    let outside = empty_workspace("outside-summary-hardlink-unverified");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    let summary_path = workspace.join("out/summary.txt");
    fs::hard_link(&outside_target, &summary_path).expect("summary hard link");

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("unverifiable hardlink is safely replaced");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert_eq!(
        fs::read_to_string(&summary_path).expect("summary is replaced"),
        "hello\n"
    );
}
