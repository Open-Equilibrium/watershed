#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::{ExecutorSelection, resolve_executor};
use crate::runtime::{
    fs_guards::AnchoredWorkspace,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::RuntimeError,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use preparation::{RetainedSource, retain_runtime_sources};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use response::{decode_tool_outcome, validate_receipt_identity};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::collections::BTreeMap;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use transport::{ExecutorPreflightProcess, preflight_one_shot, start_one_shot};

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod deadline_tests;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod preparation;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod response;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod transport;

/// Definitive result of one bounded Executor dispatch.
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub(crate) enum ExecutorDispatchOutcome {
    Completed(Box<ExecutorToolExecution>),
    Error(proto::ExecutorErrorCodeV0),
}

/// Result of validating one exact Tool request in its retained one-shot Executor.
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub(crate) enum ExecutorPreflightOutcome {
    Ready(Box<PreparedExecutorWaiting>),
    Rejected(proto::ExecutorErrorCodeV0),
}

/// Validated result and enforcement evidence from one isolated Tool execution.
pub(crate) struct ExecutorToolExecution {
    pub(crate) enforcement: proto::EnforcementReceiptV0,
    pub(crate) outcome: ToolExecutionOutcome,
    pub(crate) request_hash: String,
}

/// Fully validated request with every filesystem capability retained before recovery or dispatch.
pub(crate) struct PreparedExecutorTool {
    request_hash: String,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    mounts: Vec<preparation::PreparedMount>,
    request: proto::ExecutorRequestV0,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    request_bytes: Vec<u8>,
}

/// One validated Tool request waiting for explicit start in its retained Executor process.
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub(crate) struct PreparedExecutorWaiting {
    prepared: PreparedExecutorTool,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    process: transport::WaitingExecutor,
}

impl PreparedExecutorTool {
    pub(crate) fn request_hash(&self) -> &str {
        &self.request_hash
    }

    pub(crate) fn policy_digest(&self) -> &str {
        &self.request.policy_digest
    }

    pub(crate) fn max_concurrent_processes_and_threads(&self) -> u32 {
        self.request.limits.max_concurrent_processes_and_threads
    }

    pub(crate) fn runtime_profile(&self) -> proto::RuntimeReadProfileV0 {
        self.request.runtime_profile
    }
}

/// Ready one-shot Executor and the runtime objects retained from its validated manifest.
pub(crate) struct PreparedExecutor {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    selection: ExecutorSelection,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    runtime_sources: BTreeMap<String, RetainedSource>,
}

impl PreparedExecutor {
    /// Resolves and probes the selected Executor and retains every advertised runtime object.
    pub(crate) fn prepare_selected() -> Result<Self, RuntimeError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let selection = resolve_executor()?;
            let runtime_sources = retain_runtime_sources(selection.probe())?;
            Ok(Self {
                selection,
                runtime_sources,
            })
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            Err(RuntimeError::executor(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "productive Executor support requires Ubuntu 24.04 x64",
            ))
        }
    }

    /// Retains and hashes the exact Executor request without launching any process.
    pub(crate) fn prepare_tool(
        &self,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        invocation: &ToolInvocation,
        request_id: &str,
    ) -> Result<PreparedExecutorTool, RuntimeError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            self.prepare_tool_linux(workspace, policy, command_policy, invocation, request_id)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (workspace, policy, command_policy, invocation, request_id);
            Err(RuntimeError::executor(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "productive Executor support requires Ubuntu 24.04 x64",
            ))
        }
    }

    /// Launches exactly one Executor and validates the prepared request without starting its Tool.
    pub(crate) fn preflight_prepared(
        &self,
        prepared: PreparedExecutorTool,
    ) -> Result<ExecutorPreflightOutcome, RuntimeError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let process = preflight_one_shot(
                self.selection.executable(),
                &prepared.mounts,
                &prepared.request,
                &prepared.request_bytes,
            )?;
            Ok(match process {
                ExecutorPreflightProcess::Ready(process) => {
                    ExecutorPreflightOutcome::Ready(Box::new(PreparedExecutorWaiting {
                        prepared,
                        process,
                    }))
                }
                ExecutorPreflightProcess::Rejected(code) => {
                    ExecutorPreflightOutcome::Rejected(code)
                }
            })
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = prepared;
            Err(RuntimeError::executor(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "productive Executor support requires Ubuntu 24.04 x64",
            ))
        }
    }

    /// Explicitly starts a Tool in its already validated one-shot Executor.
    pub(crate) fn start_prepared(
        &self,
        waiting: PreparedExecutorWaiting,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let PreparedExecutorWaiting { prepared, process } = waiting;
            match start_one_shot(process)? {
                proto::ExecutorResponseV0::Completed {
                    enforcement,
                    tool_result,
                    ..
                } => {
                    self.validate_prepared_receipt(&prepared, &enforcement)?;
                    Ok(ExecutorDispatchOutcome::Completed(Box::new(
                        ExecutorToolExecution {
                            enforcement,
                            outcome: decode_tool_outcome(tool_result)?,
                            request_hash: prepared.request_hash,
                        },
                    )))
                }
                proto::ExecutorResponseV0::Error { code, .. } => {
                    Ok(ExecutorDispatchOutcome::Error(code))
                }
            }
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = waiting;
            Err(RuntimeError::executor(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "productive Executor support requires Ubuntu 24.04 x64",
            ))
        }
    }

    /// Runs both stages immediately for non-productive conformance and startup evidence.
    #[cfg_attr(
        not(all(target_os = "linux", target_arch = "x86_64")),
        allow(dead_code)
    )]
    pub(crate) fn execute_prepared(
        &self,
        prepared: PreparedExecutorTool,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError> {
        match self.preflight_prepared(prepared)? {
            ExecutorPreflightOutcome::Ready(waiting) => self.start_prepared(*waiting),
            ExecutorPreflightOutcome::Rejected(code) => Ok(ExecutorDispatchOutcome::Error(code)),
        }
    }

    pub(crate) fn validate_prepared_receipt(
        &self,
        prepared: &PreparedExecutorTool,
        receipt: &proto::EnforcementReceiptV0,
    ) -> Result<(), RuntimeError> {
        proto::validate_enforcement_receipt_v0(
            receipt,
            prepared.policy_digest(),
            prepared.runtime_profile(),
            prepared.max_concurrent_processes_and_threads(),
        )
        .map_err(|_| {
            RuntimeError::executor(
                proto::ExecutorErrorCodeV0::InvalidResponse,
                "Executor enforcement receipt does not match its prepared request",
            )
        })?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        validate_receipt_identity(receipt, self.selection.probe())?;
        Ok(())
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn invalid_request(_: proto::ExecutorProtocolError) -> RuntimeError {
    invalid_response("Flow constructed an invalid Executor protocol document")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn invalid_response(message: impl Into<String>) -> RuntimeError {
    executor_error(proto::ExecutorErrorCodeV0::InvalidResponse, message)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn runtime_open_error(_: rustix::io::Errno) -> RuntimeError {
    executor_error(
        proto::ExecutorErrorCodeV0::PolicyUnsupported,
        "Executor mount capability could not be retained",
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn executor_error(code: proto::ExecutorErrorCodeV0, message: impl Into<String>) -> RuntimeError {
    RuntimeError::executor(code, message)
}

#[cfg(all(test, not(all(target_os = "linux", target_arch = "x86_64"))))]
mod unsupported_platform_tests {
    use super::{PreparedExecutor, PreparedExecutorTool};
    use crate::runtime::{
        fs_guards::AnchoredWorkspace, tool_runner::ToolInvocation, types::RuntimeError,
    };
    #[test]
    fn prepared_tool_metadata_and_receipt_validation_remain_bound_on_unsupported_platforms() {
        let prepared = prepared_tool();
        assert_eq!(prepared.request_hash(), "sha256:request");
        assert_eq!(prepared.policy_digest(), &"a".repeat(64));
        assert_eq!(
            prepared.runtime_profile(),
            proto::RuntimeReadProfileV0::HostSystemRead
        );

        let executor = PreparedExecutor {};
        assert!(
            executor
                .validate_prepared_receipt(&prepared, &receipt(&prepared, true))
                .is_ok()
        );
        assert_policy_unsupported(executor.execute_prepared(prepared_tool()));

        let mut inactive = receipt(&prepared, false);
        inactive.applied_policy_digest = "b".repeat(64);
        assert_invalid_response(executor.validate_prepared_receipt(&prepared, &inactive));

        let mut widened = receipt(&prepared, true);
        widened.max_concurrent_processes_and_threads += 1;
        assert_invalid_response(executor.validate_prepared_receipt(&prepared, &widened));
    }

    #[test]
    fn prepare_tool_fails_closed_before_reading_its_candidate_inputs() {
        let workspace = AnchoredWorkspace::open(std::env::temp_dir().as_path())
            .expect("temporary directory is an anchored workspace");
        let policy = executor_policy();
        let command = &policy.commands[0];
        let invocation = ToolInvocation {
            executable: "/bin/echo".to_owned(),
            argv: Vec::new(),
        };

        assert_policy_unsupported(PreparedExecutor {}.prepare_tool(
            &workspace,
            &policy,
            command,
            &invocation,
            "unsupported-platform-request",
        ));
    }

    fn prepared_tool() -> PreparedExecutorTool {
        PreparedExecutorTool {
            request_hash: "sha256:request".to_owned(),
            request: proto::ExecutorRequestV0 {
                argv: Vec::new(),
                environment: std::collections::BTreeMap::new(),
                executable: "/bin/echo".to_owned(),
                limits: proto::ExecutorLimitsV0 {
                    max_concurrent_processes_and_threads: 16,
                    max_stderr_bytes: 0,
                    max_stdout_bytes: 0,
                    timeout_ms: 1_000,
                },
                mounts: Vec::new(),
                policy_digest: "a".repeat(64),
                request_id: "unsupported-platform-request".to_owned(),
                resolved_policy: proto::ExecutorResolvedPolicyV0 {
                    artifact: serde_json::json!({}),
                    command: serde_json::json!({}),
                    limits: proto::ExecutorLimitsV0 {
                        max_concurrent_processes_and_threads: 16,
                        max_stderr_bytes: 0,
                        max_stdout_bytes: 0,
                        timeout_ms: 1_000,
                    },
                    mounts: Vec::new(),
                    runtime_profile: proto::RuntimeReadProfileV0::HostSystemRead,
                    tool_id: "echo".to_owned(),
                    tool_kind: "predefined-command".to_owned(),
                },
                runtime_profile: proto::RuntimeReadProfileV0::HostSystemRead,
                schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
                tool_id: "echo".to_owned(),
                tool_kind: "predefined-command".to_owned(),
                working_directory: "/workspace".to_owned(),
            },
        }
    }

    fn receipt(
        prepared: &PreparedExecutorTool,
        isolation_active: bool,
    ) -> proto::EnforcementReceiptV0 {
        proto::EnforcementReceiptV0 {
            applied_policy_digest: prepared.policy_digest().to_owned(),
            backend: "backend".to_owned(),
            backend_version: "1".to_owned(),
            executor: "executor".to_owned(),
            executor_version: "1".to_owned(),
            isolation_active,
            max_concurrent_processes_and_threads: prepared.max_concurrent_processes_and_threads(),
            platform: "unsupported".to_owned(),
            runtime_profile: prepared.runtime_profile(),
        }
    }

    fn executor_policy() -> core_policy::PolicyArtifact {
        core_policy::PolicyArtifact {
            commands: vec![core_policy::CommandPolicy {
                allowed_parameters: Vec::new(),
                argv: Vec::new(),
                command_id: "echo".to_owned(),
                environment: core_policy::EnvironmentPolicy {
                    allow: Vec::new(),
                    default: core_policy::EnvironmentDefault::Clear,
                },
                executable: "registry:echo".to_owned(),
                filesystem: core_policy::FilesystemPolicy {
                    read_only_mounts: vec!["workspace".to_owned()],
                    writable_mounts: Vec::new(),
                },
                max_concurrent_processes_and_threads: 16,
                network: core_policy::NetworkPolicy {
                    allow: Vec::new(),
                    default: core_policy::NetworkDefault::Deny,
                },
                runtime_profile: core_policy::ToolRuntimeProfile::Exact,
                script_runtime: None,
                tool_id: "echo".to_owned(),
                tool_kind: core_policy::ToolKind::PredefinedCommand,
            }],
            phase_scope: vec![core_policy::PhaseScope {
                phase_id: "phase".to_owned(),
                tool_ids: vec!["echo".to_owned()],
            }],
            policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
            runtime_limits: core_policy::RuntimeLimits {
                headless: true,
                timeout_ms: 1_000,
            },
            source_flow_definition_id: "flow".to_owned(),
            target: core_policy::PolicyTarget::LinuxBubblewrapSeccomp,
        }
    }

    fn assert_policy_unsupported<T>(result: Result<T, RuntimeError>) {
        match result {
            Err(RuntimeError::Executor(error)) => {
                assert_eq!(error.code(), proto::ExecutorErrorCodeV0::PolicyUnsupported);
            }
            Err(error) => panic!("unexpected error: {error}"),
            Ok(_) => panic!("unsupported platform must fail closed"),
        }
    }

    fn assert_invalid_response(result: Result<(), RuntimeError>) {
        match result {
            Err(RuntimeError::Executor(error)) => {
                assert_eq!(error.code(), proto::ExecutorErrorCodeV0::InvalidResponse);
            }
            Err(error) => panic!("unexpected error: {error}"),
            Ok(()) => panic!("invalid receipt must be rejected"),
        }
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::super::process::{
        process_group_cleanup_calls_for_test, reset_process_group_cleanup_calls_for_test,
    };
    use super::preparation::{
        executor_request_hash, runtime_profile, validate_executor_executable,
    };
    use super::response::{decode_tool_outcome, validate_receipt_identity};
    use super::transport::{
        ExecutorPreflightProcess, c_close, duplicate_executor_descriptor, preflight_one_shot,
        preflight_one_shot_at_deadline, start_one_shot,
    };
    use crate::runtime::run_attempts::{RunAttemptOutcome, ToolTerminalClassification};
    use std::{
        collections::BTreeMap,
        fs::File,
        os::fd::AsRawFd as _,
        os::unix::process::CommandExt as _,
        process::Command,
        time::{Duration, Instant},
    };

    #[test]
    fn executor_wire_terminal_values_map_to_runtime_results() {
        use proto::{
            ExecutorToolClassificationV0 as WireClass, ExecutorToolStatusV0 as WireStatus,
        };

        let cases = [
            (
                WireStatus::Completed,
                None,
                RunAttemptOutcome::Completed,
                None,
            ),
            (
                WireStatus::Failed,
                Some(WireClass::NonzeroExit),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::NonzeroExit),
            ),
            (
                WireStatus::Failed,
                Some(WireClass::ProcessCapacityExceeded),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::ProcessCapacityExceeded),
            ),
            (
                WireStatus::Failed,
                Some(WireClass::SignalTermination),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::SignalTermination),
            ),
            (
                WireStatus::Failed,
                Some(WireClass::StderrCapExceeded),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::StderrCapExceeded),
            ),
            (
                WireStatus::Failed,
                Some(WireClass::StdoutCapExceeded),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::StdoutCapExceeded),
            ),
            (
                WireStatus::Failed,
                Some(WireClass::StdoutStderrCapExceeded),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::StdoutStderrCapExceeded),
            ),
            (
                WireStatus::TimedOut,
                Some(WireClass::ToolTimedOut),
                RunAttemptOutcome::TimedOut,
                Some(ToolTerminalClassification::ToolTimedOut),
            ),
            (
                WireStatus::Failed,
                Some(WireClass::OutputCollectorFailed),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::OutputCollectorFailed),
            ),
            (
                WireStatus::Failed,
                Some(WireClass::OutputDrainTimeout),
                RunAttemptOutcome::Failed,
                Some(ToolTerminalClassification::OutputDrainTimeout),
            ),
            (
                WireStatus::Cancelled,
                Some(WireClass::Cancelled),
                RunAttemptOutcome::Cancelled,
                Some(ToolTerminalClassification::Cancelled),
            ),
        ];

        for (status, classification, expected_status, expected_classification) in cases {
            let outcome = decode_tool_outcome(proto::ExecutorToolResultV0 {
                classification,
                exit_code: None,
                status,
                stderr_base64: proto::encode_executor_stream_v0(b""),
                stdout_base64: proto::encode_executor_stream_v0(b""),
            })
            .expect("a reachable Executor terminal result decodes");
            assert_eq!(outcome.status, expected_status);
            assert_eq!(outcome.classification, expected_classification);
        }
    }

    #[test]
    fn host_system_read_profile_maps_to_the_wire_contract() {
        assert_eq!(
            runtime_profile(core_script::ToolRuntimeProfile::HostSystemRead),
            proto::RuntimeReadProfileV0::HostSystemRead
        );
    }

    #[test]
    fn one_shot_executor_works_without_parent_standard_descriptors() {
        const CHILD_ENV: &str = "WATERSHED_EXECUTOR_WITHOUT_STDIO_CHILD";
        if std::env::var_os(CHILD_ENV).is_none() && std::env::var_os("NEXTEST").is_none() {
            let test_name = std::thread::current()
                .name()
                .expect("test thread has a name")
                .to_owned();
            let mut command =
                Command::new(std::env::current_exe().expect("core test executable resolves"));
            command
                .args(["--exact", &test_name, "--nocapture"])
                .env(CHILD_ENV, "1");
            unsafe {
                command.pre_exec(|| {
                    for descriptor in 0..=2 {
                        let _ = c_close(descriptor);
                    }
                    Ok(())
                });
            }
            let status = command.status().expect("isolated core test starts");
            assert!(status.success(), "isolated core test failed");
            return;
        }
        if std::env::var_os("NEXTEST").is_some() {
            for descriptor in 0..=2 {
                // SAFETY: nextest gives this test its own process, and the test deliberately
                // invalidates only that process's inherited standard descriptors.
                unsafe {
                    let _ = c_close(descriptor);
                }
            }
        }

        let request = one_shot_request();
        let response = proto::ExecutorPreflightV0::Error {
            code: proto::ExecutorErrorCodeV0::Unavailable,
            message: "unavailable".to_owned(),
            request_id: request.request_id.clone(),
            schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
        };
        let response = String::from_utf8(
            proto::canonical_executor_preflight_v0(&response).expect("response is canonical"),
        )
        .expect("response is UTF-8");
        let script = format!("printf '%s' '{response}'\n");
        let executor = File::open("/bin/sh").expect("shell executor opens");

        let response = preflight_one_shot(&executor, &[], &request, script.as_bytes())
            .expect("fake Executor returns its canonical unavailable response");
        assert!(matches!(
            response,
            ExecutorPreflightProcess::Rejected(proto::ExecutorErrorCodeV0::Unavailable)
        ));
    }

    #[test]
    fn one_shot_cancellation_signals_the_executor_before_forced_group_cleanup() {
        const CHILD_ENV: &str = "WATERSHED_EXECUTOR_LEADER_CANCELLATION_CHILD";
        if crate::tests::run_isolated_test(CHILD_ENV) {
            return;
        }

        let workspace = crate::tests::empty_workspace();
        let ready = workspace.join("ready");
        let child_signalled = workspace.join("child-signalled");
        let response_path = workspace.join("response.json");
        let request = one_shot_request();
        let response = proto::ExecutorResponseV0::Completed {
            enforcement: proto::EnforcementReceiptV0 {
                applied_policy_digest: request.policy_digest.clone(),
                backend: proto::EXECUTOR_BACKEND_V0.to_owned(),
                backend_version: "test".to_owned(),
                executor: proto::EXECUTOR_NAME_V0.to_owned(),
                executor_version: "test".to_owned(),
                isolation_active: true,
                max_concurrent_processes_and_threads: request
                    .limits
                    .max_concurrent_processes_and_threads,
                platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
                runtime_profile: proto::RuntimeReadProfileV0::Exact,
            },
            request_id: request.request_id.clone(),
            schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
            tool_result: proto::ExecutorToolResultV0 {
                classification: Some(proto::ExecutorToolClassificationV0::Cancelled),
                exit_code: None,
                status: proto::ExecutorToolStatusV0::Cancelled,
                stderr_base64: proto::encode_executor_stream_v0(&[]),
                stdout_base64: proto::encode_executor_stream_v0(&[]),
            },
        };
        std::fs::write(
            &response_path,
            proto::canonical_executor_response_v0(&response).expect("response is canonical"),
        )
        .expect("response fixture is written");
        let preflight = String::from_utf8(
            proto::canonical_executor_preflight_v0(&proto::ExecutorPreflightV0::Ready {
                request_id: request.request_id.clone(),
                schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
            })
            .expect("preflight is canonical"),
        )
        .expect("preflight is UTF-8");
        let script = format!(
            "printf '%s' '{preflight}'\n\
             IFS= read -r _start\n\
             trap \"/bin/sleep 0.1; /bin/cat -- '{response}'; exit 0\" TERM\n\
             (trap \"printf signalled > '{child_signalled}'; exit 0\" TERM; while :; do /bin/sleep 1; done) &\n\
             printf ready > '{ready}'\n\
             while :; do /bin/sleep 1; done\n",
            response = response_path.display(),
            preflight = preflight,
            child_signalled = child_signalled.display(),
            ready = ready.display(),
        );
        let ready_for_interrupt = ready.clone();
        crate::begin_productive_operation().expect("productive operation begins");
        let interrupter = std::thread::spawn(move || {
            let started = Instant::now();
            while !ready_for_interrupt.is_file() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "fake Executor did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
        });

        let executor = File::open("/bin/sh").expect("shell executor opens");
        let preflight = preflight_one_shot(&executor, &[], &request, script.as_bytes())
            .expect("Executor reaches preflight readiness");
        let ExecutorPreflightProcess::Ready(waiting) = preflight else {
            panic!("cancellation fixture must reach readiness")
        };
        let terminal =
            start_one_shot(waiting).expect("Executor returns canonical cancellation evidence");
        interrupter.join().expect("interrupt thread joins");
        crate::settle_productive_operation();

        assert!(matches!(
            terminal,
            proto::ExecutorResponseV0::Completed {
                tool_result: proto::ExecutorToolResultV0 {
                    status: proto::ExecutorToolStatusV0::Cancelled,
                    ..
                },
                ..
            }
        ));
        assert!(
            !child_signalled.exists(),
            "graceful cancellation must reach only the Executor leader"
        );
    }

    #[test]
    fn predefined_policy_id_is_not_compared_to_resolved_sandbox_executable() {
        assert!(validate_executor_executable("/bin/cat").is_ok());
        assert!(validate_executor_executable("registry:agent-read").is_err());
    }

    #[test]
    fn prepared_executor_request_hash_uses_run_log_format() {
        let hash = executor_request_hash(b"canonical Executor request");
        let digest = hash
            .strip_prefix(crate::runtime::digest::SHA256_PREFIX)
            .expect("Executor request hash has the canonical prefix");
        assert!(proto::decode_lowercase_sha256_hex(digest).is_some());
    }

    #[test]
    fn own_script_uses_the_closed_shell_executable() {
        assert!(validate_executor_executable(proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0).is_ok());
    }

    #[test]
    fn executor_descriptor_is_moved_above_mount_slots_under_fd_pressure() {
        let pressure = (0..proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + 8)
            .map(|_| File::open("/dev/null").expect("fd pressure source"))
            .collect::<Vec<_>>();
        let executor = duplicate_executor_descriptor(pressure.last().expect("pressure descriptor"))
            .expect("move Executor descriptor");
        let last_mount_slot = proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 as i32
            + proto::MAX_EXECUTOR_MOUNTS_V0 as i32
            - 1;
        assert!(executor.as_raw_fd() > last_mount_slot);
    }

    #[test]
    fn terminal_receipt_identity_must_match_the_prepared_probe() {
        let probe = proto::ExecutorProbeV0 {
            backend: proto::EXECUTOR_BACKEND_V0.to_owned(),
            backend_version: "1".to_owned(),
            executor: proto::EXECUTOR_NAME_V0.to_owned(),
            executor_version: "1".to_owned(),
            platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
            protocol_versions: vec![proto::EXECUTOR_PROTOCOL_VERSION_V0.to_owned()],
            ready: true,
            runtime_mounts: Vec::new(),
            schema: proto::EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
            supported_policy_features: Vec::new(),
        };
        let mut receipt = proto::EnforcementReceiptV0 {
            applied_policy_digest: "0".repeat(64),
            backend: probe.backend.clone(),
            backend_version: probe.backend_version.clone(),
            executor: probe.executor.clone(),
            executor_version: probe.executor_version.clone(),
            isolation_active: true,
            max_concurrent_processes_and_threads: 16,
            platform: probe.platform.clone(),
            runtime_profile: proto::RuntimeReadProfileV0::Exact,
        };
        assert!(validate_receipt_identity(&receipt, &probe).is_ok());
        receipt.backend_version = "different".to_owned();
        assert!(validate_receipt_identity(&receipt, &probe).is_err());
    }

    #[test]
    fn one_shot_completion_cleans_its_process_group_once() {
        reset_process_group_cleanup_calls_for_test();
        let request = one_shot_request();
        let expected_response = proto::ExecutorPreflightV0::Error {
            code: proto::ExecutorErrorCodeV0::Unavailable,
            message: "unavailable".to_owned(),
            request_id: request.request_id.clone(),
            schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
        };
        let response = String::from_utf8(
            proto::canonical_executor_preflight_v0(&expected_response).expect("canonical response"),
        )
        .expect("response is UTF-8");
        let request_bytes = format!("printf '%s' '{response}'\n").into_bytes();
        let executor = File::open("/bin/sh").expect("shell executor opens");

        assert!(matches!(
            preflight_without_wall_clock_deadline(&executor, &request, &request_bytes)
                .expect("canonical Executor response is accepted"),
            ExecutorPreflightProcess::Rejected(proto::ExecutorErrorCodeV0::Unavailable)
        ));
        assert_eq!(
            process_group_cleanup_calls_for_test(),
            1,
            "a synchronously reaped Executor leader must not be signaled again by ChildGuard"
        );
    }

    #[test]
    fn continuous_executor_output_cannot_starve_its_deadline_or_cleanup() {
        for writer in [
            "exec /bin/cat /dev/zero\n",
            "exec /bin/sh -c '/bin/cat /dev/zero >&2'\n",
        ] {
            reset_process_group_cleanup_calls_for_test();
            let request = one_shot_request();
            let executor = File::open("/bin/sh").expect("shell executor opens");
            let request_bytes = writer.as_bytes();
            let error =
                match preflight_without_wall_clock_deadline(&executor, &request, request_bytes) {
                    Err(error) => error,
                    Ok(_) => panic!("a capped Executor stream is rejected"),
                };
            assert!(error.to_string().contains("byte limit"));
            assert_eq!(
                process_group_cleanup_calls_for_test(),
                1,
                "a capped Executor stream must clean up its process group"
            );
        }
    }

    fn preflight_without_wall_clock_deadline(
        executor: &File,
        request: &proto::ExecutorRequestV0,
        request_bytes: &[u8],
    ) -> Result<ExecutorPreflightProcess, crate::runtime::types::RuntimeError> {
        let now = Instant::now();
        preflight_one_shot_at_deadline(
            executor,
            &[],
            request,
            request_bytes,
            now + Duration::from_secs(1),
            |_| Ok(()),
            || now,
        )
    }

    fn one_shot_request() -> proto::ExecutorRequestV0 {
        proto::ExecutorRequestV0 {
            argv: Vec::new(),
            environment: BTreeMap::new(),
            executable: "/bin/sh".to_owned(),
            limits: proto::ExecutorLimitsV0 {
                max_concurrent_processes_and_threads: 16,
                max_stderr_bytes: 0,
                max_stdout_bytes: 0,
                timeout_ms: 100,
            },
            mounts: Vec::new(),
            resolved_policy: proto::ExecutorResolvedPolicyV0 {
                artifact: serde_json::json!({}),
                command: serde_json::json!({}),
                limits: proto::ExecutorLimitsV0 {
                    max_concurrent_processes_and_threads: 16,
                    max_stderr_bytes: 0,
                    max_stdout_bytes: 0,
                    timeout_ms: 100,
                },
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
}
