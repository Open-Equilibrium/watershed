use crate::runtime::{
    cancellation::{ProductiveTerminalClaim, claim_productive_terminal},
    config_io::GlobalConfigAuthority,
    context::ContextModelProfile,
    execution_plan::runtime_policy_target,
    fs_guards::AnchoredWorkspace,
    instructions::read_applicable_agent_instructions,
    session_definition::{SessionLogMetadata, verify_resume_definition_metadata_values},
    types::RuntimeError,
};
#[cfg(test)]
use std::cell::RefCell;

fn verify_productive_model_profile(
    run_session_id: &str,
    recorded: &SessionLogMetadata,
    model: &str,
    profile: crate::runtime::context::ContextModelProfile,
) -> Result<(), RuntimeError> {
    if recorded.model.as_deref() == Some(model)
        && recorded.model_profile_id.as_deref() == Some(profile.id)
        && recorded.model_context_limit == Some(profile.context_limit)
        && recorded.output_reserve == Some(profile.output_reserve)
        && recorded.safety_margin == Some(profile.safety_margin)
    {
        return Ok(());
    }
    Err(RuntimeError::Protocol(format!(
        "productive run {run_session_id} model profile does not match the recorded Run"
    )))
}

#[cfg(test)]
std::thread_local! {
    static PRODUCTIVE_PRE_RUN_CREATE_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static PRODUCTIVE_PRE_RUN_PUBLISH_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static PRODUCTIVE_RUN_COMMIT_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static RUN_POST_CONFIG_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static RUN_PRE_PLAN_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_productive_pre_run_create_observer(observer: impl FnOnce() + 'static) {
    PRODUCTIVE_PRE_RUN_CREATE_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
pub(crate) fn set_productive_pre_run_publish_observer(observer: impl FnOnce() + 'static) {
    PRODUCTIVE_PRE_RUN_PUBLISH_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
pub(crate) fn set_productive_run_commit_observer(observer: impl FnOnce() + 'static) {
    PRODUCTIVE_RUN_COMMIT_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
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
fn run_post_config_observer() {
    if let Some(observer) = RUN_POST_CONFIG_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[cfg(test)]
fn productive_pre_run_create_observer() {
    if let Some(observer) = PRODUCTIVE_PRE_RUN_CREATE_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[cfg(test)]
fn productive_pre_run_publish_observer() {
    if let Some(observer) = PRODUCTIVE_PRE_RUN_PUBLISH_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[cfg(test)]
fn productive_run_commit_observer() {
    if let Some(observer) = PRODUCTIVE_RUN_COMMIT_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

fn reconcile_productive_preflight<T, E>(result: Result<T, E>) -> Result<T, RuntimeError>
where
    E: Into<RuntimeError>,
{
    match result.map_err(Into::into) {
        Err(_) if claim_productive_terminal() == ProductiveTerminalClaim::Cancellation => {
            Err(RuntimeError::Cancelled)
        }
        result => result,
    }
}

struct RecordedProductivePreflight {
    registry: core_script::ResolvedRegistry,
    flow_ref: String,
    policy: core_policy::PolicyArtifact,
    credential: crate::runtime::oauth_credential::CredentialRecord,
    agent_instructions: String,
}

#[allow(clippy::too_many_arguments)]
fn prepare_recorded_productive_preflight<C>(
    execution_workspace: &AnchoredWorkspace,
    authority: &GlobalConfigAuthority,
    run_session_id: &str,
    recorded: &SessionLogMetadata,
    missing_flow_id_message: &'static str,
    model: &str,
    model_profile: ContextModelProfile,
    resolve_credential: C,
) -> Result<RecordedProductivePreflight, RuntimeError>
where
    C: FnOnce() -> Result<crate::runtime::oauth_credential::CredentialRecord, RuntimeError>,
{
    let config = &authority.config;
    let flow_ref = reconcile_productive_preflight(
        recorded
            .flow_definition_id
            .as_deref()
            .ok_or_else(|| RuntimeError::Protocol(missing_flow_id_message.to_owned())),
    )?;
    reconcile_productive_preflight(verify_productive_model_profile(
        run_session_id,
        recorded,
        model,
        model_profile,
    ))?;
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
    reconcile_productive_preflight(verify_resume_definition_metadata_values(
        run_session_id,
        recorded,
        &registry,
        flow_block,
    ))?;
    let policy = reconcile_productive_preflight(core_policy::compile_policy_artifact(
        &registry,
        flow_ref,
        runtime_policy_target(),
    ))?;
    let credential = reconcile_productive_preflight(resolve_credential())?;
    let agent_instructions = reconcile_productive_preflight(read_applicable_agent_instructions(
        &authority.home,
        execution_workspace,
    ))?;
    Ok(RecordedProductivePreflight {
        registry,
        flow_ref: flow_ref.to_owned(),
        policy,
        credential,
        agent_instructions,
    })
}

mod continuation;
#[cfg(test)]
pub(crate) use continuation::continue_conversation_with_provider;
pub use continuation::{
    continue_conversation, continue_conversation_with_execution_activation,
    continue_conversation_with_live_events,
};

mod new_run;
pub use new_run::{
    run_flow, run_flow_with_execution_activation, run_flow_with_live_events,
    run_flow_with_root_input, run_flow_with_root_input_and_live_events,
};
#[cfg(test)]
pub(crate) use new_run::{
    run_flow_internal_with_cleanup_observer, run_flow_internal_with_stage_observers,
};

mod productive_run;
#[cfg(test)]
pub(crate) use productive_run::{
    execute_reserved_productive_recovery, run_productive_session_with_provider,
};

mod resume_run;
pub use resume_run::{
    resume_conversation_run, resume_conversation_run_with_execution_activation,
    resume_conversation_run_with_live_events,
};
#[cfg(test)]
pub(crate) use resume_run::{
    resume_conversation_run_with_provider, resume_conversation_run_with_provider_and_live_events,
    resume_conversation_run_with_provider_and_preflight,
};
