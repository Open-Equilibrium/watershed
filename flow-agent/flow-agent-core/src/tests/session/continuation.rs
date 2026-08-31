use super::{SessionProvider, session_credential};
use crate::{
    runtime::{
        config_io::load_global_config,
        context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
        conversations::{
            append_productive_run_checkpoint, conversation_status_page,
            create_conversation_run_with_model_profile,
        },
        fs_guards::AnchoredWorkspace,
        instructions::{read_applicable_agent_instructions, read_workspace_agent_instructions},
        live_events::live_event_channel,
        session::{
            continue_conversation, continue_conversation_with_execution_activation,
            continue_conversation_with_live_events, continue_conversation_with_provider,
            resume_conversation_run, resume_conversation_run_with_execution_activation,
            resume_conversation_run_with_live_events, run_flow_with_root_input_and_live_events,
            run_productive_session_with_provider,
        },
        session_reading::SessionEventReader,
        session_store::open_flow_agent_home,
        types::{EmitMode, RuntimeError},
    },
    tests::{
        conversations::{write_terminal_recovery_snapshot, write_terminal_run},
        helpers::{
            disabled_configured_smoke_productive_execution_fixture, empty_workspace,
            write_productive_workspace_config,
        },
        test_support::workspace_copy,
    },
};
use proto::EventType;
use std::{fs, fs::OpenOptions, path::Path};

#[test]
fn productive_continuation_rejects_missing_or_mismatched_parent_profile_before_provider() {
    let (workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
    let config = load_global_config().expect("productive config");
    let registry = &fixture.registry;
    let flow = fixture.smoke_flow();
    let credential = fixture.credential();
    let model_profile = ContextModelProfile {
        context_limit: 128000,
        id: OPERATOR_MODEL_PROFILE_ID,
        output_reserve: 16384,
        safety_margin: CONTEXT_SAFETY_MARGIN,
    };
    let mut initial_provider = SessionProvider::default();
    let initial = run_productive_session_with_provider(
        &workspace,
        &fixture.anchored,
        &config,
        "gpt-fixture",
        model_profile,
        registry,
        flow,
        &fixture.policy,
        None,
        false,
        credential,
        "",
        None,
        &mut initial_provider,
    )
    .expect("initial productive run completes");
    assert_eq!(initial_provider.calls, 1);
    let run_log = crate::tests::helpers::workspace_session_dir(&workspace)
        .join(&initial.session_id)
        .join("runs")
        .join(&initial.session_id)
        .join("run-log.jsonl");
    let original = fs::read_to_string(&run_log).expect("Run Log reads");

    for case in ["missing", "mismatched"] {
        let mut lines = original.lines();
        let mut definition: serde_json::Value =
            serde_json::from_str(lines.next().expect("Run Log begins with its definition"))
                .expect("definition parses");
        {
            let definition = definition.as_object_mut().expect("definition is an object");
            if case == "missing" {
                for field in [
                    "model",
                    "model_profile_id",
                    "model_context_limit",
                    "output_reserve",
                    "safety_margin",
                ] {
                    definition.remove(field);
                }
            } else {
                definition.insert(
                    "model".to_owned(),
                    serde_json::Value::String("gpt-different".to_owned()),
                );
            }
        }
        let mut changed = proto::canonical_json(&definition).expect("definition canonicalizes");
        changed.push('\n');
        for line in lines {
            changed.push_str(line);
            changed.push('\n');
        }
        fs::write(&run_log, changed).expect("invalid profile fixture writes");

        let mut continuation_provider = SessionProvider::default();
        let error = continue_conversation_with_provider(
            &workspace,
            &initial.session_id,
            None,
            None,
            None,
            false,
            credential,
            &mut continuation_provider,
        )
        .expect_err("productive parent profile is required to match");
        assert!(
            error
                .to_string()
                .contains("model profile does not match the recorded Run"),
            "{case}: {error}"
        );
        assert_eq!(continuation_provider.calls, 0, "{case}");
    }
}

#[test]
fn workspace_agent_instructions_are_optional_bounded_and_real() {
    let workspace = empty_workspace("workspace-instructions-absent");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace anchors");
    assert_eq!(
        read_workspace_agent_instructions(&anchored).expect("absent instructions are allowed"),
        ""
    );

    let workspace = empty_workspace("workspace-instructions-present");
    fs::write(workspace.join("AGENTS.md"), "Workspace guidance.\n").expect("instructions write");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace anchors");
    assert_eq!(
        read_workspace_agent_instructions(&anchored).expect("instructions read"),
        "Workspace guidance.\n"
    );

    let workspace = empty_workspace("workspace-instructions-directory");
    fs::create_dir(workspace.join("AGENTS.md")).expect("directory fixture creates");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace anchors");
    assert!(read_workspace_agent_instructions(&anchored).is_err());

    let workspace = empty_workspace("workspace-instructions-oversized");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(workspace.join("AGENTS.md"))
        .expect("oversized fixture opens");
    file.set_len(1024 * 1024 + 1)
        .expect("oversized fixture grows");
    drop(file);
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace anchors");
    assert!(read_workspace_agent_instructions(&anchored).is_err());
}

#[test]
fn applicable_agent_instructions_load_global_then_harness_workspace() {
    let workspace = empty_workspace("applicable-agent-instructions");
    crate::initialize_global_config(None).expect("global Flow authority initializes");
    fs::write(
        crate::tests::test_support::session_home_path().join("AGENTS.md"),
        "Global guidance.\n",
    )
    .expect("global instructions write");
    fs::write(workspace.join("AGENTS.md"), "Workspace guidance.\n")
        .expect("workspace instructions write");
    let home = open_flow_agent_home(false, true)
        .expect("global home opens")
        .expect("global home exists");
    let workspace = AnchoredWorkspace::open(&workspace).expect("workspace anchors");

    assert_eq!(
        read_applicable_agent_instructions(&home, &workspace)
            .expect("applicable instructions read"),
        "Global guidance.\n\nWorkspace guidance.\n"
    );
}

#[cfg(unix)]
#[test]
fn workspace_agent_instructions_refuse_symlinks() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("workspace-instructions-symlink");
    fs::write(workspace.join("real-agents.md"), "Workspace guidance.\n")
        .expect("target instructions write");
    symlink("real-agents.md", workspace.join("AGENTS.md")).expect("symlink fixture creates");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace anchors");
    assert!(read_workspace_agent_instructions(&anchored).is_err());
}

#[test]
fn conversation_entry_points_validate_ids_before_workspace_access() {
    let missing = Path::new("this-workspace-must-not-be-opened");
    for (conversation_id, run_id) in [("INVALID", "run"), ("conversation", "INVALID")] {
        assert!(
            resume_conversation_run(missing, conversation_id, run_id, EmitMode::Human).is_err()
        );
        assert!(
            resume_conversation_run_with_live_events(
                missing,
                conversation_id,
                run_id,
                live_event_channel().0,
            )
            .is_err()
        );
        assert!(
            resume_conversation_run_with_execution_activation(
                missing,
                conversation_id,
                run_id,
                None,
                EmitMode::Human,
                |_| -> Result<(), RuntimeError> { Ok(()) },
            )
            .is_err()
        );
    }
    assert!(continue_conversation(missing, "INVALID", None, None, EmitMode::Human).is_err());
    assert!(
        continue_conversation_with_live_events(
            missing,
            "INVALID",
            None,
            None,
            live_event_channel().0,
        )
        .is_err()
    );
    assert!(
        continue_conversation_with_execution_activation(
            missing,
            "INVALID",
            None,
            None,
            None,
            EmitMode::Human,
            |_| -> Result<(), RuntimeError> { Ok(()) },
        )
        .is_err()
    );
    assert!(
        continue_conversation(
            missing,
            "conversation",
            Some("INVALID"),
            None,
            EmitMode::Human,
        )
        .is_err()
    );
    assert!(
        continue_conversation_with_live_events(
            missing,
            "conversation",
            Some("INVALID"),
            None,
            live_event_channel().0,
        )
        .is_err()
    );
    assert!(
        continue_conversation_with_execution_activation(
            missing,
            "conversation",
            Some("INVALID"),
            None,
            None,
            EmitMode::Human,
            |_| -> Result<(), RuntimeError> { Ok(()) },
        )
        .is_err()
    );
}

#[test]
fn typed_live_run_uses_notifications_instead_of_buffered_stdout() {
    let workspace = workspace_copy("smoke-flow");
    let (notifier, receiver) = live_event_channel();
    let output = run_flow_with_root_input_and_live_events(
        &workspace,
        "smoke-flow",
        core_script::FlowValue::String("operator input".to_owned()),
        notifier,
    )
    .expect("typed live run completes");

    assert!(output.stdout.is_empty());
    assert!(receiver.highest_committed_sequence() > 0);
}

#[test]
fn valid_conversation_addresses_fail_before_provider_or_run_creation() {
    let workspace = workspace_copy("smoke-flow");
    let continuation =
        continue_conversation(&workspace, "conversation001", None, None, EmitMode::Human)
            .expect_err("fixture backend cannot continue a productive conversation");
    assert!(
        continuation
            .to_string()
            .contains("conversation continuation requires provider openai-codex"),
        "{continuation}"
    );

    let (notifier, _receiver) = live_event_channel();
    assert!(
        continue_conversation_with_live_events(
            &workspace,
            "conversation001",
            None,
            None,
            notifier,
        )
        .is_err()
    );
    assert!(
        resume_conversation_run(&workspace, "conversation001", "run001", EmitMode::Human,).is_err()
    );
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("conversation001")
            .exists()
    );
}

#[test]
fn productive_continuation_uses_the_selected_history_input_and_live_checkpoint_stream() {
    let (workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
    let config = load_global_config().expect("productive config");
    let registry = &fixture.registry;
    let flow = fixture.smoke_flow();
    let credential = fixture.credential();
    let mut provider = SessionProvider::default();
    let model_profile = ContextModelProfile {
        context_limit: 128000,
        id: OPERATOR_MODEL_PROFILE_ID,
        output_reserve: 16384,
        safety_margin: CONTEXT_SAFETY_MARGIN,
    };

    run_productive_session_with_provider(
        &workspace,
        &fixture.anchored,
        &config,
        "gpt-fixture",
        model_profile,
        registry,
        flow,
        &fixture.policy,
        None,
        false,
        credential,
        "Agent guidance.",
        None,
        &mut provider,
    )
    .expect("initial productive run completes");
    let page = conversation_status_page(&workspace, None).expect("conversation status");
    let conversation_id = page.conversations[0].conversation_id.clone();
    let selected_entry = page.conversations[0]
        .latest_entry_id
        .clone()
        .expect("latest entry");
    let (notifier, receiver) = live_event_channel();

    let output = continue_conversation_with_provider(
        &workspace,
        &conversation_id,
        Some(&selected_entry),
        Some(core_script::FlowValue::String(
            "operator follow-up".to_owned(),
        )),
        Some(notifier),
        false,
        credential,
        &mut provider,
    )
    .expect("productive continuation completes");

    assert!(!output.failed);
    assert!(
        output.stdout.starts_with("flow smoke-flow (conversation "),
        "notifier-backed execution must not retain canonical JSONL in memory"
    );
    let mut reader =
        SessionEventReader::open_conversation_run(&workspace, &conversation_id, &output.session_id)
            .expect("continuation Run opens");
    let events = reader.read_after(0).expect("continuation Run reads");
    assert_eq!(events.len(), output.event_count);
    assert_eq!(
        events.last().map(|event| event.event_type),
        Some(EventType::SessionCompleted)
    );
    assert_eq!(
        receiver.highest_committed_sequence(),
        events.last().expect("terminal event exists").sequence
    );
    assert_eq!(provider.calls, 2);
    let page = conversation_status_page(&workspace, None).expect("updated conversation status");
    assert_eq!(page.conversations[0].run_count, 2);
    assert_ne!(
        page.conversations[0].latest_entry_id.as_deref(),
        Some(selected_entry.as_str())
    );
}

#[test]
fn conversation_continuation_rejects_registry_drift_before_auth_or_run_creation() {
    let workspace = workspace_copy("smoke-flow");
    write_productive_workspace_config(&workspace);
    create_conversation_run_with_model_profile(
        &workspace,
        "review",
        "review",
        "smoke-flow",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        (
            "gpt-fixture",
            ContextModelProfile {
                context_limit: 128_000,
                id: OPERATOR_MODEL_PROFILE_ID,
                output_reserve: 16_384,
                safety_margin: CONTEXT_SAFETY_MARGIN,
            },
        ),
    )
    .expect("recorded run is created");
    write_terminal_run(&workspace, "review", "review");
    append_productive_run_checkpoint(
        &workspace,
        "review",
        "review",
        None,
        &write_terminal_recovery_snapshot(&workspace, "review", "review"),
        2,
        "2026-07-30T12:00:01Z",
    )
    .expect("root checkpoint appends");

    let mut provider = SessionProvider::default();
    let error = continue_conversation_with_provider(
        &workspace,
        "review",
        None,
        None,
        None,
        false,
        &session_credential(),
        &mut provider,
    )
    .expect_err("registry drift must reject continuation");
    assert!(error.to_string().contains("registry drift"), "{error}");
    assert_eq!(provider.calls, 0, "preflight must not call the provider");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-2")
            .exists(),
        "preflight rejection must not create a run"
    );
}
