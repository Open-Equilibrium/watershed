use super::super::{
    helpers::{ProductiveExecutionFixture, disabled_configured_smoke_productive_execution_fixture},
    productive_recovery_support::{InterruptingProductiveRecovery, ProductiveInterruptionPoint},
    support::run_isolated_test,
};
#[cfg(unix)]
use super::support::FakeToolExecutor;
use super::support::{
    FakeProvider, ScriptedProvider, UnsupportedToolExecutor,
    disabled_smoke_productive_execution_fixture, smoke_productive_execution_fixture,
};
use crate::runtime::{
    config_io::load_global_config,
    context::{
        CONTEXT_SAFETY_MARGIN, ContextHistory, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID,
    },
    conversations::{
        ConversationAttemptLog, ConversationEventWriter, ProductiveRecoveryWriter,
        conversation_status_page, create_conversation_run, inspect_run_attempts,
        read_conversation_history, reserve_conversation_run_recovery,
        set_productive_run_creation_observer,
    },
    live_events::live_event_channel,
    openai_codex::derive_prompt_cache_key,
    productive::{
        ProductiveExecution, ProductiveProvider, execute_productive_flow,
        execute_productive_flow_with_recovery,
    },
    session::{
        continue_conversation_with_provider, execute_reserved_productive_recovery,
        run_productive_session_with_provider, set_productive_executor_readiness_observer,
        set_productive_pre_run_create_observer, set_productive_pre_run_publish_observer,
        set_productive_run_commit_observer,
    },
    session_definition::{SessionDefinitionMetadata, session_definition_metadata},
    types::{RunOutput, RuntimeError},
};
#[cfg(unix)]
use crate::runtime::{
    openai_codex::{ProviderToolCall, ProviderTurn},
    productive::execute_productive_flow_with_tool_executor_and_recovery,
    run_attempts::RunAttemptKind,
};
#[cfg(unix)]
use crate::tests::helpers::configured_smoke_productive_execution_fixture;
use std::{collections::VecDeque, fs, path::Path};

fn open_productive_recovery_prefix(
    workspace: &Path,
    definition: &SessionDefinitionMetadata,
    event_clock_base_unix_seconds: i64,
    prior_history: &ContextHistory,
) -> (
    ProductiveRecoveryWriter,
    ConversationEventWriter,
    ConversationAttemptLog,
) {
    create_conversation_run(
        workspace,
        "conversation",
        "run",
        &definition.flow_definition_id,
        &definition.registry_hash,
        &definition.flow_definition_hash,
    )
    .expect("conversation run");
    let recovery = ProductiveRecoveryWriter::create(
        workspace,
        "conversation",
        "run",
        &definition.flow_definition_id,
        &definition.registry_hash,
        &definition.flow_definition_hash,
        None,
        None,
        event_clock_base_unix_seconds,
        prior_history,
        0,
    )
    .expect("recovery header");
    let writer = ConversationEventWriter::open(workspace, "conversation", "run", false)
        .expect("conversation event writer");
    let attempts = ConversationAttemptLog::open(workspace, "conversation", "run")
        .expect("conversation attempt log opens");
    (recovery, writer, attempts)
}

fn run_default_smoke_productive_session<P: ProductiveProvider>(
    workspace: &Path,
    fixture: &ProductiveExecutionFixture,
    provider: &mut P,
) -> Result<RunOutput, RuntimeError> {
    run_default_smoke_productive_session_with_credential_resolver(
        workspace,
        fixture,
        || Ok(fixture.credential().clone()),
        provider,
    )
}

fn run_default_smoke_productive_session_with_credential_resolver<P, C>(
    workspace: &Path,
    fixture: &ProductiveExecutionFixture,
    resolve_credential: C,
    provider: &mut P,
) -> Result<RunOutput, RuntimeError>
where
    P: ProductiveProvider,
    C: FnOnce() -> Result<crate::runtime::oauth_credential::CredentialRecord, RuntimeError>,
{
    let config = load_global_config()?;
    run_productive_session_with_provider(
        workspace,
        &fixture.anchored,
        &config,
        "gpt-fixture",
        ContextModelProfile::stub_v0(),
        &fixture.registry,
        fixture.smoke_flow(),
        &fixture.policy,
        None,
        false,
        resolve_credential,
        "",
        None,
        provider,
    )
}

#[test]
fn executor_readiness_failure_precedes_new_run_reservation() {
    let (workspace, fixture) = smoke_productive_execution_fixture();
    set_productive_executor_readiness_observer(|| {
        Err(RuntimeError::executor(
            proto::ExecutorErrorCodeV0::Unavailable,
            "injected Executor readiness failure",
        ))
    });
    let error = run_default_smoke_productive_session_with_credential_resolver(
        &workspace,
        &fixture,
        || panic!("credential resolution must follow Executor readiness"),
        &mut FakeProvider::default(),
    )
    .expect_err("failed readiness must stop before reservation");

    assert!(matches!(
        error,
        RuntimeError::Executor(ref failure)
            if failure.code() == proto::ExecutorErrorCodeV0::Unavailable
    ));
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("conversation")
            .exists(),
        "readiness failure must not create a durable conversation reservation"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_tool_preflight_fails_before_provider_dispatch_or_run_reservation() {
    let (workspace, fixture) = smoke_productive_execution_fixture();
    let mut provider = FakeProvider::default();

    let error = run_default_smoke_productive_session(&workspace, &fixture, &mut provider)
        .expect_err("macOS must reject productive Tool execution before reservation");

    assert!(matches!(
        error,
        RuntimeError::Executor(ref failure)
            if failure.code() == proto::ExecutorErrorCodeV0::PolicyUnsupported
    ));
    assert!(provider.bodies.is_empty(), "the provider must not dispatch");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("conversation")
            .exists(),
        "platform rejection must not create a durable conversation reservation"
    );
}

#[test]
fn provider_only_new_run_does_not_probe_an_executor() {
    let (workspace, fixture) = disabled_smoke_productive_execution_fixture();
    set_productive_executor_readiness_observer(|| {
        panic!("provider-only policy must not inspect Executor readiness")
    });
    let output =
        run_default_smoke_productive_session(&workspace, &fixture, &mut FakeProvider::default())
            .expect("provider-only productive Run succeeds without an Executor");

    assert!(!output.failed);
}

fn recover_interrupted_productive_run<P, T>(
    workspace: &Path,
    execution: ProductiveExecution<'_>,
    provider: &mut P,
    tool_executor: &mut T,
) -> RunOutput
where
    P: ProductiveProvider,
    T: crate::runtime::productive::ProductiveToolExecutor,
{
    let reservation = reserve_conversation_run_recovery(workspace, "conversation", "run")
        .expect("interrupted run reserves for exact recovery");
    let output = execute_reserved_productive_recovery(
        workspace,
        execution.workspace,
        execution.model,
        execution.model_profile,
        execution.registry,
        execution.root_flow,
        execution.policy,
        false,
        execution.credential,
        execution.agent_instructions,
        provider,
        tool_executor,
        &reservation,
        None,
    )
    .expect("exact recovery completes");
    reservation
        .release()
        .expect("recovery reservation releases");
    output
}

#[test]
fn productive_run_persists_one_complete_conversation_bundle() {
    let (workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    create_conversation_run(
        &workspace,
        "conversation",
        "run",
        "smoke-flow",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("conversation run");
    let mut provider = FakeProvider::default();
    let mut attempts = ConversationAttemptLog::open(&workspace, "conversation", "run")
        .expect("conversation attempt log opens");
    let mut sink = ConversationEventWriter::open(&workspace, "conversation", "run", true)
        .expect("conversation event writer");

    let execution = execute_productive_flow(
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "run")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("productive execution");
    sink.finish().expect("writer finalization");

    assert!(!execution.failed, "{:?}", execution.terminal_error);
    let events = fs::read_to_string(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("conversation/runs/run/events.jsonl"),
    )
    .expect("events");
    assert!(events.contains("\"event_type\":\"session.completed\""));
    assert!(!events.contains("secret-access"));
    let contexts = fs::read_to_string(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("conversation/runs/run/contexts.jsonl"),
    )
    .expect("contexts");
    assert!(!contexts.is_empty());
    assert!(
        fs::read_dir(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join("conversation/runs/run/objects")
        )
        .expect("objects")
        .next()
        .is_some()
    );
    assert_eq!(
        inspect_run_attempts(&workspace, "conversation", "run")
            .expect("attempts")
            .len(),
        1
    );
}

#[test]
fn productive_session_entrypoint_persists_a_resumable_conversation() {
    let (workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
    let config = load_global_config().expect("productive config");
    let registry = &fixture.registry;
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider::default();
    let model_profile = ContextModelProfile {
        context_limit: 128000,
        id: OPERATOR_MODEL_PROFILE_ID,
        output_reserve: 16384,
        safety_margin: CONTEXT_SAFETY_MARGIN,
    };

    let output = run_productive_session_with_provider(
        &workspace,
        &fixture.anchored,
        &config,
        "gpt-fixture",
        model_profile,
        registry,
        flow,
        &fixture.policy,
        Some(core_script::FlowValue::String("root input".to_owned())),
        true,
        || Ok(fixture.credential().clone()),
        "Agent guidance.",
        None,
        &mut provider,
    )
    .expect("productive session completes");

    assert!(!output.failed);
    assert!(
        output
            .stdout
            .contains("\"event_type\":\"session.completed\"")
    );
    assert_eq!(provider.bodies.len(), 1);
    let page = conversation_status_page(&workspace, None).expect("conversation status");
    assert_eq!(page.conversations.len(), 1);
    assert!(page.conversations[0].latest_entry_id.is_some());
    assert_eq!(page.conversations[0].run_count, 1);
    assert_eq!(page.conversations[0].uncertain_attempts, 0);

    let conversation_id = page.conversations[0].conversation_id.clone();
    let run_log = fs::read_to_string(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join(&conversation_id)
            .join("runs")
            .join(&output.session_id)
            .join("run-log.jsonl"),
    )
    .expect("Run definition reads");
    assert!(run_log.contains("\"model\":\"gpt-fixture\""));
    assert!(run_log.contains("\"model_profile_id\":\"operator-model-v0\""));
    assert!(run_log.contains("\"model_context_limit\":128000"));
    assert!(run_log.contains("\"output_reserve\":16384"));
    let (notifier, _receiver) = live_event_channel();
    let mut continuation_provider = FakeProvider::default();
    let continuation = continue_conversation_with_provider(
        &workspace,
        &conversation_id,
        None,
        None,
        Some(notifier),
        false,
        || Ok(fixture.credential().clone()),
        &mut continuation_provider,
    )
    .expect("live continuation completes");
    assert!(
        continuation
            .stdout
            .starts_with("flow smoke-flow (conversation "),
        "notifier-backed execution must not retain canonical JSONL in memory"
    );
    assert_eq!(continuation_provider.bodies.len(), 1);
    assert_eq!(
        continuation_provider.bodies[0]["prompt_cache_key"],
        derive_prompt_cache_key(&conversation_id, "gpt-fixture")
    );
}

#[test]
fn cancellation_at_productive_run_creation_respects_publication_boundary() {
    const CHILD_ENV: &str = "WATERSHED_PRE_RUN_CREATE_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    for cancellation_point in [
        "before-creation",
        "during-creation",
        "before-publication",
        "during-publication",
    ] {
        crate::begin_productive_operation().expect("productive operation begins");
        let (workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
        let cancel = || {
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
        };
        match cancellation_point {
            "before-creation" => set_productive_pre_run_create_observer(cancel),
            "during-creation" => set_productive_run_creation_observer(cancel),
            "before-publication" => set_productive_pre_run_publish_observer(cancel),
            "during-publication" => set_productive_run_commit_observer(cancel),
            _ => unreachable!(),
        }
        let mut provider = FakeProvider::default();

        let error = run_default_smoke_productive_session(&workspace, &fixture, &mut provider)
            .expect_err("cancellation wins at the durable run creation boundary");
        assert!(provider.bodies.is_empty());
        let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
        assert!(
            !sessions.exists()
                || fs::read_dir(&sessions)
                    .expect("session root reads")
                    .next()
                    .is_none(),
            "cancellation at {cancellation_point} must leave no durable conversation or Run"
        );
        assert!(matches!(error, RuntimeError::Cancelled), "{error:?}");
        assert!(!crate::settle_productive_operation());
    }
}

#[test]
fn productive_session_human_output_reports_success_and_persists_failure() {
    let (success_workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
    let mut provider = FakeProvider::default();

    let output = run_default_smoke_productive_session(&success_workspace, &fixture, &mut provider)
        .expect("human productive session completes");

    assert!(!output.failed);
    assert!(output.stdout.starts_with("flow smoke-flow (conversation "));
    assert!(output.stdout.ends_with(") completed\n"));

    let (failure_workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::new(),
    };

    let error = run_default_smoke_productive_session(&failure_workspace, &fixture, &mut provider)
        .expect_err("provider failure is reported after durable closure");

    assert!(error.to_string().contains("session"));
    let page = conversation_status_page(&failure_workspace, None).expect("conversation status");
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(page.conversations[0].run_count, 1);
    assert_eq!(page.conversations[0].uncertain_attempts, 1);
}

#[test]
fn productive_recovery_reuses_committed_provider_attempt_without_redispatch() {
    let (workspace, fixture) = disabled_configured_smoke_productive_execution_fixture();
    let config = load_global_config().expect("productive config");
    let registry = &fixture.registry;
    let flow = fixture.smoke_flow();
    let definition = session_definition_metadata(registry, flow).expect("definition metadata");
    let prior_history = ContextHistory::default();
    let (mut recovery, mut writer, mut attempts) = open_productive_recovery_prefix(
        &workspace,
        &definition,
        config.event_clock.base_unix_seconds,
        &prior_history,
    );
    let mut initial_provider = FakeProvider::default();
    {
        let mut interruption =
            InterruptingProductiveRecovery::new(&mut recovery, ProductiveInterruptionPoint::Phase);
        let error = execute_productive_flow_with_recovery(
            ProductiveExecution {
                clock: config.event_clock,
                prior_history,
                agent_instructions: "Agent guidance.",
                ..fixture.execution(flow, "run")
            },
            &mut initial_provider,
            &mut attempts,
            &mut writer,
            &mut interruption,
        )
        .expect_err("fixture interruption leaves a recoverable prefix");
        assert!(
            error
                .to_string()
                .contains("interruption after committed provider result")
        );
    }
    writer.finish().expect("event prefix finalizes");
    drop(recovery);
    assert_eq!(initial_provider.bodies.len(), 1);

    let mut recovery_provider = FakeProvider::default();
    let mut tool_executor = UnsupportedToolExecutor;
    let output = recover_interrupted_productive_run(
        &workspace,
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "run")
        },
        &mut recovery_provider,
        &mut tool_executor,
    );

    assert!(!output.failed);
    assert!(
        recovery_provider.bodies.is_empty(),
        "provider must not rerun"
    );
    assert_eq!(
        inspect_run_attempts(&workspace, "conversation", "run")
            .expect("attempts")
            .len(),
        1
    );
    assert_eq!(
        read_conversation_history(&workspace, "conversation")
            .expect("history")
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn productive_recovery_reuses_committed_tool_attempt_without_redispatch() {
    let (workspace, fixture) = configured_smoke_productive_execution_fixture();
    let config = load_global_config().expect("productive config");
    let registry = &fixture.registry;
    let flow = fixture.smoke_flow();
    let definition = session_definition_metadata(registry, flow).expect("definition metadata");
    let prior_history = ContextHistory::default();
    let (mut recovery, mut writer, mut attempts) = open_productive_recovery_prefix(
        &workspace,
        &definition,
        config.event_clock.base_unix_seconds,
        &prior_history,
    );
    let tool_call = serde_json::json!({
        "arguments": "{}",
        "call_id": "call-1",
        "name": "echo",
        "type": "function_call",
    });
    let mut initial_provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([
            ProviderTurn {
                token_usage: None,
                response_id: "response-tool".to_owned(),
                output_text: String::new(),
                retained_items: vec![tool_call],
                tool_calls: vec![ProviderToolCall {
                    call_id: "call-1".to_owned(),
                    name: "echo".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            },
            ProviderTurn {
                token_usage: None,
                response_id: "response-final".to_owned(),
                output_text: "{\"type\":\"string\",\"value\":\"after-tool\"}".to_owned(),
                retained_items: Vec::new(),
                tool_calls: Vec::new(),
            },
        ]),
    };
    let mut tools = FakeToolExecutor::default();
    {
        let mut interruption =
            InterruptingProductiveRecovery::new(&mut recovery, ProductiveInterruptionPoint::Phase);
        execute_productive_flow_with_tool_executor_and_recovery(
            ProductiveExecution {
                clock: config.event_clock,
                prior_history,
                agent_instructions: "Agent guidance.",
                ..fixture.execution(flow, "run")
            },
            &mut initial_provider,
            &mut attempts,
            &mut writer,
            &mut tools,
            &mut interruption,
        )
        .expect_err("fixture interruption leaves a recoverable Tool attempt");
    }
    writer.finish().expect("event prefix finalizes");
    drop(recovery);
    assert_eq!(initial_provider.bodies.len(), 2);
    assert_eq!(tools.invocations.len(), 1);

    let mut recovery_provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::new(),
    };
    let output = recover_interrupted_productive_run(
        &workspace,
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "run")
        },
        &mut recovery_provider,
        &mut tools,
    );

    assert!(!output.failed);
    assert!(
        recovery_provider.bodies.is_empty(),
        "provider must not rerun"
    );
    assert_eq!(tools.invocations.len(), 1, "Tool must not rerun");
    let recovered_attempts =
        inspect_run_attempts(&workspace, "conversation", "run").expect("attempts");
    assert_eq!(recovered_attempts.len(), 3);
    assert_eq!(
        recovered_attempts
            .iter()
            .filter(|attempt| attempt.attempt_kind == RunAttemptKind::Tool)
            .count(),
        1
    );
}
