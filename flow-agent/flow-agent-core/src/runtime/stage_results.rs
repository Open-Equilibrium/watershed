use crate::runtime::types::RuntimeError;

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

pub(crate) fn reconcile_operation_and_cleanup<T>(
    operation: Result<T, RuntimeError>,
    cleanup: Result<(), RuntimeError>,
) -> Result<T, RuntimeError> {
    reconcile_controlled_stages(operation, Ok(()), cleanup)
}

pub(crate) fn reconcile_cleanup_failures(
    mut failures: Vec<RuntimeError>,
) -> Result<(), RuntimeError> {
    match failures.len() {
        0 => Ok(()),
        1 => Err(failures.pop().expect("one cleanup failure exists")),
        _ => Err(RuntimeError::SessionCleanupFailures(
            failures.into_iter().map(Box::new).collect(),
        )),
    }
}
