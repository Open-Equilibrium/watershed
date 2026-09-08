use super::productive_run::run_productive_session_with_provider;
use super::reconcile_productive_preflight;
#[cfg(test)]
use super::{run_post_config_observer, run_pre_plan_observer};
#[cfg(test)]
use crate::runtime::event_writer::post_writer_finish_observer;
use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_anchored_workspace, preflight_flow_execution_plan},
    auth::resolve_openai_codex_credential,
    config_io::{ExecutionBackend, load_global_config_authority, require_execution_backend},
    event_writer::SerialSessionWriter,
    execution_plan::{FlowExecutionOptions, ToolSideEffectMode},
    fs_guards::{AnchoredFile, AnchoredWorkspace},
    instructions::read_applicable_agent_instructions,
    live_events::LiveEventNotifier,
    planning::plan_flow_with_workspace,
    productive::{OpenAiCodexProvider, ensure_productive_execution_platform},
    session_definition::session_definition_metadata,
    session_reservation::{
        materialize_session_candidate, reserve_unique_session_candidate_with_anchored_workspace,
        write_reserved_session_metadata,
    },
    stage_results::reconcile_controlled_stages,
    types::{EmitMode, RunOutput, RuntimeError},
};
use std::path::Path;

/// Runs a globally registered Flow in one Workspace and captures its output.
pub fn run_flow(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_root_input(workspace, flow_ref, None, None, emit == EmitMode::Jsonl)
}

/// Runs a Flow with one validated typed selected-root-Flow input.
pub fn run_flow_with_root_input(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    root_input: core_script::FlowValue,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_root_input(
        workspace,
        flow_ref,
        Some(root_input),
        None,
        emit == EmitMode::Jsonl,
    )
}

/// Runs a flow with bounded, non-blocking committed-event notifications.
///
/// The caller owns the receiver and any blocking transport. Notifications carry only a
/// high-watermark wake-up; read event payloads from [`crate::SessionEventReader`] by sequence.
pub fn run_flow_with_live_events(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut output =
        run_flow_internal_with_root_input(workspace, flow_ref, None, Some(notifier), false)?;
    output.stdout.clear();
    Ok(output)
}

/// Runs a typed-input Flow with bounded committed-event notifications.
pub fn run_flow_with_root_input_and_live_events(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    root_input: core_script::FlowValue,
    notifier: LiveEventNotifier,
) -> Result<RunOutput, RuntimeError> {
    let mut output = run_flow_internal_with_root_input(
        workspace,
        flow_ref,
        Some(root_input),
        Some(notifier),
        false,
    )?;
    output.stdout.clear();
    Ok(output)
}

/// Runs a Flow after binding one caller-owned guard to the loaded execution backend.
pub fn run_flow_with_execution_activation<G>(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    root_input: Option<core_script::FlowValue>,
    notifier: Option<LiveEventNotifier>,
    emit: EmitMode,
    activate: impl FnOnce(bool) -> Result<G, RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    let live = notifier.is_some();
    let mut output = run_flow_internal_with_root_input_and_activation(
        workspace,
        flow_ref,
        root_input,
        notifier,
        emit == EmitMode::Jsonl && !live,
        activate,
    )?;
    if live {
        output.stdout.clear();
    }
    Ok(output)
}

fn run_flow_internal_with_root_input(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    root_input: Option<core_script::FlowValue>,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_root_input_and_activation(
        workspace,
        flow_ref,
        root_input,
        notifier,
        capture_jsonl,
        |_| Ok(()),
    )
}

fn run_flow_internal_with_root_input_and_activation<G>(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    root_input: Option<core_script::FlowValue>,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
    activate: impl FnOnce(bool) -> Result<G, RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_cleanup_observer_impl(
        workspace,
        flow_ref,
        root_input,
        notifier,
        capture_jsonl,
        activate,
        (|result| result, |result| result),
        |_| {},
    )
}

#[cfg(test)]
pub(crate) fn run_flow_internal_with_cleanup_observer(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    capture_jsonl: bool,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_cleanup_observer_impl(
        workspace,
        flow_ref,
        None,
        None,
        capture_jsonl,
        |_| Ok(()),
        (|result| result, |result| result),
        before_cleanup,
    )
}

#[cfg(test)]
pub(crate) fn run_flow_internal_with_stage_observers(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    capture_jsonl: bool,
    after_operation: impl FnOnce(Result<RunOutput, RuntimeError>) -> Result<RunOutput, RuntimeError>,
    after_finalization: impl FnOnce(Result<(), RuntimeError>) -> Result<(), RuntimeError>,
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_cleanup_observer_impl(
        workspace,
        flow_ref,
        None,
        None,
        capture_jsonl,
        |_| Ok(()),
        (after_operation, after_finalization),
        before_cleanup,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_flow_internal_with_cleanup_observer_impl<G>(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    root_input: Option<core_script::FlowValue>,
    notifier: Option<LiveEventNotifier>,
    capture_jsonl: bool,
    activate: impl FnOnce(bool) -> Result<G, RuntimeError>,
    stage_observers: (
        impl FnOnce(Result<RunOutput, RuntimeError>) -> Result<RunOutput, RuntimeError>,
        impl FnOnce(Result<(), RuntimeError>) -> Result<(), RuntimeError>,
    ),
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    let (after_operation, after_finalization) = stage_observers;
    let workspace = workspace.as_ref();
    let execution_workspace = AnchoredWorkspace::open(workspace)?;
    let authority = load_global_config_authority()?;
    let config = &authority.config;
    #[cfg(test)]
    run_post_config_observer();
    let backend = require_execution_backend(config)?;
    let _activation = activate(matches!(&backend, ExecutionBackend::OpenAiCodex { .. }))?;
    let registry = reconcile_productive_preflight(core_script::load_flow_registry_from_root_dir(
        &authority.home.dir,
        &authority.home.path,
        &config.registry_root,
        flow_ref,
    ))?;
    let flow_block = reconcile_productive_preflight(
        registry
            .flow_block(flow_ref)
            .ok_or_else(|| RuntimeError::Usage(format!("unknown flow {flow_ref}"))),
    )?;
    let definition_metadata =
        reconcile_productive_preflight(session_definition_metadata(&registry, flow_block))?;
    let policy =
        reconcile_productive_preflight(core_policy::compile_policy_artifact(&registry, flow_ref))?;
    if let ExecutionBackend::OpenAiCodex {
        model,
        model_profile,
    } = backend
    {
        reconcile_productive_preflight(ensure_productive_execution_platform())?;
        let agent_instructions = reconcile_productive_preflight(
            read_applicable_agent_instructions(&authority.home, &execution_workspace),
        )?;
        let mut provider = OpenAiCodexProvider;
        return run_productive_session_with_provider(
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
            resolve_openai_codex_credential,
            &agent_instructions,
            notifier,
            &mut provider,
        );
    }
    let base_session_id = &flow_block.identity.id;
    let candidate = reserve_unique_session_candidate_with_anchored_workspace(
        &execution_workspace,
        base_session_id,
    )?;
    let expected_session_id = candidate.session_id.clone();
    #[cfg(test)]
    run_pre_plan_observer();
    execution_workspace.verify_binding()?;
    let plan_options = flow_execution_options(
        config.event_clock,
        ToolSideEffectMode::Plan,
        config.stub_model_fixture_profile,
        root_input.as_ref(),
    );
    let plan = plan_flow_with_workspace(
        &execution_workspace,
        &registry,
        &policy,
        flow_block,
        &expected_session_id,
        plan_options,
    )?;
    preflight_flow_execution_plan(&plan, &execution_workspace, ToolSideEffectMode::Apply)?;
    let reservation = materialize_session_candidate(&execution_workspace, candidate)?;
    let mut finalization_result = Ok(());
    let operation_result = (|| {
        write_reserved_session_metadata(&reservation, Some(&definition_metadata))?;
        let mut serial_writer = SerialSessionWriter::start(&reservation, notifier)?;
        if capture_jsonl {
            serial_writer.enable_jsonl_capture();
        }
        let runtime_result = apply_flow_with_anchored_workspace(
            FlowApplication {
                #[cfg(test)]
                workspace,
                session_id: &expected_session_id,
                options: flow_execution_options(
                    config.event_clock,
                    ToolSideEffectMode::Apply,
                    config.stub_model_fixture_profile,
                    root_input.as_ref(),
                ),
                plan: &plan,
            },
            &execution_workspace,
            Some(&mut serial_writer),
        );
        finalization_result = serial_writer.finish();
        #[cfg(test)]
        post_writer_finish_observer(&reservation.session_path);
        let captured_jsonl = serial_writer.take_captured_jsonl();
        let runtime = runtime_result?;
        let runtime_failed = runtime.failed;
        let event_count = runtime.events.record_count;
        let outcome = runtime.failure_status.unwrap_or_else(|| {
            if runtime_failed {
                "failed"
            } else {
                "completed"
            }
            .to_owned()
        });
        if let Some(err) = runtime.terminal_error {
            return Err(RuntimeError::session_failed(&expected_session_id, err));
        }
        let stdout = if capture_jsonl {
            captured_jsonl.expect("JSONL capture enabled before runtime application")
        } else {
            format!(
                "flow {} (session {expected_session_id}) {outcome}\n",
                flow_block.identity.id
            )
        };
        Ok(RunOutput {
            event_count,
            failed: runtime_failed,
            session_id: expected_session_id,
            session_path: reservation.session_path.diagnostic_path().to_owned(),
            stdout,
        })
    })();
    let operation_result = after_operation(operation_result);
    finalization_result = after_finalization(finalization_result);
    before_cleanup(&reservation.lock_path);
    let cleanup_result = reservation.cleanup();
    reconcile_controlled_stages(operation_result, finalization_result, cleanup_result)
}

fn flow_execution_options(
    clock: crate::runtime::types::EventClock,
    side_effect_mode: ToolSideEffectMode,
    stub_model_fixture_profile: bool,
    root_input: Option<&core_script::FlowValue>,
) -> FlowExecutionOptions {
    let options = FlowExecutionOptions::with_stub_model_fixture_profile(
        clock,
        side_effect_mode,
        stub_model_fixture_profile,
    );
    match root_input {
        Some(input) => options.with_root_input(input.clone()),
        None => options,
    }
}
