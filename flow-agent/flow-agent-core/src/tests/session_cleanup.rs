use super::{
    helpers::{empty_workspace, reserve_session_log},
    test_support::workspace_copy,
};
use crate::runtime::{
    session::{run_flow_internal_with_cleanup_observer, run_flow_internal_with_stage_observers},
    session_reservation::write_reserved_session_metadata,
    stage_results::reconcile_controlled_stages,
    types::{RunOutput, RuntimeError},
};
use std::fs;

fn inject_runtime_operation_failure(
    result: Result<RunOutput, RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    result?;
    Err(RuntimeError::Protocol(
        "injected runtime operation failure".to_owned(),
    ))
}

#[test]
fn controlled_stage_failure_combinations_preserve_every_cause() {
    for (operation_failed, finalization_failed, cleanup_failed) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (true, true, false),
        (true, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let operation = if operation_failed {
            Err(RuntimeError::Protocol("operation failed".to_owned()))
        } else {
            Ok(())
        };
        let finalization = if finalization_failed {
            Err(RuntimeError::Protocol("finalization failed".to_owned()))
        } else {
            Ok(())
        };
        let cleanup = if cleanup_failed {
            Err(RuntimeError::Protocol("cleanup failed".to_owned()))
        } else {
            Ok(())
        };

        let err = reconcile_controlled_stages(operation, finalization, cleanup)
            .expect_err("the injected stage failure must be returned");
        let text = err.to_string();

        assert_eq!(
            text.contains("operation failed"),
            operation_failed,
            "{text}"
        );
        assert_eq!(
            text.contains("finalization failed"),
            finalization_failed,
            "{text}"
        );
        assert_eq!(text.contains("cleanup failed"), cleanup_failed, "{text}");
        let expected_source = if operation_failed {
            "operation failed"
        } else if finalization_failed {
            "finalization failed"
        } else {
            "cleanup failed"
        };
        assert_eq!(
            std::error::Error::source(&err).map(ToString::to_string),
            if operation_failed as u8 + finalization_failed as u8 + cleanup_failed as u8 > 1 {
                Some(expected_source.to_owned())
            } else {
                None
            },
            "{text}"
        );
    }
}

#[test]
fn runtime_and_writer_finalization_failures_remain_visible() {
    let workspace = workspace_copy("sandbox-negative");
    let lock_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("sandbox-negative-write.lock");

    let err = run_flow_internal_with_stage_observers(
        &workspace,
        "sandbox-negative-write",
        false,
        inject_runtime_operation_failure,
        |_| {
            Err(RuntimeError::Protocol(
                "injected writer finalization failure".to_owned(),
            ))
        },
        |_| {},
    )
    .expect_err("runtime and finalization failures must be retained");

    assert!(matches!(
        &err,
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: Some(finalization),
            cleanup: None,
        } if operation.to_string().contains("injected runtime operation failure")
            && finalization.to_string().contains("injected writer finalization failure")
    ));
    assert!(lock_path.exists());
}

#[test]
fn runtime_finalization_and_real_cleanup_failures_remain_visible() {
    let workspace = workspace_copy("sandbox-negative");
    let lock_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("sandbox-negative-write.lock");

    let err = run_flow_internal_with_stage_observers(
        &workspace,
        "sandbox-negative-write",
        false,
        inject_runtime_operation_failure,
        |_| {
            Err(RuntimeError::Protocol(
                "injected writer finalization failure".to_owned(),
            ))
        },
        |lock| {
            lock.remove().expect("lock file removed");
            fs::create_dir(lock.diagnostic_path()).expect("lock path replaced with a directory");
        },
    )
    .expect_err("all three failures must be retained");

    assert!(matches!(
        &err,
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: Some(finalization),
            cleanup: Some(cleanup),
        } if operation.to_string().contains("injected runtime operation failure")
            && finalization.to_string().contains("injected writer finalization failure")
            && cleanup.to_string().contains("sandbox-negative-write.lock")
    ));
    assert!(
        err.to_string()
            .contains("injected writer finalization failure"),
        "{err}"
    );
    assert!(
        err.to_string().contains("sandbox-negative-write.lock"),
        "{err}"
    );
    assert!(
        std::error::Error::source(&err).is_some_and(|source| source
            .to_string()
            .contains("injected runtime operation failure")),
        "{err}"
    );

    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
}

#[test]
fn controlled_cleanup_retains_empty_orphans_and_preserves_active_reservations() {
    let workspace = empty_workspace("controlled-reservation-state");
    let empty =
        reserve_session_log(&workspace, "emptycontrolled001").expect("empty reservation succeeds");
    let empty_paths = [
        empty.session_path.diagnostic_path().to_owned(),
        empty.log_path.diagnostic_path().to_owned(),
        empty.context_path.diagnostic_path().to_owned(),
        empty.lock_path.diagnostic_path().to_owned(),
    ];

    let empty_err = reconcile_controlled_stages::<()>(
        Err(RuntimeError::Protocol("controlled failure".to_owned())),
        Ok(()),
        empty.cleanup(),
    )
    .expect_err("operation failure remains visible");

    match empty_err {
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: None,
            cleanup: Some(cleanup),
        } => {
            assert!(matches!(
                *operation,
                RuntimeError::Protocol(message) if message == "controlled failure"
            ));
            assert!(
                cleanup.to_string().contains("cannot safely remove"),
                "{cleanup}"
            );
        }
        other => panic!("unexpected controlled failure: {other}"),
    }
    for orphan in &empty_paths[..3] {
        assert_eq!(
            fs::read(orphan).expect("owned orphan remains readable"),
            b""
        );
    }
    assert!(!empty_paths[3].exists());

    let active = reserve_session_log(&workspace, "activecontrolled001")
        .expect("active reservation succeeds");
    write_reserved_session_metadata(&active, None).expect("metadata activates reservation");
    let active_paths = [
        active.session_path.diagnostic_path().to_owned(),
        active.log_path.diagnostic_path().to_owned(),
        active.context_path.diagnostic_path().to_owned(),
    ];
    let active_lock = active.lock_path.diagnostic_path().to_owned();

    let active_err = reconcile_controlled_stages::<()>(
        Err(RuntimeError::Protocol("controlled failure".to_owned())),
        Ok(()),
        active.cleanup(),
    )
    .expect_err("operation failure remains visible");

    assert!(
        matches!(active_err, RuntimeError::Protocol(message) if message == "controlled failure")
    );
    assert!(active_paths.iter().all(|path| path.is_file()));
    assert!(active_lock.exists());
}

#[test]
fn controlled_run_cleanup_failure_is_returned_and_keeps_valid_artifacts() {
    let workspace = workspace_copy("smoke-flow");
    let lock_path =
        crate::tests::helpers::workspace_session_dir(&workspace).join("smoke-flow.lock");

    let err = run_flow_internal_with_cleanup_observer(&workspace, "smoke-flow", true, |lock| {
        lock.remove().expect("lock file removed");
        fs::create_dir(lock.diagnostic_path()).expect("lock path replaced with a directory");
    })
    .expect_err("cleanup failure must replace a successful return");

    assert!(
        matches!(
            err,
            RuntimeError::Io { ref path, .. } if path == &lock_path
        ) || err.to_string().contains("must be a file"),
        "{err}"
    );
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("smoke-flow.jsonl")
            .is_file()
    );
    assert!(lock_path.is_dir());
    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
}

#[test]
fn controlled_run_operation_and_cleanup_failures_are_both_returned() {
    let workspace = workspace_copy("sandbox-negative");
    let lock_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("sandbox-negative-write.lock");

    let err = run_flow_internal_with_stage_observers(
        &workspace,
        "sandbox-negative-write",
        false,
        inject_runtime_operation_failure,
        |result| result,
        |lock| {
            lock.remove().expect("lock file removed");
            fs::create_dir(lock.diagnostic_path()).expect("lock path replaced with a directory");
        },
    )
    .expect_err("operation and cleanup failures must both be returned");

    assert!(matches!(
        &err,
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: None,
            cleanup: Some(cleanup),
        } if operation.to_string().contains("injected runtime operation failure")
            && cleanup.to_string().contains("sandbox-negative-write.lock")
    ));
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("sandbox-negative-write.jsonl")
            .is_file()
    );
    assert!(lock_path.is_dir());
    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
}
