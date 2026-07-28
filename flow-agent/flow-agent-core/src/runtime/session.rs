use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_anchored_workspace, preflight_flow_execution_plan},
    config_io::{load_workspace_config_from, require_fixture_execution_backend},
    event_writer::{EventWriterTimings, SerialSessionWriter},
    fs_guards::{AnchoredFile, AnchoredWorkspace},
    live_events::LiveEventNotifier,
    planning::{
        FlowExecutionOptions, ToolSideEffectMode, plan_flow_with_workspace, runtime_policy_target,
    },
    resume::session_definition_metadata,
    session_reservation::{
        materialize_session_candidate, reserve_unique_session_candidate_with_anchored_workspace,
        write_reserved_session_metadata,
    },
    types::{EmitMode, RunOutput, RuntimeError},
};
#[cfg(test)]
use std::cell::RefCell;
use std::path::Path;

#[cfg(test)]
type PostWriterFinishObserver = Box<dyn FnOnce(&AnchoredFile)>;

#[cfg(test)]
std::thread_local! {
    static POST_WRITER_FINISH_OBSERVER: RefCell<Option<PostWriterFinishObserver>> = RefCell::new(None);
    static RUN_POST_CONFIG_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static RUN_PRE_PLAN_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_post_writer_finish_observer(observer: impl FnOnce(&AnchoredFile) + 'static) {
    POST_WRITER_FINISH_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
pub(crate) fn set_run_post_config_observer(observer: impl FnOnce() + 'static) {
    RUN_POST_CONFIG_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(all(test, any(unix, windows)))]
pub(crate) fn set_run_pre_plan_observer(observer: impl FnOnce() + 'static) {
    RUN_PRE_PLAN_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn run_pre_plan_observer() {
    if let Some(observer) = RUN_PRE_PLAN_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[cfg(test)]
pub(crate) fn post_writer_finish_observer(path: &AnchoredFile) {
    if let Some(observer) = POST_WRITER_FINISH_OBSERVER.with_borrow_mut(Option::take) {
        observer(path);
    }
}

#[cfg(test)]
fn run_post_config_observer() {
    if let Some(observer) = RUN_POST_CONFIG_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

pub(crate) fn reconcile_controlled_stages<T>(
    operation: Result<T, RuntimeError>,
    finalization: Result<(), RuntimeError>,
    cleanup: Result<(), RuntimeError>,
) -> Result<T, RuntimeError> {
    match (operation, finalization, cleanup) {
        (Ok(value), Ok(()), Ok(())) => Ok(value),
        (Err(error), Ok(()), Ok(()))
        | (Ok(_), Err(error), Ok(()))
        | (Ok(_), Ok(()), Err(error)) => Err(error),
        (operation, finalization, cleanup) => Err(RuntimeError::ControlledStageFailures {
            operation: operation.err().map(Box::new),
            finalization: finalization.err().map(Box::new),
            cleanup: cleanup.err().map(Box::new),
        }),
    }
}

/// Runs a flow from a workspace registry and captures its output.
pub fn run_flow(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal(workspace, flow_ref, None, None, emit == EmitMode::Jsonl)
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
    let mut output = run_flow_internal(workspace, flow_ref, Some(notifier), None, false)?;
    output.stdout.clear();
    Ok(output)
}

pub fn run_flow_internal(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
) -> Result<RunOutput, RuntimeError> {
    run_flow_internal_with_cleanup_observer_impl(
        workspace,
        flow_ref,
        notifier,
        timings,
        capture_jsonl,
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
        (after_operation, after_finalization),
        before_cleanup,
    )
}

fn run_flow_internal_with_cleanup_observer_impl(
    workspace: impl AsRef<Path>,
    flow_ref: &str,
    notifier: Option<LiveEventNotifier>,
    timings: Option<&mut EventWriterTimings>,
    capture_jsonl: bool,
    stage_observers: (
        impl FnOnce(Result<RunOutput, RuntimeError>) -> Result<RunOutput, RuntimeError>,
        impl FnOnce(Result<(), RuntimeError>) -> Result<(), RuntimeError>,
    ),
    before_cleanup: impl FnOnce(&AnchoredFile),
) -> Result<RunOutput, RuntimeError> {
    let (after_operation, after_finalization) = stage_observers;
    let workspace = workspace.as_ref();
    let execution_workspace = AnchoredWorkspace::open(workspace)?;
    let config = load_workspace_config_from(execution_workspace.root())?;
    #[cfg(test)]
    run_post_config_observer();
    require_fixture_execution_backend(&config)?;
    let registry = core_script::load_flow_registry_from_workspace_dir(
        &execution_workspace.root().dir,
        workspace,
        &config.registry_root,
        flow_ref,
    )?;
    let flow_block = registry
        .flow_block(flow_ref)
        .ok_or_else(|| RuntimeError::Usage(format!("unknown flow {flow_ref}")))?;
    let definition_metadata = session_definition_metadata(&registry, flow_block)?;
    let policy =
        core_policy::compile_policy_artifact(&registry, flow_ref, runtime_policy_target())?;
    let base_session_id = &flow_block.identity.id;
    let candidate = reserve_unique_session_candidate_with_anchored_workspace(
        &execution_workspace,
        base_session_id,
    )?;
    let expected_session_id = candidate.session_id.clone();
    #[cfg(test)]
    run_pre_plan_observer();
    let plan = plan_flow_with_workspace(
        &execution_workspace,
        &registry,
        &policy,
        flow_block,
        &expected_session_id,
        FlowExecutionOptions::with_stub_model_fixture_profile(
            config.event_clock,
            ToolSideEffectMode::Plan,
            config.stub_model_fixture_profile,
        ),
    )?;
    preflight_flow_execution_plan(&plan, &execution_workspace, ToolSideEffectMode::Apply)?;
    let reservation = materialize_session_candidate(&execution_workspace, candidate)?;
    let mut finalization_result = Ok(());
    let operation_result = (|| {
        write_reserved_session_metadata(&reservation, Some(&definition_metadata))?;
        let mut serial_writer = SerialSessionWriter::start(&reservation, notifier, timings)?;
        if capture_jsonl {
            serial_writer.enable_jsonl_capture();
        }
        let runtime_result = apply_flow_with_anchored_workspace(
            FlowApplication {
                #[cfg(test)]
                workspace,
                session_id: &expected_session_id,
                options: FlowExecutionOptions::with_stub_model_fixture_profile(
                    config.event_clock,
                    ToolSideEffectMode::Apply,
                    config.stub_model_fixture_profile,
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
