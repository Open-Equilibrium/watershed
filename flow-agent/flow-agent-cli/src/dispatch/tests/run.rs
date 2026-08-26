use super::super::{dispatch_in_workspace, execution::set_run_activation_observer};
use crate::{interrupt::InterruptCoordinator, test_support};
use flow_agent_core::RuntimeError;

#[test]
fn fixture_run_keeps_the_productive_interrupt_coordinator_idle() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    set_run_activation_observer(|| {
        assert_eq!(
            flow_agent_core::request_productive_interrupt(),
            flow_agent_core::ProductiveInterruptAction::Exit
        );
    });
    let args = ["run", "smoke-flow"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    dispatch_in_workspace(&args, &InterruptCoordinator::new(), &workspace)
        .expect("fixture run completes without productive activation");
}

#[test]
fn activated_backend_selection_remains_authoritative_for_execution() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    let config = test_support::session_home_path().join("config.yaml");
    std::fs::write(
        &config,
        "model: gpt-fixture\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("productive config is written");
    set_run_activation_observer(move || {
        std::fs::write(
            config,
            "fixture_profile: stub-model\nregistry_root: registry\nstub_model: deterministic\n",
        )
        .expect("fixture config replaces productive config");
        assert_eq!(
            flow_agent_core::request_productive_interrupt(),
            flow_agent_core::ProductiveInterruptAction::Cancel
        );
    });
    let args = ["run", "smoke-flow"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let result = dispatch_in_workspace(&args, &InterruptCoordinator::new(), &workspace);

    assert!(
        !matches!(result, Ok(code) if code == std::process::ExitCode::SUCCESS),
        "the backend loaded for activation must also govern execution"
    );
    assert_eq!(
        flow_agent_core::request_productive_interrupt(),
        flow_agent_core::ProductiveInterruptAction::Exit
    );
}

#[test]
fn cancellation_during_fallible_preflight_is_returned_without_a_session() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    let config = test_support::session_home_path().join("config.yaml");
    std::fs::write(
        config,
        concat!(
            "model: gpt-fixture\n",
            "provider: openai-codex\n",
            "model_context_limit: 128000\n",
            "output_reserve: 16384\n",
            "registry_root: registry\n",
        ),
    )
    .expect("productive config is written");
    let registry_file = test_support::session_home_path().join("registry/flows/smoke-flow.yaml");
    set_run_activation_observer(move || {
        assert_eq!(
            flow_agent_core::request_productive_interrupt(),
            flow_agent_core::ProductiveInterruptAction::Cancel
        );
        std::fs::remove_file(registry_file).expect("registry failure is injected");
    });
    let args = ["run", "smoke-flow"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let result = dispatch_in_workspace(&args, &InterruptCoordinator::new(), &workspace);

    assert!(
        matches!(result, Err(RuntimeError::Cancelled)),
        "preflight cancellation must be visible, got {result:?}"
    );
    assert!(
        !test_support::workspace_session_dir(&workspace).exists(),
        "preflight cancellation must not invent a session lifecycle"
    );
    assert_eq!(
        flow_agent_core::request_productive_interrupt(),
        flow_agent_core::ProductiveInterruptAction::Exit
    );
}
