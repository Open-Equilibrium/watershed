use crate::{
    runtime::{session::resume_conversation_run_with_provider_and_preflight, types::RuntimeError},
    tests::{
        helpers::write_productive_workspace_config, productive::support::FakeProvider,
        test_support::workspace_copy,
    },
};

fn cancel_during_preflight() {
    assert_eq!(
        crate::request_productive_interrupt(),
        crate::ProductiveInterruptAction::Cancel
    );
}

fn cancel_then_platform_error() -> Result<(), RuntimeError> {
    cancel_during_preflight();
    Err(RuntimeError::ProductiveExecutionUnavailable)
}

fn cancel_then_preflight_success() -> Result<(), RuntimeError> {
    cancel_during_preflight();
    Ok(())
}

#[test]
fn resume_cancellation_wins_over_preflight_outcome() {
    for (name, preflight) in [
        (
            "platform preflight error",
            cancel_then_platform_error as fn() -> Result<(), RuntimeError>,
        ),
        ("workspace preflight success", cancel_then_preflight_success),
    ] {
        let workspace = workspace_copy("smoke-flow");
        write_productive_workspace_config(&workspace);
        let mut provider = FakeProvider::default();
        let error = resume_conversation_run_with_provider_and_preflight(
            &workspace,
            preflight,
            &mut provider,
        )
        .expect_err(name);

        assert!(
            matches!(error, RuntimeError::Cancelled),
            "{name}: {error:?}"
        );
        assert!(provider.bodies.is_empty(), "{name}");
        assert!(!crate::settle_productive_operation(), "{name}");
    }
}
