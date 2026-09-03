use super::super::process::child_exited_without_reaping;
use super::transport::{
    ExecutorPreflightProcess, WaitingExecutor, preflight_one_shot, preflight_one_shot_at_deadline,
    start_one_shot_at_deadlines,
};
use crate::runtime::types::RuntimeError;
use std::{cell::Cell, fs::File, path::Path, process::Child, time::Duration, time::Instant};

#[test]
fn expired_preflight_rejects_available_ready_and_error_records() {
    for (name, response, wait_for_exit) in [
        (
            "ready",
            proto::ExecutorPreflightV0::Ready {
                request_id: "request-1".to_owned(),
                schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
            },
            false,
        ),
        (
            "error",
            proto::ExecutorPreflightV0::Error {
                code: proto::ExecutorErrorCodeV0::PolicyUnsupported,
                message: "unsupported".to_owned(),
                request_id: "request-1".to_owned(),
                schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
            },
            true,
        ),
    ] {
        let workspace = crate::tests::empty_workspace();
        let response_ready = workspace.join(format!("{name}-response-ready"));
        let tool_started = workspace.join(format!("{name}-tool-started"));
        let response = canonical_preflight(&response);
        let script = format!(
            "printf '%s' '{response}'\n\
             printf ready > '{response_ready}'\n\
             {after_response}\n",
            response_ready = response_ready.display(),
            after_response = if wait_for_exit {
                "exit 0".to_owned()
            } else {
                format!(
                    "IFS= read -r _start && printf started > '{}'",
                    tool_started.display()
                )
            },
        );
        let request = request();
        let executor = File::open("/bin/sh").expect("shell executor opens");
        let before = Instant::now();
        let deadline = before + Duration::from_secs(1);
        let after = deadline + Duration::from_secs(1);
        let deadline_crossed = Cell::new(false);

        let outcome = preflight_one_shot_at_deadline(
            &executor,
            &[],
            &request,
            script.as_bytes(),
            deadline,
            |child| {
                wait_until(|| response_ready.is_file(), "preflight response");
                if wait_for_exit {
                    wait_for_exit_without_reaping(child);
                }
                deadline_crossed.set(true);
                Ok(())
            },
            || {
                if deadline_crossed.get() {
                    after
                } else {
                    before
                }
            },
        );

        assert_executor_invalid(outcome);
        assert!(!tool_started.exists(), "expired preflight published Start");
    }
}

#[test]
fn expired_start_deadline_writes_nothing() {
    let workspace = crate::tests::empty_workspace();
    let tool_started = workspace.join("tool-started");
    let request = request();
    let waiting = waiting_executor(&request, &tool_started);
    let before = Instant::now();
    let deadline = before + Duration::from_secs(1);
    let after = deadline + Duration::from_secs(1);

    let outcome = start_one_shot_at_deadlines(
        waiting,
        deadline,
        |_| panic!("terminal deadline began before Start was published"),
        || after,
    );

    assert_executor_invalid(outcome);
    assert!(!tool_started.exists(), "expired Start granted authority");
}

#[test]
fn expired_terminal_deadline_rejects_available_result() {
    let workspace = crate::tests::empty_workspace();
    let tool_started = workspace.join("tool-started");
    let request = request();
    let waiting = waiting_executor(&request, &tool_started);
    let before = Instant::now();
    let deadline = before + Duration::from_secs(1);
    let after = deadline + Duration::from_secs(1);
    let deadline_crossed = Cell::new(false);

    let outcome = start_one_shot_at_deadlines(
        waiting,
        deadline,
        |child| {
            wait_for_exit_without_reaping(child);
            deadline_crossed.set(true);
            Ok(deadline)
        },
        || {
            if deadline_crossed.get() {
                after
            } else {
                before
            }
        },
    );

    assert_executor_invalid(outcome);
    assert!(
        tool_started.is_file(),
        "test did not reach post-Start execution"
    );
}

fn waiting_executor(request: &proto::ExecutorRequestV0, marker: &Path) -> WaitingExecutor {
    let preflight = canonical_preflight(&proto::ExecutorPreflightV0::Ready {
        request_id: request.request_id.clone(),
        schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
    });
    let response = String::from_utf8(
        proto::canonical_executor_response_v0(&proto::ExecutorResponseV0::Completed {
            enforcement: proto::EnforcementReceiptV0 {
                applied_policy_digest: request.policy_digest.clone(),
                backend: proto::EXECUTOR_BACKEND_V0.to_owned(),
                backend_version: "test".to_owned(),
                executor: proto::EXECUTOR_NAME_V0.to_owned(),
                executor_version: "test".to_owned(),
                isolation_active: true,
                max_concurrent_processes_and_threads: 16,
                platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
                runtime_profile: proto::RuntimeReadProfileV0::Exact,
            },
            request_id: request.request_id.clone(),
            schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
            tool_result: proto::ExecutorToolResultV0 {
                classification: None,
                exit_code: Some(0),
                status: proto::ExecutorToolStatusV0::Completed,
                stderr_base64: proto::encode_executor_stream_v0(&[]),
                stdout_base64: proto::encode_executor_stream_v0(&[]),
            },
        })
        .expect("terminal response is canonical"),
    )
    .expect("terminal response is UTF-8");
    let script = format!(
        "printf '%s' '{preflight}'\n\
         IFS= read -r _start\n\
         printf started > '{marker}'\n\
         printf '%s' '{response}'\n",
        marker = marker.display(),
    );
    let executor = File::open("/bin/sh").expect("shell executor opens");
    match preflight_one_shot(&executor, &[], request, script.as_bytes())
        .expect("fake Executor reaches readiness")
    {
        ExecutorPreflightProcess::Ready(waiting) => waiting,
        ExecutorPreflightProcess::Rejected(code) => panic!("unexpected rejection: {code:?}"),
    }
}

fn request() -> proto::ExecutorRequestV0 {
    let limits = proto::ExecutorLimitsV0 {
        max_concurrent_processes_and_threads: 16,
        max_stderr_bytes: 0,
        max_stdout_bytes: 0,
        timeout_ms: 100,
    };
    proto::ExecutorRequestV0 {
        argv: Vec::new(),
        environment: Default::default(),
        executable: "/bin/sh".to_owned(),
        limits: limits.clone(),
        mounts: Vec::new(),
        resolved_policy: proto::ExecutorResolvedPolicyV0 {
            artifact: serde_json::json!({}),
            command: serde_json::json!({}),
            limits,
            mounts: Vec::new(),
            runtime_profile: proto::RuntimeReadProfileV0::Exact,
            tool_id: "tool".to_owned(),
            tool_kind: "command".to_owned(),
        },
        policy_digest: "a".repeat(64),
        request_id: "request-1".to_owned(),
        runtime_profile: proto::RuntimeReadProfileV0::Exact,
        schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
        tool_id: "tool".to_owned(),
        tool_kind: "command".to_owned(),
        working_directory: "/".to_owned(),
    }
}

fn canonical_preflight(response: &proto::ExecutorPreflightV0) -> String {
    String::from_utf8(
        proto::canonical_executor_preflight_v0(response).expect("preflight response is canonical"),
    )
    .expect("preflight response is UTF-8")
}

fn wait_for_exit_without_reaping(child: &mut Child) {
    wait_until(
        || child_exited_without_reaping(&*child).expect("fake Executor process is observable"),
        "Executor exit",
    );
}

fn wait_until(mut condition: impl FnMut() -> bool, description: &str) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "fake {description} was not observed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_executor_invalid<T>(outcome: Result<T, RuntimeError>) {
    match outcome {
        Err(RuntimeError::Executor(error)) => {
            assert_eq!(error.code(), proto::ExecutorErrorCodeV0::InvalidResponse);
        }
        Err(error) => panic!("unexpected error: {error}"),
        Ok(_) => panic!("expired Executor progress was accepted"),
    }
}
