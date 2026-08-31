use super::{prepare_productive_tool_executor, reconcile_productive_preflight};
#[cfg(test)]
use super::{
    productive_pre_run_create_observer, productive_pre_run_publish_observer,
    productive_run_commit_observer,
};
use crate::runtime::{
    cancellation::{
        claim_productive_effect_dispatch, claim_productive_run_creation_publication,
        ensure_productive_dispatch_allowed,
    },
    conversations::{
        ConversationAttemptLog, ConversationEventWriter, ProductiveRecoveryWriter, RUN_EVENTS_LEAF,
        append_productive_run_checkpoint,
        create_unpublished_productive_conversation_run_with_model_profile, existing_anchored_run,
        reclaim_productive_run_creation, reclaim_unpublished_productive_run,
        reserve_new_conversation_run,
    },
    execution_plan::RuntimeExecution,
    fs_guards::AnchoredWorkspace,
    live_events::LiveEventNotifier,
    productive::{
        ProductiveExecution, ProductiveProvider, ProductiveToolExecutor,
        execute_productive_flow_with_tool_executor_and_recovery,
    },
    run_attempts::ProductiveRecovery,
    session_definition::session_definition_metadata,
    stage_results::reconcile_controlled_stages,
    types::{RunOutput, RuntimeError},
};
use std::path::Path;

struct ProductiveRunFinalization<'a> {
    capture_jsonl: bool,
    flow_id: &'a str,
    recovering: bool,
    reservation: &'a crate::runtime::conversations::ProductiveConversationReservation,
    workspace: &'a Path,
}

fn finalize_productive_run(
    finalization: ProductiveRunFinalization<'_>,
    runtime_result: Result<RuntimeExecution, RuntimeError>,
    writer: &mut ConversationEventWriter,
    recovery: &ProductiveRecoveryWriter,
) -> Result<RunOutput, RuntimeError> {
    let finish_result = writer.finish();
    let runtime = reconcile_controlled_stages(runtime_result, finish_result, Ok(()))?;
    let (missing_snapshot, missing_events) = if finalization.recovering {
        (
            "productive run recovery emitted no terminal snapshot",
            "productive run recovery emitted no events",
        )
    } else {
        (
            "productive run emitted no terminal recovery snapshot",
            "productive run emitted no events",
        )
    };
    let recovery_snapshot_hash = recovery
        .terminal_snapshot_hash()
        .ok_or_else(|| RuntimeError::Protocol(missing_snapshot.to_owned()))?;
    let (sequence, timestamp) = writer
        .last_checkpoint()
        .ok_or_else(|| RuntimeError::Protocol(missing_events.to_owned()))?;
    let conversation_id = finalization.reservation.conversation_id();
    let run_session_id = finalization.reservation.run_session_id();
    append_productive_run_checkpoint(
        finalization.workspace,
        conversation_id,
        run_session_id,
        finalization.reservation.parent_entry_id(),
        recovery_snapshot_hash,
        sequence,
        timestamp,
    )?;
    let stdout = if finalization.capture_jsonl {
        writer.captured_jsonl().unwrap_or_default().to_owned()
    } else {
        let outcome = if runtime.failed {
            "failed"
        } else {
            "completed"
        };
        format!(
            "flow {} (conversation {conversation_id}, run {run_session_id}) {outcome}\n",
            finalization.flow_id
        )
    };
    let output = RunOutput {
        event_count: writer.event_count(),
        failed: runtime.failed,
        session_id: run_session_id.to_owned(),
        session_path: existing_anchored_run(
            finalization.workspace,
            conversation_id,
            run_session_id,
        )?
        .file(RUN_EVENTS_LEAF)
        .diagnostic_path()
        .to_owned(),
        stdout,
    };
    if let Some(error) = runtime.terminal_error {
        return Err(RuntimeError::session_failed(run_session_id, error));
    }
    Ok(output)
}

fn execute_and_finalize_productive_run<P, T>(
    finalization: ProductiveRunFinalization<'_>,
    execution: ProductiveExecution<'_>,
    provider: &mut P,
    tool_executor: &mut T,
    mut writer: ConversationEventWriter,
    mut recovery: ProductiveRecoveryWriter,
) -> Result<RunOutput, RuntimeError>
where
    P: ProductiveProvider,
    T: ProductiveToolExecutor,
{
    let mut attempts = ConversationAttemptLog::open_with_run_objects(
        finalization.workspace,
        finalization.reservation.conversation_id(),
        finalization.reservation.run_session_id(),
        recovery.run_objects(),
    )?;
    let runtime_result = execute_productive_flow_with_tool_executor_and_recovery(
        execution,
        provider,
        &mut attempts,
        &mut writer,
        tool_executor,
        &mut recovery,
    );
    finalize_productive_run(finalization, runtime_result, &mut writer, &recovery)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_productive_session_with_provider<P: ProductiveProvider>(
    workspace: &Path,
    execution_workspace: &AnchoredWorkspace,
    config: &crate::runtime::config_io::GlobalConfig,
    model: &str,
    model_profile: crate::runtime::context::ContextModelProfile,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
    policy: &core_policy::PolicyArtifact,
    root_input: Option<core_script::FlowValue>,
    capture_jsonl: bool,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    agent_instructions: &str,
    notifier: Option<LiveEventNotifier>,
    provider: &mut P,
) -> Result<RunOutput, RuntimeError> {
    let mut tool_executor = prepare_productive_tool_executor(policy)?;
    let reservation = reconcile_productive_preflight(reserve_new_conversation_run(
        workspace,
        &flow_block.identity.id,
    ))?;
    let operation = execute_reserved_productive_session(
        workspace,
        execution_workspace,
        config,
        model,
        model_profile,
        registry,
        flow_block,
        policy,
        root_input,
        capture_jsonl,
        credential,
        agent_instructions,
        notifier,
        provider,
        &mut tool_executor,
        &reservation,
    );
    let cleanup = reservation.release();
    reconcile_controlled_stages(operation, Ok(()), cleanup)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_reserved_productive_session<P: ProductiveProvider>(
    workspace: &Path,
    execution_workspace: &AnchoredWorkspace,
    config: &crate::runtime::config_io::GlobalConfig,
    model: &str,
    model_profile: crate::runtime::context::ContextModelProfile,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
    policy: &core_policy::PolicyArtifact,
    root_input: Option<core_script::FlowValue>,
    capture_jsonl: bool,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    agent_instructions: &str,
    notifier: Option<LiveEventNotifier>,
    provider: &mut P,
    tool_executor: &mut Option<crate::runtime::executor::PreparedExecutor>,
    reservation: &crate::runtime::conversations::ProductiveConversationReservation,
) -> Result<RunOutput, RuntimeError> {
    let definition =
        reconcile_productive_preflight(session_definition_metadata(registry, flow_block))?;
    let conversation_id = reservation.conversation_id().to_owned();
    let run_session_id = reservation.run_session_id().to_owned();
    (|| {
        #[cfg(test)]
        productive_pre_run_create_observer();
        let creation_dispatch = claim_productive_effect_dispatch()?;
        let creation = create_unpublished_productive_conversation_run_with_model_profile(
            workspace,
            &conversation_id,
            &run_session_id,
            &definition.flow_definition_id,
            &definition.registry_hash,
            &definition.flow_definition_hash,
            Some((model, model_profile)),
        );
        drop(creation_dispatch);
        if let Err(RuntimeError::Cancelled) = ensure_productive_dispatch_allowed() {
            let rollback = if creation.is_ok() {
                reclaim_unpublished_productive_run(workspace, &conversation_id, &run_session_id)
            } else {
                Ok(())
            };
            return reconcile_controlled_stages(Err(RuntimeError::Cancelled), Ok(()), rollback);
        }
        creation?;
        let publication = match claim_productive_run_creation_publication() {
            Ok(publication) => publication,
            Err(error) => {
                return reconcile_controlled_stages(
                    Err(error),
                    Ok(()),
                    reclaim_unpublished_productive_run(
                        workspace,
                        &conversation_id,
                        &run_session_id,
                    ),
                );
            }
        };
        #[cfg(test)]
        productive_pre_run_publish_observer();
        let prepared = match ProductiveRecoveryWriter::prepare(
            workspace,
            &conversation_id,
            &run_session_id,
            &definition.flow_definition_id,
            &definition.registry_hash,
            &definition.flow_definition_hash,
            root_input.as_ref(),
            reservation.parent_entry_id(),
            config.event_clock.base_unix_seconds,
            reservation.prior_history(),
            reservation.prior_event_count(),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                drop(publication);
                return reconcile_controlled_stages(
                    reconcile_productive_preflight::<RunOutput, _>(Err(error)),
                    Ok(()),
                    reclaim_unpublished_productive_run(
                        workspace,
                        &conversation_id,
                        &run_session_id,
                    ),
                );
            }
        };
        let expected_recovery_header = prepared.header().clone();
        let publication = match publication.commit() {
            Ok(publication) => publication,
            Err(error) => {
                drop(prepared);
                return reconcile_controlled_stages(
                    Err(error),
                    Ok(()),
                    reclaim_productive_run_creation(
                        workspace,
                        &conversation_id,
                        &run_session_id,
                        &expected_recovery_header,
                    ),
                );
            }
        };
        #[cfg(test)]
        productive_run_commit_observer();
        let recovery = match prepared.publish() {
            Ok(recovery) => recovery,
            Err(error) => {
                drop(publication);
                return reconcile_controlled_stages(
                    reconcile_productive_preflight::<RunOutput, _>(Err(error)),
                    Ok(()),
                    reclaim_productive_run_creation(
                        workspace,
                        &conversation_id,
                        &run_session_id,
                        &expected_recovery_header,
                    ),
                );
            }
        };
        if let Err(error) = publication.finish() {
            drop(recovery);
            return reconcile_controlled_stages(
                Err(error),
                Ok(()),
                reclaim_productive_run_creation(
                    workspace,
                    &conversation_id,
                    &run_session_id,
                    &expected_recovery_header,
                ),
            );
        }
        let run_objects = recovery.run_objects();
        let writer = ConversationEventWriter::open_with_run_objects(
            workspace,
            &conversation_id,
            &run_session_id,
            capture_jsonl,
            notifier,
            run_objects,
        )?;
        execute_and_finalize_productive_run(
            ProductiveRunFinalization {
                capture_jsonl,
                flow_id: &flow_block.identity.id,
                recovering: false,
                reservation,
                workspace,
            },
            ProductiveExecution {
                clock: config.event_clock,
                conversation_id: &conversation_id,
                credential,
                model,
                model_profile,
                policy,
                prior_history: reservation.prior_history().clone(),
                registry,
                agent_instructions,
                root_flow: flow_block,
                root_input,
                session_id: &run_session_id,
                workspace: execution_workspace,
            },
            provider,
            tool_executor,
            writer,
            recovery,
        )
    })()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_reserved_productive_recovery<P, T>(
    workspace: &Path,
    execution_workspace: &AnchoredWorkspace,
    model: &str,
    model_profile: crate::runtime::context::ContextModelProfile,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
    policy: &core_policy::PolicyArtifact,
    capture_jsonl: bool,
    credential: &crate::runtime::oauth_credential::CredentialRecord,
    agent_instructions: &str,
    provider: &mut P,
    tool_executor: &mut T,
    reservation: &crate::runtime::conversations::ProductiveConversationReservation,
    notifier: Option<LiveEventNotifier>,
) -> Result<RunOutput, RuntimeError>
where
    P: ProductiveProvider,
    T: ProductiveToolExecutor,
{
    let conversation_id = reservation.conversation_id().to_owned();
    let run_session_id = reservation.run_session_id().to_owned();
    let clock = reservation.recovery_event_clock().ok_or_else(|| {
        RuntimeError::Protocol("productive run recovery lacks its recorded Event clock".to_owned())
    })?;
    let recovery =
        ProductiveRecoveryWriter::open_for_resume(workspace, &conversation_id, &run_session_id)?;
    let run_objects = recovery.run_objects();
    let writer = ConversationEventWriter::open_for_recovery_with_run_objects(
        workspace,
        &conversation_id,
        &run_session_id,
        capture_jsonl,
        notifier,
        run_objects,
    )?;
    execute_and_finalize_productive_run(
        ProductiveRunFinalization {
            capture_jsonl,
            flow_id: &flow_block.identity.id,
            recovering: true,
            reservation,
            workspace,
        },
        ProductiveExecution {
            clock,
            conversation_id: &conversation_id,
            credential,
            model,
            model_profile,
            policy,
            prior_history: reservation.prior_history().clone(),
            registry,
            agent_instructions,
            root_flow: flow_block,
            root_input: reservation.recovery_root_input().cloned(),
            session_id: &run_session_id,
            workspace: execution_workspace,
        },
        provider,
        tool_executor,
        writer,
        recovery,
    )
}
