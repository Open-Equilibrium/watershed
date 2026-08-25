use super::productive_run::execute_reserved_productive_recovery;
use super::{
    RecordedProductivePreflight, prepare_recorded_productive_preflight,
    reconcile_productive_preflight,
};
use crate::runtime::{
    auth::resolve_openai_codex_credential,
    config_io::{ExecutionBackend, load_global_config_authority, require_execution_backend},
    conversations::{
        RUN_EVENTS_LEAF, existing_anchored_run, legacy_flat_compatibility_is_available,
        migrate_legacy_session, migrate_legacy_session_if_present,
        reserve_conversation_run_recovery,
    },
    fs_guards::AnchoredWorkspace,
    live_events::LiveEventNotifier,
    openai_codex::OPENAI_CODEX_PROVIDER_ID,
    productive::{OpenAiCodexProvider, ProductiveProvider, ensure_productive_execution_platform},
    stage_results::reconcile_controlled_stages,
    types::{EmitMode, RunOutput, RuntimeError},
};
use std::path::Path;

fn resume_legacy_conversation_run_if_present(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
) -> Result<Option<RunOutput>, RuntimeError> {
    if conversation_id == run_session_id
        && legacy_flat_compatibility_is_available(workspace, run_session_id)?
    {
        let (mut output, migration) = if let Some(notifier) = notifier {
            let mut output = crate::runtime::resume::resume_migrating_conversation_run_internal(
                workspace,
                run_session_id,
                None,
                true,
            )?;
            let migration = migrate_legacy_session(workspace, run_session_id);
            announce_migrated_legacy_resume(
                &notifier,
                conversation_id,
                run_session_id,
                &output,
                migration.is_ok(),
            )?;
            output.stdout.clear();
            (output, migration)
        } else {
            (
                crate::runtime::resume::resume_migrating_conversation_run_internal(
                    workspace,
                    run_session_id,
                    None,
                    capture_jsonl,
                )?,
                migrate_legacy_session(workspace, run_session_id),
            )
        };
        migration?;
        output.session_path = existing_anchored_run(workspace, conversation_id, run_session_id)?
            .file(RUN_EVENTS_LEAF)
            .diagnostic_path()
            .to_owned();
        return Ok(Some(output));
    }
    if conversation_id == run_session_id {
        migrate_legacy_session_if_present(workspace, run_session_id)?;
    }
    Ok(None)
}

fn announce_migrated_legacy_resume(
    notifier: &LiveEventNotifier,
    conversation_id: &str,
    run_session_id: &str,
    output: &RunOutput,
    migrated: bool,
) -> Result<(), RuntimeError> {
    let appended = output.stdout.lines().count();
    if appended == 0 {
        return Ok(());
    }
    let first = output
        .event_count
        .checked_sub(appended)
        .and_then(|prefix| prefix.checked_add(1))
        .and_then(|sequence| u64::try_from(sequence).ok())
        .ok_or_else(|| RuntimeError::Protocol("legacy resume event range overflow".to_owned()))?;
    let last = u64::try_from(output.event_count)
        .map_err(|_| RuntimeError::Protocol("legacy resume event count overflow".to_owned()))?;
    if migrated {
        notifier.try_notify_conversation_run(conversation_id, run_session_id, first);
        notifier.try_notify_conversation_run(conversation_id, run_session_id, last);
    } else {
        notifier.try_notify(run_session_id, first);
        notifier.try_notify(run_session_id, last);
    }
    Ok(())
}

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
    let legacy_notifier = notifier
        .as_ref()
        .map(LiveEventNotifier::duplicate_for_same_operation);
    if let Some(output) = resume_legacy_conversation_run_if_present(
        workspace,
        conversation_id,
        run_session_id,
        legacy_notifier,
        capture_jsonl,
    )? {
        return Ok(output);
    }
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
    let reservation = reconcile_productive_preflight(reserve_conversation_run_recovery(
        workspace,
        conversation_id,
        run_session_id,
    ))?;
    let operation = (|| {
        let recorded =
            reconcile_productive_preflight(reservation.recorded_definition().ok_or_else(|| {
                RuntimeError::Protocol("productive run recovery lacks a definition".to_owned())
            }))?;
        let RecordedProductivePreflight {
            registry,
            flow_ref,
            policy,
            credential,
            agent_instructions,
        } = prepare_recorded_productive_preflight(
            &execution_workspace,
            &authority,
            run_session_id,
            recorded,
            "productive run recovery lacks a Flow id",
            &model,
            model_profile,
            resolve_credential,
        )?;
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
            &reservation,
            notifier,
        )
    })();
    let cleanup = reservation.release();
    reconcile_controlled_stages(operation, Ok(()), cleanup)
}

#[cfg(test)]
pub(crate) fn resume_conversation_run_with_provider<P: ProductiveProvider>(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    emit: EmitMode,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    provider: &mut P,
) -> Result<RunOutput, RuntimeError> {
    resume_conversation_run_internal(
        workspace,
        conversation_id,
        run_session_id,
        None,
        emit == EmitMode::Jsonl,
        |_| Ok(()),
        || Ok(()),
        provider,
        || Ok(credential.clone()),
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
