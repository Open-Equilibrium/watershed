use super::productive_run::execute_reserved_productive_session;
use super::{
    RecordedProductivePreflight, prepare_productive_tool_executor,
    prepare_recorded_productive_preflight, reconcile_productive_preflight,
};
use crate::runtime::{
    auth::resolve_openai_codex_credential,
    config_io::{ExecutionBackend, load_global_config_authority, require_execution_backend},
    conversations::{read_conversation_continuation_definition, reserve_conversation_continuation},
    fs_guards::AnchoredWorkspace,
    live_events::LiveEventNotifier,
    openai_codex::OPENAI_CODEX_PROVIDER_ID,
    productive::{OpenAiCodexProvider, ProductiveProvider, ensure_productive_execution_platform},
    stage_results::reconcile_controlled_stages,
    types::{EmitMode, RunOutput, RuntimeError},
};
use std::path::Path;

/// Continues the latest committed entry, or branches from one explicitly selected entry.
pub fn continue_conversation(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    from_entry_id: Option<&str>,
    root_input: Option<core_script::FlowValue>,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    continue_conversation_internal(
        workspace,
        conversation_id,
        from_entry_id,
        root_input,
        None,
        emit == EmitMode::Jsonl,
    )
}

/// Continues a conversation with bounded committed-event notifications.
pub fn continue_conversation_with_live_events(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    from_entry_id: Option<&str>,
    root_input: Option<core_script::FlowValue>,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut output = continue_conversation_internal(
        workspace,
        conversation_id,
        from_entry_id,
        root_input,
        Some(notifier),
        false,
    )?;
    output.stdout.clear();
    Ok(output)
}

/// Continues a conversation after binding a guard to its loaded productive backend.
pub fn continue_conversation_with_execution_activation<G>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    from_entry_id: Option<&str>,
    root_input: Option<core_script::FlowValue>,
    notifier: Option<LiveEventNotifier>,
    emit: EmitMode,
    activate: impl FnOnce(bool) -> Result<G, RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    let live = notifier.is_some();
    let mut provider = OpenAiCodexProvider;
    let mut output = continue_conversation_internal_with_provider(
        workspace,
        conversation_id,
        from_entry_id,
        root_input,
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

fn continue_conversation_internal(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    from_entry_id: Option<&str>,
    root_input: Option<core_script::FlowValue>,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    let mut provider = OpenAiCodexProvider;
    continue_conversation_internal_with_provider(
        workspace,
        conversation_id,
        from_entry_id,
        root_input,
        notifier,
        capture_jsonl,
        |_| Ok(()),
        ensure_productive_execution_platform,
        &mut provider,
        resolve_openai_codex_credential,
    )
}

#[allow(clippy::too_many_arguments)]
fn continue_conversation_internal_with_provider<P, C, A, G>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    from_entry_id: Option<&str>,
    root_input: Option<core_script::FlowValue>,
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
    if !proto::is_valid_session_id(conversation_id)
        || from_entry_id.is_some_and(|entry_id| !proto::is_valid_session_id(entry_id))
    {
        return Err(RuntimeError::Usage(
            "invalid conversation or entry id".to_owned(),
        ));
    }
    let workspace = workspace.as_ref();
    let execution_workspace = AnchoredWorkspace::open(workspace)?;
    let authority = load_global_config_authority()?;
    let config = &authority.config;
    let ExecutionBackend::OpenAiCodex {
        model,
        model_profile,
    } = require_execution_backend(config)?
    else {
        return Err(RuntimeError::Usage(format!(
            "conversation continuation requires provider {OPENAI_CODEX_PROVIDER_ID}"
        )));
    };
    let _activation = activate(true)?;
    reconcile_productive_preflight(platform_preflight())?;
    let (recorded_run_session_id, recorded_definition) = reconcile_productive_preflight(
        read_conversation_continuation_definition(workspace, conversation_id, from_entry_id),
    )?;
    let RecordedProductivePreflight {
        registry,
        flow_ref,
        policy,
        credential,
        agent_instructions,
    } = prepare_recorded_productive_preflight(
        &execution_workspace,
        &authority,
        &recorded_run_session_id,
        &recorded_definition,
        "continuation lacks Flow id",
        &model,
        model_profile,
        resolve_credential,
    )?;
    let mut tool_executor = prepare_productive_tool_executor(&policy)?;
    let reservation = reconcile_productive_preflight(reserve_conversation_continuation(
        workspace,
        conversation_id,
        from_entry_id,
    ))?;
    let operation = (|| {
        if reservation.recorded_definition() != Some(&recorded_definition) {
            return Err(RuntimeError::Protocol(
                "continuation definition changed during preflight".to_owned(),
            ));
        }
        let flow_block = registry
            .flow_block(&flow_ref)
            .expect("recorded productive preflight verified the Flow");
        execute_reserved_productive_session(
            workspace,
            &execution_workspace,
            config,
            &model,
            model_profile,
            &registry,
            flow_block,
            &policy,
            root_input,
            capture_jsonl,
            &credential,
            &agent_instructions,
            notifier,
            provider,
            &mut tool_executor,
            &reservation,
        )
    })();
    let cleanup = reservation.release();
    reconcile_controlled_stages(operation, Ok(()), cleanup)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn continue_conversation_with_provider<P: ProductiveProvider>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    from_entry_id: Option<&str>,
    root_input: Option<core_script::FlowValue>,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    provider: &mut P,
) -> Result<RunOutput, RuntimeError> {
    continue_conversation_internal_with_provider(
        workspace,
        conversation_id,
        from_entry_id,
        root_input,
        notifier,
        capture_jsonl,
        |_| Ok(()),
        || Ok(()),
        provider,
        || Ok(credential.clone()),
    )
}
