use crate::{
    runtime::{session::resume_conversation_run_with_provider_and_preflight, types::RuntimeError},
    tests::{productive::support::FakeProvider, test_support::workspace_copy},
};

#[test]
fn resume_activates_controlled_cancellation_before_platform_preflight() {
    fn cancel_during_preflight() -> Result<(), RuntimeError> {
        assert_eq!(
            crate::request_productive_interrupt(),
            crate::ProductiveInterruptAction::Cancel
        );
        Err(RuntimeError::ProductiveExecutionUnavailable)
    }

    let workspace = workspace_copy("smoke-flow");
    let mut provider = FakeProvider::default();
    let error = resume_conversation_run_with_provider_and_preflight(
        &workspace,
        cancel_during_preflight,
        &mut provider,
    )
    .expect_err("preflight cancellation is controlled");

    assert!(matches!(error, RuntimeError::Cancelled), "{error:?}");
    assert!(provider.bodies.is_empty());
    assert!(!crate::settle_productive_operation());
}

#[test]
fn resume_reconciles_cancellation_with_workspace_preflight_failure() {
    fn cancel_during_preflight() -> Result<(), RuntimeError> {
        assert_eq!(
            crate::request_productive_interrupt(),
            crate::ProductiveInterruptAction::Cancel
        );
        Ok(())
    }

    let workspace = workspace_copy("smoke-flow").join("missing");
    let mut provider = FakeProvider::default();
    let error = resume_conversation_run_with_provider_and_preflight(
        &workspace,
        cancel_during_preflight,
        &mut provider,
    )
    .expect_err("workspace failure reconciles with controlled cancellation");

    assert!(matches!(error, RuntimeError::Cancelled), "{error:?}");
    assert!(provider.bodies.is_empty());
    assert!(!crate::settle_productive_operation());
}
