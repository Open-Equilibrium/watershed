use super::productive_run::execute_reserved_productive_recovery;
use super::{
    RecordedProductivePreflight, prepare_productive_tool_executor,
    prepare_recorded_productive_preflight, reconcile_productive_preflight,
};
use crate::runtime::{
    auth::resolve_openai_codex_credential,
    config_io::{ExecutionBackend, load_global_config_authority, require_execution_backend},
    conversations::{read_conversation_recovery_definition, reserve_conversation_run_recovery},
    fs_guards::AnchoredWorkspace,
    live_events::LiveEventNotifier,
    openai_codex::OPENAI_CODEX_PROVIDER_ID,
    productive::{OpenAiCodexProvider, ProductiveProvider, ensure_productive_execution_platform},
    stage_results::reconcile_controlled_stages,
    types::{EmitMode, RunOutput, RuntimeError},
};
use std::path::Path;

/// Resumes one exactly addressed productive run without repeating completed external work.
pub fn resume_conversation_run(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let mut provider = OpenAiCodexProvider;
    resume_conversation_run_internal(
        workspace,
        conversation_id,
        run_session_id,
        None,
        emit == EmitMode::Jsonl,
        |_| Ok(()),
        ensure_productive_execution_platform,
        &mut provider,
        resolve_openai_codex_credential,
    )
}

/// Resumes one exactly addressed productive run with committed-event notifications.
pub fn resume_conversation_run_with_live_events(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut provider = OpenAiCodexProvider;
    let mut output = resume_conversation_run_internal(
        workspace,
        conversation_id,
        run_session_id,
        Some(notifier),
        false,
        |_| Ok(()),
        ensure_productive_execution_platform,
        &mut provider,
        resolve_openai_codex_credential,
    )?;
    output.stdout.clear();
    Ok(output)
}

/// Resumes a productive run after binding a guard to its loaded backend.
pub fn resume_conversation_run_with_execution_activation<G>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    notifier: Option<LiveEventNotifier>,
    emit: EmitMode,
    activate: impl FnOnce(bool) -> Result<G, RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    let live = notifier.is_some();
    let mut provider = OpenAiCodexProvider;
    let mut output = resume_conversation_run_internal(
        workspace,
        conversation_id,
        run_session_id,
        notifier,
        emit == EmitMode::Jsonl && !live,
        activate,
        ensure_productive_execution_platform,
        &mut provider,
        resolve_openai_codex_credential,
    )?;
    if live {
        output.stdout.clear();
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn resume_conversation_run_internal<P, C, A, G>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
    activate: A,
    platform_preflight: fn() -> Result<(), RuntimeError>,
    provider: &mut P,
    resolve_credential: C,
) -> Result<RunOutput, RuntimeError>
where
    P: ProductiveProvider,
    C: FnOnce() -> Result<crate::runtime::oauth_credential::CredentialRecord, RuntimeError>,
    A: FnOnce(bool) -> Result<G, RuntimeError>,
{
    if !proto::is_valid_session_id(conversation_id) || !proto::is_valid_session_id(run_session_id) {
        return Err(RuntimeError::Usage(
            "invalid conversation or run id".to_owned(),
        ));
    }
    let workspace = workspace.as_ref();
    let execution_workspace = reconcile_productive_preflight(AnchoredWorkspace::open(workspace))?;
    let authority = reconcile_productive_preflight(load_global_config_authority())?;
    let config = &authority.config;
    let backend = reconcile_productive_preflight(require_execution_backend(config))?;
    let _activation = activate(matches!(&backend, ExecutionBackend::OpenAiCodex { .. }))?;
    let ExecutionBackend::OpenAiCodex {
        model,
        model_profile,
    } = backend
    else {
        return Err(RuntimeError::Usage(format!(
            "productive run recovery requires provider {OPENAI_CODEX_PROVIDER_ID}"
        )));
    };
    reconcile_productive_preflight(platform_preflight())?;
    let recorded_definition = reconcile_productive_preflight(
        read_conversation_recovery_definition(workspace, conversation_id, run_session_id),
    )?;
    let RecordedProductivePreflight {
        registry,
        flow_ref,
        policy,
        agent_instructions,
    } = prepare_recorded_productive_preflight(
        &execution_workspace,
        &authority,
        run_session_id,
        &recorded_definition,
        "productive run recovery lacks a Flow id",
        &model,
        model_profile,
    )?;
    let mut tool_executor = prepare_productive_tool_executor(&policy)?;
    let credential = reconcile_productive_preflight(resolve_credential())?;
    let reservation = reconcile_productive_preflight(reserve_conversation_run_recovery(
        workspace,
        conversation_id,
        run_session_id,
    ))?;
    let operation = (|| {
        if reservation.recorded_definition() != Some(&recorded_definition) {
            return Err(RuntimeError::Protocol(
                "productive recovery definition changed during preflight".to_owned(),
            ));
        }
        let flow_block = registry
            .flow_block(&flow_ref)
            .expect("recorded productive preflight verified the Flow");
        execute_reserved_productive_recovery(
            workspace,
            &execution_workspace,
            &model,
            model_profile,
            &registry,
            flow_block,
            &policy,
            capture_jsonl,
            &credential,
            &agent_instructions,
            provider,
            &mut tool_executor,
            &reservation,
            notifier,
        )
    })();
    let cleanup = reservation.release();
    reconcile_controlled_stages(operation, Ok(()), cleanup)
}

#[cfg(test)]
pub(crate) fn resume_conversation_run_with_provider<P, C>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    emit: EmitMode,
    resolve_credential: C,
    provider: &mut P,
) -> Result<RunOutput, RuntimeError>
where
    P: ProductiveProvider,
    C: FnOnce() -> Result<crate::runtime::oauth_credential::CredentialRecord, RuntimeError>,
{
    resume_conversation_run_internal(
        workspace,
        conversation_id,
        run_session_id,
        None,
        emit == EmitMode::Jsonl,
        |_| Ok(()),
        || Ok(()),
        provider,
        resolve_credential,
    )
}

#[cfg(test)]
pub(crate) fn resume_conversation_run_with_provider_and_preflight<P: ProductiveProvider>(
    workspace: impl AsRef<Path>,
    platform_preflight: fn() -> Result<(), RuntimeError>,
    provider: &mut P,
) -> Result<RunOutput, RuntimeError> {
    resume_conversation_run_internal(
        workspace,
        "conversation",
        "run",
        None,
        false,
        |_| {
            crate::runtime::cancellation::begin_productive_operation()?;
            Ok(())
        },
        platform_preflight,
        provider,
        || {
            Err(RuntimeError::Protocol(
                "credential lookup must not run".to_owned(),
            ))
        },
    )
}

#[cfg(test)]
pub(crate) fn resume_conversation_run_with_provider_and_live_events<P: ProductiveProvider>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    provider: &mut P,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut output = resume_conversation_run_internal(
        workspace,
        conversation_id,
        run_session_id,
        Some(notifier),
        false,
        |_| Ok(()),
        || Ok(()),
        provider,
        || Ok(credential.clone()),
    )?;
    output.stdout.clear();
    Ok(output)
}
