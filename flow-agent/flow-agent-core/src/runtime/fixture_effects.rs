use crate::runtime::{
    execution_plan::{PlannedFixtureAction, PlannedFixtureEffect},
    fixture_tools::{plan_own_script, preflight_own_script_outputs, write_script_output},
    fs_guards::AnchoredDir,
    types::RuntimeError,
};

#[cfg(test)]
std::thread_local! {
    static FIXTURE_TOOL_APPLIED_IDS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub fn reset_fixture_tool_apply_count() {
    FIXTURE_TOOL_APPLIED_IDS.with_borrow_mut(Vec::clear);
}

#[cfg(test)]
pub fn fixture_tool_apply_count() -> usize {
    FIXTURE_TOOL_APPLIED_IDS.with_borrow(Vec::len)
}

#[cfg(test)]
pub fn fixture_tool_applied_ids() -> Vec<String> {
    FIXTURE_TOOL_APPLIED_IDS.with_borrow(Clone::clone)
}

pub fn compile_fixture_tool_effect(
    tool: &core_script::ToolBlock,
    policy: &core_policy::CommandPolicy,
) -> Result<(PlannedFixtureEffect, Option<&'static str>), RuntimeError> {
    match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => {
            let progress = execute_predefined_command(policy, command_id, argv)?;
            Ok((
                PlannedFixtureEffect::PredefinedCommand {
                    command_id: command_id.clone(),
                    argv: argv.clone(),
                    progress: progress.map(str::to_owned),
                },
                progress,
            ))
        }
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(_)) => {
            let write = plan_own_script(tool, policy)?;
            Ok((
                PlannedFixtureEffect::OwnScript {
                    progress: "stub write completed".to_owned(),
                    write,
                },
                Some("stub write completed"),
            ))
        }
        _ => Err(RuntimeError::Protocol(format!(
            "tool command shape does not match {}",
            tool.identity.id
        ))),
    }
}

pub fn preflight_planned_fixture_effect(
    workspace: &AnchoredDir,
    action: &PlannedFixtureAction,
) -> Result<(), RuntimeError> {
    match &action.effect {
        PlannedFixtureEffect::PredefinedCommand {
            command_id, argv, ..
        } => {
            execute_predefined_command(&action.command_policy, command_id, argv)?;
            Ok(())
        }
        PlannedFixtureEffect::OwnScript { write, .. } => {
            preflight_own_script_outputs(workspace, write.as_ref())
        }
    }
}

pub fn apply_planned_fixture_effect(
    workspace: &AnchoredDir,
    action: &PlannedFixtureAction,
) -> Result<(), RuntimeError> {
    #[cfg(test)]
    FIXTURE_TOOL_APPLIED_IDS.with_borrow_mut(|ids| {
        ids.push(action.failure_transition.tool_id.clone());
    });
    match &action.effect {
        PlannedFixtureEffect::PredefinedCommand {
            command_id, argv, ..
        } => {
            execute_predefined_command(&action.command_policy, command_id, argv)?;
            Ok(())
        }
        PlannedFixtureEffect::OwnScript { write, .. } => {
            if let Some(write) = write {
                write_script_output(workspace, &write.target, &write.contents)?;
            }
            Ok(())
        }
    }
}

pub fn execute_predefined_command(
    policy: &core_policy::CommandPolicy,
    command_id: &str,
    argv: &[String],
) -> Result<Option<&'static str>, RuntimeError> {
    let progress = trusted_predefined_command_progress(command_id)?;
    let executable = core_policy::TrustedPredefinedCommand::parse(command_id)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!("unsupported predefined command {command_id:?}"))
        })?
        .executable();
    if policy.executable != executable {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy executable does not match trusted command {command_id:?}"
        )));
    }
    if policy.argv != argv {
        return Err(RuntimeError::Protocol(format!(
            "runtime policy arguments do not match trusted command {command_id:?}"
        )));
    }
    Ok(progress)
}

pub fn trusted_predefined_command_progress(
    command_id: &str,
) -> Result<Option<&'static str>, RuntimeError> {
    let command = core_policy::TrustedPredefinedCommand::parse(command_id).ok_or_else(|| {
        RuntimeError::Protocol(format!("unsupported predefined command {command_id:?}"))
    })?;
    Ok((command == core_policy::TrustedPredefinedCommand::Read).then_some("stub read completed"))
}
