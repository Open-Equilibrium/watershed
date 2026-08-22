use crate::{
    runtime::{
        conversations::set_legacy_migration_roots_observer,
        resume::resume_session,
        session::{resume_conversation_run_with_execution_activation, run_flow},
        types::{EmitMode, RuntimeError, render_human_failure_status},
        validate::validate_protocol_jsonl_text,
    },
    tests::test_support::workspace_copy,
};
use std::{cell::Cell, fs, path::Path, rc::Rc};

struct TestActivationGuard(Rc<Cell<bool>>);

impl Drop for TestActivationGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

#[cfg(windows)]
use crate::{
    runtime::{
        fs_guards::{
            windows_directory_is_current_user_only_for_test,
            windows_file_is_current_user_only_for_test,
        },
        live_events::live_event_channel,
        session::{
            continue_conversation, continue_conversation_with_execution_activation,
            continue_conversation_with_live_events, resume_conversation_run,
            resume_conversation_run_with_live_events, run_flow_with_execution_activation,
            run_flow_with_live_events, run_flow_with_root_input,
            run_flow_with_root_input_and_live_events,
        },
    },
    tests::helpers::{disable_smoke_echo_tool, write_productive_workspace_config},
};

#[cfg(windows)]
#[test]
fn persisted_session_store_is_current_user_only_on_windows() {
    let workspace = workspace_copy("smoke-flow");
    let output =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("fixture run completes");
    let sessions = output
        .session_path
        .parent()
        .expect("session directory exists");
    let workspace_store = sessions.parent().expect("workspace store exists");
    let workspaces = workspace_store
        .parent()
        .expect("workspaces directory exists");
    let session_home = workspaces.parent().expect("session home exists");

    for directory in [session_home, workspaces, workspace_store, sessions] {
        assert!(
            windows_directory_is_current_user_only_for_test(directory)
                .unwrap_or_else(|error| panic!("{}: {error}", directory.display())),
            "{} must grant access to the current Windows user only",
            directory.display()
        );
    }
    assert!(
        windows_file_is_current_user_only_for_test(&output.session_path)
            .expect("persisted session DACL reads"),
        "{} must grant access to the current Windows user only",
        output.session_path.display()
    );
}

#[test]
fn activated_legacy_resume_binds_its_loaded_backend_before_migration() {
    let workspace = workspace_copy("smoke-flow");
    let legacy =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("legacy fixture run completes");
    let prefix = legacy
        .stdout
        .lines()
        .next()
        .expect("fixture stream starts")
        .to_owned()
        + "\n";
    fs::write(&legacy.session_path, prefix).expect("legacy stream is made resumable");
    let context_path = crate::tests::helpers::workspace_log_dir(&workspace)
        .join(format!("{}.contexts.jsonl", legacy.session_id));
    let context_prefix = fs::read_to_string(&context_path)
        .expect("fixture context stream reads")
        .lines()
        .next()
        .expect("fixture context stream starts")
        .to_owned()
        + "\n";
    fs::write(context_path, context_prefix).expect("context stream is made resumable");
    let active = Rc::new(Cell::new(false));
    let mode = Rc::new(Cell::new(None));
    set_legacy_migration_roots_observer({
        let active = Rc::clone(&active);
        move || {
            assert!(active.get(), "activation guard must outlive migration");
            Ok(())
        }
    });

    let output = resume_conversation_run_with_execution_activation(
        &workspace,
        &legacy.session_id,
        &legacy.session_id,
        None,
        EmitMode::Human,
        {
            let active = Rc::clone(&active);
            let mode = Rc::clone(&mode);
            move |productive| {
                mode.set(Some(productive));
                assert!(!active.replace(true), "activation guard is unique");
                Ok(TestActivationGuard(active))
            }
        },
    )
    .expect("activated legacy resume completes");

    assert_eq!(output.session_id, legacy.session_id);
    assert_eq!(mode.get(), Some(false));
    assert!(!active.get(), "activation guard releases after resume");
}

#[cfg(windows)]
#[test]
fn productive_windows_preflight_guards_every_public_session_entrypoint() {
    let workspace = workspace_copy("smoke-flow");
    disable_smoke_echo_tool(&workspace);
    write_productive_workspace_config(&workspace);
    let root_input = || core_script::FlowValue::String("input".to_owned());
    let notifier = || live_event_channel().0;
    let activations = std::cell::Cell::new(0_u8);
    let activation = |_| -> Result<(), RuntimeError> {
        activations.set(activations.get().saturating_add(1));
        Ok(())
    };
    let errors = vec![
        run_flow(&workspace, "smoke-flow", EmitMode::Human)
            .expect_err("Windows rejects a productive run"),
        run_flow_with_root_input(&workspace, "smoke-flow", root_input(), EmitMode::Jsonl)
            .expect_err("Windows rejects a typed productive run"),
        run_flow_with_live_events(&workspace, "smoke-flow", notifier())
            .expect_err("Windows rejects a live productive run"),
        run_flow_with_root_input_and_live_events(
            &workspace,
            "smoke-flow",
            root_input(),
            notifier(),
        )
        .expect_err("Windows rejects a typed live productive run"),
        run_flow_with_execution_activation(
            &workspace,
            "smoke-flow",
            None,
            None,
            EmitMode::Human,
            activation,
        )
        .expect_err("Windows rejects an activated productive run"),
        continue_conversation(&workspace, "conversation", None, None, EmitMode::Human)
            .expect_err("Windows rejects a productive continuation"),
        continue_conversation_with_live_events(&workspace, "conversation", None, None, notifier())
            .expect_err("Windows rejects a live productive continuation"),
        continue_conversation_with_execution_activation(
            &workspace,
            "conversation",
            None,
            None,
            None,
            EmitMode::Jsonl,
            activation,
        )
        .expect_err("Windows rejects an activated productive continuation"),
        resume_conversation_run(&workspace, "conversation", "run", EmitMode::Human)
            .expect_err("Windows rejects an exact productive resume"),
        resume_conversation_run_with_live_events(&workspace, "conversation", "run", notifier())
            .expect_err("Windows rejects a live exact productive resume"),
        resume_conversation_run_with_execution_activation(
            &workspace,
            "conversation",
            "run",
            None,
            EmitMode::Jsonl,
            activation,
        )
        .expect_err("Windows rejects an activated exact productive resume"),
    ];

    assert!(
        errors
            .iter()
            .all(|error| matches!(error, RuntimeError::ProductiveExecutionUnavailable))
    );
    assert_eq!(activations.get(), 3);
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

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
fn human_run_reports_status_and_resume_rejects_terminal_session() {
    let workspace = workspace_copy("smoke-flow");

    let run = run_flow(&workspace, "smoke-flow", EmitMode::Human).expect("flow runs");
    assert!(!run.failed);
    assert_eq!(
        run.stdout,
        "flow smoke-flow (session smoke-flow) completed\n"
    );

    let before = fs::read_to_string(&run.session_path).expect("terminal session readable");
    assert!(matches!(
        resume_session(&workspace, &run.session_id, EmitMode::Jsonl),
        Err(RuntimeError::TerminalSession(session_id)) if session_id == run.session_id
    ));
    assert_eq!(
        fs::read_to_string(&run.session_path).expect("terminal session remains readable"),
        before
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
