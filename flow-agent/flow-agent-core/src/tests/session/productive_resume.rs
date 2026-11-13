use super::{SessionProvider, session_credential};
use crate::{
    runtime::{
        config_io::load_global_config,
        context::{
            CONTEXT_SAFETY_MARGIN, ContextHistory, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID,
        },
        conversations::{
            ConversationAttemptLog, ConversationEventWriter, ProductiveRecoveryWriter,
            StatusTransactionCrashPoint, append_run_attempt_intent,
            create_conversation_run_with_model_profile, read_conversation_history,
            reserve_conversation_run_recovery, set_conversation_file_sync_error_for_path_for_test,
            set_status_transaction_crash_point,
        },
        live_events::live_event_channel,
        productive::{ProductiveExecution, execute_productive_flow_with_recovery},
        run_attempts::{RunAttemptIntent, RunAttemptKind},
        session::{
            resume_conversation_run_with_provider,
            resume_conversation_run_with_provider_and_live_events,
            set_productive_executor_readiness_observer,
        },
        session_definition::session_definition_metadata,
        session_reading::SessionEventReader,
        types::{EmitMode, EventClock, RuntimeError},
    },
    tests::{
        helpers::{
            ProductiveExecutionFixture, disable_smoke_echo_tool,
            load_productive_execution_fixture_with_credential, replace_registry_text,
            write_productive_workspace_config,
        },
        productive_recovery_support::{
            InterruptingProductiveRecovery, ProductiveInterruptionPoint,
        },
        test_support::{TempWorkspace, workspace_copy},
    },
};
use proto::EventType;
use std::{collections::BTreeMap, fs, io::Write as _, path::Path, time::Duration};

struct ProductiveResumeFixture {
    clock: EventClock,
    definition: crate::runtime::session_definition::SessionDefinitionMetadata,
    execution_fixture: ProductiveExecutionFixture,
    model_profile: ContextModelProfile,
    workspace: TempWorkspace,
}

impl ProductiveResumeFixture {
    fn new(workspace: TempWorkspace) -> Self {
        write_productive_workspace_config(&workspace);
        let config = load_global_config().expect("productive config");
        let execution_fixture = load_productive_execution_fixture_with_credential(
            &workspace,
            "smoke-flow",
            session_credential(),
        );
        let definition = session_definition_metadata(
            &execution_fixture.registry,
            execution_fixture.smoke_flow(),
        )
        .expect("definition metadata");
        let model_profile = ContextModelProfile {
            context_limit: 128000,
            id: OPERATOR_MODEL_PROFILE_ID,
            output_reserve: 16384,
            safety_margin: CONTEXT_SAFETY_MARGIN,
        };
        create_conversation_run_with_model_profile(
            &workspace,
            "conversation",
            "run",
            &definition.flow_definition_id,
            &definition.registry_hash,
            &definition.flow_definition_hash,
            ("gpt-fixture", model_profile),
        )
        .expect("conversation run creates");
        Self {
            clock: config.event_clock,
            definition,
            execution_fixture,
            model_profile,
            workspace,
        }
    }

    fn create_recovery(&self, prior_history: &ContextHistory) -> ProductiveRecoveryWriter {
        ProductiveRecoveryWriter::create(
            &self.workspace,
            "conversation",
            "run",
            &self.definition.flow_definition_id,
            &self.definition.registry_hash,
            &self.definition.flow_definition_hash,
            None,
            None,
            self.clock.base_unix_seconds,
            prior_history,
            0,
        )
        .expect("recovery header creates")
    }

    fn open_recovery_prefix(
        &self,
        prior_history: &ContextHistory,
    ) -> (
        ProductiveRecoveryWriter,
        ConversationEventWriter,
        ConversationAttemptLog,
    ) {
        let recovery = self.create_recovery(prior_history);
        let writer = ConversationEventWriter::open(&self.workspace, "conversation", "run", false)
            .expect("event writer opens");
        let attempts = ConversationAttemptLog::open(&self.workspace, "conversation", "run")
            .expect("conversation attempt log opens");
        (recovery, writer, attempts)
    }

    fn execution(&self, prior_history: ContextHistory) -> ProductiveExecution<'_> {
        ProductiveExecution {
            conversation_id: "conversation",
            clock: self.clock,
            credential: self.execution_fixture.credential(),
            model: "gpt-fixture",
            model_profile: self.model_profile,
            policy: &self.execution_fixture.policy,
            prior_history,
            registry: &self.execution_fixture.registry,
            agent_instructions: "",
            root_flow: self.execution_fixture.smoke_flow(),
            root_input: None,
            session_id: "run",
            workspace: &self.execution_fixture.anchored,
        }
    }
}

#[test]
fn executor_readiness_failure_precedes_recovery_reservation() {
    let fixture = ProductiveResumeFixture::new(workspace_copy("smoke-flow"));
    drop(fixture.create_recovery(&ContextHistory::default()));
    // Another owner holds the recovery reservation, so readiness must fail before any reservation attempt.
    let held = reserve_conversation_run_recovery(&fixture.workspace, "conversation", "run")
        .expect("another owner holds the recovery reservation");
    set_productive_executor_readiness_observer(|| {
        Err(RuntimeError::executor(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "injected unsupported Tool platform",
        ))
    });
    let mut provider = SessionProvider::default();

    let error = resume_conversation_run_with_provider(
        &fixture.workspace,
        "conversation",
        "run",
        EmitMode::Human,
        || panic!("credential resolution must follow Executor readiness"),
        &mut provider,
    )
    .expect_err("failed readiness must stop before recovery reservation");

    assert!(matches!(
        error,
        RuntimeError::Executor(ref failure)
            if failure.code() == proto::ExecutorErrorCodeV0::PolicyUnsupported
    ));
    assert_eq!(provider.calls, 0);
    held.release().expect("recovery reservation releases");
    reserve_conversation_run_recovery(&fixture.workspace, "conversation", "run")
        .expect("failed readiness leaves recovery reservation available")
        .release()
        .expect("recovery reservation releases");
}

#[test]
fn paired_resume_refuses_to_redispatch_an_uncertain_productive_attempt() {
    let workspace = workspace_copy("smoke-flow");
    disable_smoke_echo_tool(&workspace);
    let fixture = ProductiveResumeFixture::new(workspace);
    append_run_attempt_intent(
        &fixture.workspace,
        "conversation",
        "run",
        &RunAttemptIntent {
            attempt_id: "provider-001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                .to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect("uncertain Provider intent is durable");
    let run_log = crate::tests::helpers::workspace_session_dir(&fixture.workspace)
        .join("conversation/runs/run/run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before Resume");
    let mut provider = SessionProvider::default();

    let error = resume_conversation_run_with_provider(
        &fixture.workspace,
        "conversation",
        "run",
        EmitMode::Human,
        || Ok(fixture.execution_fixture.credential().clone()),
        &mut provider,
    )
    .expect_err("Resume must not automatically repeat an uncertain attempt");

    assert_eq!(error.exit_code(), 65);
    assert!(error.to_string().contains("uncertain"), "{error}");
    assert_eq!(provider.calls, 0, "provider must not redispatch");
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after rejected Resume"),
        before
    );
}

#[test]
fn exact_productive_resume_reuses_the_committed_attempt_and_finishes_the_addressed_run() {
    let workspace = workspace_copy("smoke-flow");
    disable_smoke_echo_tool(&workspace);
    let fixture = ProductiveResumeFixture::new(workspace);
    let prior_history = ContextHistory::default();
    let (mut recovery, mut writer, mut attempts) = fixture.open_recovery_prefix(&prior_history);
    let mut initial_provider = SessionProvider::default();
    {
        let mut interruption =
            InterruptingProductiveRecovery::new(&mut recovery, ProductiveInterruptionPoint::Phase);
        let error = execute_productive_flow_with_recovery(
            fixture.execution(prior_history),
            &mut initial_provider,
            &mut attempts,
            &mut writer,
            &mut interruption,
        )
        .expect_err("interruption leaves an exact-recovery prefix");
        assert!(
            error
                .to_string()
                .contains("interruption after committed provider result")
        );
    }
    writer.finish().expect("event prefix finalizes");
    drop(recovery);
    assert_eq!(initial_provider.calls, 1);

    let mut recovery_provider = SessionProvider::default();
    let output = resume_conversation_run_with_provider(
        &fixture.workspace,
        "conversation",
        "run",
        EmitMode::Human,
        || Ok(fixture.execution_fixture.credential().clone()),
        &mut recovery_provider,
    )
    .expect("exact productive resume completes");

    assert!(!output.failed);
    assert_eq!(output.session_id, "run");
    assert!(output.stdout.contains("run run) completed"));
    assert_eq!(recovery_provider.calls, 0, "provider must not redispatch");
    assert_eq!(
        read_conversation_history(&fixture.workspace, "conversation")
            .expect("conversation history")
            .len(),
        1
    );
}

fn interrupted_before_terminal_snapshot() -> ProductiveResumeFixture {
    let workspace = workspace_copy("smoke-flow");
    disable_smoke_echo_tool(&workspace);
    let fixture = ProductiveResumeFixture::new(workspace);
    let prior_history = ContextHistory::default();
    let (mut recovery, mut writer, mut attempts) = fixture.open_recovery_prefix(&prior_history);
    let mut interruption =
        InterruptingProductiveRecovery::new(&mut recovery, ProductiveInterruptionPoint::Terminal);
    execute_productive_flow_with_recovery(
        fixture.execution(prior_history),
        &mut SessionProvider::default(),
        &mut attempts,
        &mut writer,
        &mut interruption,
    )
    .expect_err("interruption leaves every pre-terminal recovery boundary");
    writer.finish().expect("event prefix finalizes");
    fixture
}

fn run_objects(fixture: &ProductiveResumeFixture) -> BTreeMap<String, Vec<u8>> {
    let objects = crate::tests::helpers::workspace_session_dir(&fixture.workspace)
        .join("conversation/runs/run/objects");
    fs::read_dir(objects)
        .expect("run objects list")
        .map(|entry| {
            let entry = entry.expect("run object entry reads");
            (
                entry.file_name().into_string().expect("object name"),
                fs::read(entry.path()).expect("run object reads"),
            )
        })
        .collect()
}

fn resume_exact_run(fixture: &ProductiveResumeFixture) -> Result<bool, RuntimeError> {
    let mut provider = SessionProvider::default();
    let output = resume_conversation_run_with_provider(
        &fixture.workspace,
        "conversation",
        "run",
        EmitMode::Human,
        || Ok(fixture.execution_fixture.credential().clone()),
        &mut provider,
    )?;
    assert_eq!(provider.calls, 0, "provider must not redispatch");
    Ok(output.failed)
}

#[test]
fn exact_productive_resume_completes_an_interrupted_run_object_write() {
    let reference = interrupted_before_terminal_snapshot();
    let prefix_objects = run_objects(&reference);
    assert_eq!(resume_exact_run(&reference).ok(), Some(false));
    let completed_objects = run_objects(&reference);
    let resume_objects = completed_objects
        .iter()
        .filter(|(name, _)| !prefix_objects.contains_key(*name))
        .collect::<Vec<_>>();
    assert!(
        !resume_objects.is_empty(),
        "Resume writes a terminal object"
    );

    let fixture = interrupted_before_terminal_snapshot();
    assert_eq!(run_objects(&fixture), prefix_objects);
    let objects = crate::tests::helpers::workspace_session_dir(&fixture.workspace)
        .join("conversation/runs/run/objects");
    for (name, bytes) in &resume_objects {
        fs::write(objects.join(name), &bytes[..bytes.len() / 2])
            .expect("fixture leaves an interrupted object write");
    }

    assert_eq!(
        resume_exact_run(&fixture).map_err(|error| error.to_string()),
        Ok(false)
    );
    assert_eq!(run_objects(&fixture), completed_objects);
}

#[test]
fn exact_productive_resume_resyncs_a_complete_recovery_record_before_publication() {
    let workspace = workspace_copy("smoke-flow");
    disable_smoke_echo_tool(&workspace);
    let fixture = ProductiveResumeFixture::new(workspace);
    let prior_history = ContextHistory::default();
    let (mut recovery, mut writer, mut attempts) = fixture.open_recovery_prefix(&prior_history);
    let mut initial_provider = SessionProvider::default();
    {
        let mut interruption = InterruptingProductiveRecovery::new(
            &mut recovery,
            ProductiveInterruptionPoint::Terminal,
        );
        let error = execute_productive_flow_with_recovery(
            fixture.execution(prior_history),
            &mut initial_provider,
            &mut attempts,
            &mut writer,
            &mut interruption,
        )
        .expect_err("interruption leaves every pre-terminal recovery boundary");
        assert!(error.to_string().contains("prevents a terminal snapshot"));
    }
    writer.finish().expect("event prefix finalizes");
    drop(recovery);
    assert_eq!(initial_provider.calls, 1);

    let run = crate::tests::helpers::workspace_session_dir(&fixture.workspace)
        .join("conversation/runs/run");
    let recovery_path = run.join("recovery.jsonl");
    let history_path = crate::tests::helpers::workspace_session_dir(&fixture.workspace)
        .join("conversation/history.jsonl");
    let history_before = fs::read(&history_path).expect("history prefix reads");
    let mut resumed_recovery =
        ProductiveRecoveryWriter::open_for_resume(&fixture.workspace, "conversation", "run")
            .expect("recovery snapshot reopens before the injected append failure");
    let mut resumed_writer = ConversationEventWriter::open_for_recovery(
        &fixture.workspace,
        "conversation",
        "run",
        false,
        None,
    )
    .expect("recovery event writer opens");
    let mut resumed_attempts =
        ConversationAttemptLog::open(&fixture.workspace, "conversation", "run")
            .expect("recovery attempt log opens");
    set_conversation_file_sync_error_for_path_for_test(&recovery_path, std::io::ErrorKind::Other);
    let mut first_retry_provider = SessionProvider::default();
    let error = execute_productive_flow_with_recovery(
        fixture.execution(ContextHistory::default()),
        &mut first_retry_provider,
        &mut resumed_attempts,
        &mut resumed_writer,
        &mut resumed_recovery,
    )
    .expect_err("terminal recovery target sync failure is reported");
    resumed_writer
        .finish()
        .expect("event prefix finalizes after failed terminal recovery sync");
    drop(resumed_recovery);
    assert!(
        error
            .to_string()
            .contains("injected conversation file synchronization failure"),
        "{error}"
    );
    assert_eq!(
        first_retry_provider.calls, 0,
        "provider must not redispatch"
    );
    assert_eq!(
        fs::read(&history_path).expect("history remains readable"),
        history_before
    );
    let recovery_records = fs::read_to_string(&recovery_path)
        .expect("visible recovery prefix reads")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("record parses"))
        .collect::<Vec<_>>();
    assert_eq!(
        recovery_records
            .last()
            .and_then(|record| record["record_type"].as_str()),
        Some("terminal")
    );
    let mut reader =
        SessionEventReader::open_conversation_run(&fixture.workspace, "conversation", "run")
            .expect("incomplete run opens");
    let events_after_first_failure = reader.read_after(0).expect("event prefix reads");
    assert_ne!(
        events_after_first_failure
            .last()
            .map(|event| event.event_type),
        Some(EventType::SessionCompleted)
    );

    set_conversation_file_sync_error_for_path_for_test(&recovery_path, std::io::ErrorKind::Other);
    let events_before_reopen = fs::read(run.join("events.jsonl")).expect("event prefix reads");
    let mut second_retry_provider = SessionProvider::default();
    let error = resume_conversation_run_with_provider(
        &fixture.workspace,
        "conversation",
        "run",
        EmitMode::Human,
        || Ok(fixture.execution_fixture.credential().clone()),
        &mut second_retry_provider,
    )
    .expect_err("reopen must resynchronize the visible terminal record");
    assert!(
        error
            .to_string()
            .contains("injected conversation file synchronization failure"),
        "{error}"
    );
    assert_eq!(
        second_retry_provider.calls, 0,
        "provider must not redispatch"
    );
    assert_eq!(
        fs::read(run.join("events.jsonl")).expect("event prefix remains readable"),
        events_before_reopen
    );
    assert_eq!(
        fs::read(&history_path).expect("history remains readable"),
        history_before
    );

    let mut final_retry_provider = SessionProvider::default();
    let output = resume_conversation_run_with_provider(
        &fixture.workspace,
        "conversation",
        "run",
        EmitMode::Human,
        || Ok(fixture.execution_fixture.credential().clone()),
        &mut final_retry_provider,
    )
    .expect("the synchronized terminal record resumes exactly once");
    assert!(!output.failed);
    assert_eq!(
        final_retry_provider.calls, 0,
        "provider must not redispatch"
    );
    let mut reader =
        SessionEventReader::open_conversation_run(&fixture.workspace, "conversation", "run")
            .expect("completed run opens");
    let events = reader.read_after(0).expect("completed run reads");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::SessionCompleted)
            .count(),
        1
    );
    assert_eq!(
        read_conversation_history(&fixture.workspace, "conversation")
            .expect("conversation history")
            .len(),
        1
    );
}

#[test]
fn exact_productive_resume_announces_committed_failure_before_returning_error() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "phases/smoke.yaml",
        "tool_refs: [echo]\n  output:\n    type: string",
        "tool_refs: []\n  output:\n    type: string\n  loop:\n    max_iterations: 1\n    until:\n      path: []\n      equals:\n        type: string\n        value: done",
    );
    let fixture = ProductiveResumeFixture::new(workspace);
    let prior_history = ContextHistory::default();
    let (mut recovery, mut writer, mut attempts) = fixture.open_recovery_prefix(&prior_history);
    let mut initial_provider = SessionProvider::default();
    {
        let mut interruption =
            InterruptingProductiveRecovery::new(&mut recovery, ProductiveInterruptionPoint::Phase);
        execute_productive_flow_with_recovery(
            fixture.execution(prior_history),
            &mut initial_provider,
            &mut attempts,
            &mut writer,
            &mut interruption,
        )
        .expect_err("interruption leaves an exact-recovery prefix");
    }
    writer.finish().expect("event prefix finalizes");
    drop(recovery);

    let (notifier, receiver) = live_event_channel();
    let mut recovery_provider = SessionProvider::default();
    let error = resume_conversation_run_with_provider_and_live_events(
        &fixture.workspace,
        "conversation",
        "run",
        fixture.execution_fixture.credential(),
        &mut recovery_provider,
        notifier,
    )
    .expect_err("loop exhaustion returns the committed terminal error");

    assert!(error.to_string().contains("max_iterations"), "{error}");
    let notification = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("committed recovery failure is announced");
    assert_eq!(
        notification.conversation_id.as_deref(),
        Some("conversation")
    );
    assert_eq!(notification.session_id, "run");
    let mut reader =
        SessionEventReader::open_conversation_run(&fixture.workspace, "conversation", "run")
            .expect("recovered run opens");
    let events = reader.read_after(0).expect("recovered run reads");
    assert_eq!(
        events.last().map(|event| event.event_type),
        Some(EventType::SessionFailed)
    );
    assert_eq!(
        receiver.highest_committed_sequence(),
        events.last().expect("terminal event exists").sequence
    );
    assert_eq!(recovery_provider.calls, 0, "provider must not redispatch");
}

/// Truncates the final byte of a durable file, as a power cut that loses its line feed does.
fn cut_final_byte(path: &Path) {
    let len = fs::metadata(path).expect("durable file reads").len();
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("durable file opens")
        .set_len(len - 1)
        .expect("final byte is cut");
}

fn conversation_history_path(fixture: &ProductiveResumeFixture) -> std::path::PathBuf {
    crate::tests::helpers::workspace_session_dir(&fixture.workspace)
        .join("conversation/history.jsonl")
}

#[test]
fn exact_productive_resume_rolls_back_a_cut_run_log_intent_and_issues_its_attempt() {
    let workspace = workspace_copy("smoke-flow");
    disable_smoke_echo_tool(&workspace);
    let fixture = ProductiveResumeFixture::new(workspace);
    drop(fixture.create_recovery(&ContextHistory::default()));
    set_status_transaction_crash_point(StatusTransactionCrashPoint::CanonicalMutationApplied);
    append_run_attempt_intent(
        &fixture.workspace,
        "conversation",
        "run",
        &RunAttemptIntent {
            attempt_id: "provider-001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect_err("the crash point leaves the intent's status transaction recorded");
    let run_log = crate::tests::helpers::workspace_session_dir(&fixture.workspace)
        .join("conversation/runs/run/run-log.jsonl");
    cut_final_byte(&run_log);
    let mut provider = SessionProvider::default();

    let output = resume_conversation_run_with_provider(
        &fixture.workspace,
        "conversation",
        "run",
        EmitMode::Human,
        || Ok(fixture.execution_fixture.credential().clone()),
        &mut provider,
    )
    .expect("exact Resume rolls back the cut intent and resumes the Run");

    assert!(!output.failed);
    assert_eq!(
        provider.calls, 1,
        "the rolled-back attempt never started, so it is issued once"
    );
}

#[test]
fn exact_productive_resume_rolls_back_a_cut_history_append_and_completes_the_run() {
    let fixture = interrupted_before_terminal_snapshot();
    set_status_transaction_crash_point(StatusTransactionCrashPoint::CanonicalMutationApplied);
    resume_exact_run(&fixture).expect_err("the crash point leaves the history append recorded");
    cut_final_byte(&conversation_history_path(&fixture));

    assert_eq!(
        resume_exact_run(&fixture).map_err(|error| error.to_string()),
        Ok(false)
    );
    assert_eq!(
        read_conversation_history(&fixture.workspace, "conversation")
            .expect("conversation history")
            .len(),
        1,
        "the rolled-back entry is appended again exactly once"
    );
}

#[test]
fn exact_productive_resume_refuses_a_cut_history_tail_without_a_recorded_transaction() {
    let fixture = interrupted_before_terminal_snapshot();
    assert_eq!(
        resume_exact_run(&fixture).map_err(|error| error.to_string()),
        Ok(false)
    );
    let history = conversation_history_path(&fixture);
    fs::OpenOptions::new()
        .append(true)
        .open(&history)
        .expect("history opens")
        .write_all(b"{\"entry_id\"")
        .expect("unrecorded torn tail writes");
    let before = fs::read(&history).expect("history reads before Resume");

    let error = resume_exact_run(&fixture).expect_err("an unrecorded torn tail fails closed");

    assert_eq!(error.exit_code(), 65);
    assert!(error.to_string().contains("LF framing"), "{error}");
    assert_eq!(
        fs::read(&history).expect("history reads after Resume"),
        before,
        "refused bytes are left in place"
    );
}
