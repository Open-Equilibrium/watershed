use super::{
    ExecutorSelection,
    selection::{protected_directories, resolve_executor_with_roots},
};
use crate::runtime::{
    fs_guards::AnchoredWorkspace,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::RuntimeError,
};
use preparation::{ProtectedObject, retain_protected_objects};
use response::{decode_tool_outcome, validate_receipt_identity};
use transport::{ExecutorPreflightProcess, preflight_one_shot, start_one_shot};

#[cfg(test)]
mod deadline_tests;
mod preparation;
mod response;
mod transport;

/// Definitive result of one bounded Executor dispatch.
pub(crate) enum ExecutorDispatchOutcome {
    Completed(Box<ExecutorToolExecution>),
    Error(proto::ExecutorErrorCodeV0),
}

/// Result of validating one exact Tool request in its retained one-shot Executor.
pub(crate) enum ExecutorPreflightOutcome {
    Ready(Box<PreparedExecutorWaiting>),
    Rejected(proto::ExecutorErrorCodeV0),
}

/// Validated result and self-protection evidence from one Tool execution.
pub(crate) struct ExecutorToolExecution {
    pub(crate) enforcement: proto::EnforcementReceiptV0,
    pub(crate) outcome: ToolExecutionOutcome,
    pub(crate) request_hash: String,
}

/// Fully validated request with protected objects retained before recovery or dispatch.
pub(crate) struct PreparedExecutorTool {
    request_hash: String,
    protected_descriptors: Vec<std::os::fd::OwnedFd>,
    request: proto::ExecutorRequestV0,
    request_bytes: Vec<u8>,
}

/// One validated Tool request waiting for explicit start in its retained Executor process.
pub(crate) struct PreparedExecutorWaiting {
    prepared: PreparedExecutorTool,
    process: transport::WaitingExecutor,
}

impl PreparedExecutorTool {
    pub(crate) fn request_hash(&self) -> &str {
        &self.request_hash
    }

    pub(crate) fn policy_digest(&self) -> &str {
        &self.request.policy_digest
    }
}

/// Ready one-shot Executor and this installation's admitted protected objects.
pub(crate) struct PreparedExecutor {
    selection: ExecutorSelection,
    protected_objects: Vec<ProtectedObject>,
    _protected_directories: Vec<crate::runtime::fs_guards::AnchoredDir>,
}

impl PreparedExecutor {
    /// Resolves the selected Executor and retains the admitted protected installation.
    pub(crate) fn prepare_selected() -> Result<Self, RuntimeError> {
        let protected_directories = protected_directories(true)?;
        let selection = resolve_executor_with_roots(&protected_directories)?;
        let protected_objects = retain_protected_objects(&selection, &protected_directories)?;
        Ok(Self {
            selection,
            protected_objects,
            _protected_directories: protected_directories,
        })
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
        self.prepare_tool_native(workspace, policy, command_policy, invocation, request_id)
    }

    /// Launches exactly one Executor and validates the prepared request without starting its Tool.
    pub(crate) fn preflight_prepared(
        &self,
        prepared: PreparedExecutorTool,
    ) -> Result<ExecutorPreflightOutcome, RuntimeError> {
        let process = preflight_one_shot(
            self.selection.executable(),
            &prepared.protected_descriptors,
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
            ExecutorPreflightProcess::Rejected(code) => ExecutorPreflightOutcome::Rejected(code),
        })
    }

    /// Explicitly starts a Tool in its already validated one-shot Executor.
    pub(crate) fn start_prepared(
        &self,
        waiting: PreparedExecutorWaiting,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError> {
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

    /// Runs both stages immediately for non-productive conformance and startup evidence.
    #[cfg(any(test, feature = "m12-startup-evidence"))]
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
        proto::validate_enforcement_receipt_v0(receipt, prepared.policy_digest()).map_err(
            |_| {
                RuntimeError::executor(
                    proto::ExecutorErrorCodeV0::InvalidResponse,
                    "Executor enforcement receipt does not match its prepared request",
                )
            },
        )?;
        validate_receipt_identity(receipt, self.selection.probe())
    }
}

fn invalid_request(_: proto::ExecutorProtocolError) -> RuntimeError {
    invalid_response("Flow constructed an invalid Executor protocol document")
}

fn invalid_response(message: impl Into<String>) -> RuntimeError {
    executor_error(proto::ExecutorErrorCodeV0::InvalidResponse, message)
}

fn runtime_open_error(_: rustix::io::Errno) -> RuntimeError {
    executor_error(
        proto::ExecutorErrorCodeV0::PolicyUnsupported,
        "Executor protected object could not be retained",
    )
}

fn executor_error(code: proto::ExecutorErrorCodeV0, message: impl Into<String>) -> RuntimeError {
    RuntimeError::executor(code, message)
}

#[cfg(test)]
mod tests {
    use super::super::process::{
        process_group_cleanup_calls_for_test, reset_process_group_cleanup_calls_for_test,
    };
    use super::preparation::{ProtectedObject, executor_request_hash};
    use super::response::{decode_tool_outcome, validate_receipt_identity};
    use super::transport::{
        ExecutorPreflightProcess, c_close, preflight_one_shot, preflight_one_shot_at_deadline,
        start_one_shot,
    };
    use super::{ExecutorSelection, PreparedExecutor};
    use crate::runtime::run_attempts::{RunAttemptOutcome, ToolTerminalClassification};
    use crate::runtime::{
        executor::ExecutorSelectionSource, fs_guards::AnchoredWorkspace,
        tool_runner::ToolInvocation,
    };
    use std::{
        collections::BTreeMap,
        fs::File,
        os::fd::{AsRawFd as _, OwnedFd},
        os::unix::{fs::MetadataExt as _, process::CommandExt as _},
        path::Path,
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
    fn prepared_request_binds_host_invocation_and_retained_protected_objects() {
        if crate::tests::run_isolated_test("WATERSHED_EXECUTOR_BINDING_CHILD") {
            return;
        }
        const ALLOWED: &str = "WATERSHED_EXECUTOR_ALLOWED_FIXTURE";
        const OMITTED: &str = "WATERSHED_EXECUTOR_OMITTED_FIXTURE";
        // This test owns its process; these values contain only synthetic fixture data.
        unsafe {
            std::env::set_var(ALLOWED, "literal environment value");
            std::env::set_var(OMITTED, "must not be forwarded");
        }
        let root = crate::tests::empty_workspace();
        let owned_file = root.join("flow-owned");
        std::fs::write(&owned_file, b"owned fixture").expect("protected file is staged");
        let workspace = AnchoredWorkspace::open(&root).expect("workspace opens");
        let mut executor =
            preparation_fixture(vec![retained_object(&root), retained_object(&owned_file)]);
        let mut policy = executor_policy();
        policy.commands[0].environment.allow = vec![ALLOWED.to_owned()];
        let invocation = ToolInvocation {
            executable: "/bin/echo".to_owned(),
            argv: vec![
                "literal argument".to_owned(),
                String::new(),
                "$HOME;\nnext".to_owned(),
            ],
        };
        let prepare = |executor: &PreparedExecutor, invocation: &ToolInvocation| {
            executor
                .prepare_tool(
                    &workspace,
                    &policy,
                    &policy.commands[0],
                    invocation,
                    "binding-request",
                )
                .expect("host request is prepared")
        };
        let first = prepare(&executor, &invocation);
        let resolved = &first.request.resolved_policy;
        assert_eq!(resolved.argv, invocation.argv);
        assert_eq!(resolved.executable, invocation.executable);
        assert_eq!(
            resolved.working_directory,
            workspace.canonical_path().to_str().unwrap()
        );
        assert_eq!(
            resolved.environment,
            BTreeMap::from([(ALLOWED.to_owned(), "literal environment value".to_owned())])
        );
        assert_eq!(resolved.tool_id, policy.commands[0].tool_id);
        assert_eq!(resolved.tool_kind, policy.commands[0].tool_kind.as_str());
        assert_eq!(resolved.limits.timeout_ms, policy.runtime_limits.timeout_ms);
        assert_eq!(
            resolved.limits.max_stdout_bytes,
            crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES as u64
        );
        assert_eq!(
            resolved.limits.max_stderr_bytes,
            crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES as u64
        );
        assert_eq!(resolved.protected_objects.len(), 2);
        assert_eq!(
            first.protected_descriptors.len(),
            resolved.protected_objects.len()
        );
        for (index, (object, descriptor)) in resolved
            .protected_objects
            .iter()
            .zip(&first.protected_descriptors)
            .enumerate()
        {
            let retained = &executor.protected_objects[index];
            assert_eq!(
                object.descriptor,
                proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + index as u32
            );
            assert_eq!(object.path, retained.path);
            assert_eq!(object.identity, retained.identity);
            assert_ne!(descriptor.as_raw_fd(), retained.descriptor.as_raw_fd());
            let metadata = File::from(
                descriptor
                    .try_clone()
                    .expect("protected descriptor duplicates"),
            )
            .metadata()
            .expect("duplicated protected descriptor is open");
            assert_eq!(metadata.dev(), object.identity.device);
            assert_eq!(metadata.ino(), object.identity.inode);
        }
        assert_eq!(
            first.policy_digest(),
            proto::resolved_policy_digest_v0(resolved).unwrap()
        );
        assert_eq!(
            first.request_bytes,
            proto::canonical_executor_request_v0(&first.request).unwrap()
        );
        assert_eq!(
            first.request_hash(),
            executor_request_hash(&first.request_bytes)
        );

        let changed_invocation = ToolInvocation {
            executable: invocation.executable.clone(),
            argv: vec!["different invocation".to_owned()],
        };
        let changed = prepare(&executor, &changed_invocation);
        assert_ne!(first.policy_digest(), changed.policy_digest());
        assert_ne!(first.request_hash(), changed.request_hash());
        assert_eq!(
            resolved.protected_objects,
            changed.request.resolved_policy.protected_objects
        );

        executor
            .protected_objects
            .pop()
            .expect("file is removed from this synthetic selection");
        let changed = prepare(&executor, &invocation);
        assert_ne!(first.policy_digest(), changed.policy_digest());
        assert_ne!(first.request_hash(), changed.request_hash());
        assert_eq!(changed.request.resolved_policy.argv, invocation.argv);
        assert_eq!(changed.protected_descriptors.len(), 1);
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

        let (request, protected_descriptors) = one_shot_request();
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

        let response = preflight_one_shot(
            &executor,
            &protected_descriptors,
            &request,
            script.as_bytes(),
        )
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
        let (request, protected_descriptors) = one_shot_request();
        let response = proto::ExecutorResponseV0::Completed {
            enforcement: proto::EnforcementReceiptV0 {
                applied_policy_digest: request.policy_digest.clone(),
                backend: format!("fake-{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                backend_version: "test".to_owned(),
                executor: proto::EXECUTOR_NAME_V0.to_owned(),
                executor_version: "test".to_owned(),
                platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                self_protection_active: true,
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
        let preflight = preflight_one_shot(
            &executor,
            &protected_descriptors,
            &request,
            script.as_bytes(),
        )
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
    fn prepared_request_uses_host_executable_paths_without_a_legacy_allowlist() {
        let root = crate::tests::empty_workspace();
        let workspace = AnchoredWorkspace::open(&root).expect("workspace opens");
        let executor = preparation_fixture(vec![retained_object(&root)]);
        let policy = executor_policy();
        for (executable, accepted) in [
            ("/bin/echo", true),
            ("/opt/node/bin/node", true),
            ("registry:agent-echo", false),
            ("relative-executable", false),
            ("/bin/../bin/echo", false),
        ] {
            let invocation = ToolInvocation {
                executable: executable.to_owned(),
                argv: vec!["literal host argument".to_owned()],
            };
            let result = executor.prepare_tool(
                &workspace,
                &policy,
                &policy.commands[0],
                &invocation,
                "host-path",
            );
            if accepted {
                let prepared = result.expect("canonical host executable is accepted");
                assert_eq!(prepared.request.resolved_policy.executable, executable);
                assert_eq!(prepared.request.resolved_policy.argv, invocation.argv);
            } else {
                assert!(
                    result.is_err(),
                    "{executable} is not a canonical host executable"
                );
            }
        }
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
    fn own_script_preparation_preserves_the_exact_shell_invocation() {
        let root = crate::tests::empty_workspace();
        let workspace = AnchoredWorkspace::open(&root).expect("workspace opens");
        let executor = preparation_fixture(vec![retained_object(&root)]);
        let mut policy = executor_policy();
        let command = &mut policy.commands[0];
        command.argv.clear();
        command.command_id = core_script::own_script_command_id(&command.tool_id);
        command.executable = "runner:posix-sh".to_owned();
        command.script_runtime = Some(core_script::ScriptRuntime::PosixSh);
        command.tool_kind = core_policy::ToolKind::OwnScript;
        policy.validate().expect("own-script policy is valid");
        let invocation = ToolInvocation {
            executable: proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0.to_owned(),
            argv: vec![
                "-c".to_owned(),
                "printf '%s' \"$1\"".to_owned(),
                "script".to_owned(),
                "literal; argument".to_owned(),
            ],
        };
        let prepared = executor
            .prepare_tool(
                &workspace,
                &policy,
                &policy.commands[0],
                &invocation,
                "own-script",
            )
            .expect("own-script request is prepared");
        assert_eq!(
            prepared.request.resolved_policy.executable,
            invocation.executable
        );
        assert_eq!(prepared.request.resolved_policy.argv, invocation.argv);
        assert_eq!(prepared.request.resolved_policy.tool_kind, "own-script");
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn executor_descriptor_is_moved_above_protected_slots_under_fd_pressure() {
        use super::transport::duplicate_executor_descriptor;
        let pressure = (0..proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + 8)
            .map(|_| File::open("/dev/null").expect("fd pressure source"))
            .collect::<Vec<_>>();
        let executor = duplicate_executor_descriptor(pressure.last().expect("pressure descriptor"))
            .expect("move Executor descriptor");
        let last_protected_slot = proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 as i32
            + proto::MAX_EXECUTOR_PROTECTED_OBJECTS_V0 as i32
            - 1;
        assert!(executor.as_raw_fd() > last_protected_slot);
    }

    #[test]
    fn terminal_receipt_identity_must_match_the_prepared_probe() {
        let probe = proto::ExecutorProbeV0 {
            backend: format!("fake-{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            backend_version: "1".to_owned(),
            executor: proto::EXECUTOR_NAME_V0.to_owned(),
            executor_version: "1".to_owned(),
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            protocol_versions: vec![proto::EXECUTOR_PROTOCOL_VERSION_V0.to_owned()],
            ready: true,
            schema: proto::EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
            supported_policy_features: vec![proto::EXECUTOR_FEATURE_SELF_PROTECTION_V0.to_owned()],
        };
        let receipt = proto::EnforcementReceiptV0 {
            applied_policy_digest: "0".repeat(64),
            backend: probe.backend.clone(),
            backend_version: probe.backend_version.clone(),
            executor: probe.executor.clone(),
            executor_version: probe.executor_version.clone(),
            platform: probe.platform.clone(),
            self_protection_active: true,
        };
        assert!(validate_receipt_identity(&receipt, &probe).is_ok());
        for field in [
            "backend",
            "backend_version",
            "executor",
            "executor_version",
            "platform",
        ] {
            let mut altered = serde_json::to_value(&receipt).expect("receipt serializes");
            altered[field] = "different".into();
            let altered =
                serde_json::from_value(altered).expect("altered receipt has the same shape");
            assert!(
                validate_receipt_identity(&altered, &probe).is_err(),
                "{field} must match"
            );
        }
    }

    #[test]
    fn one_shot_completion_cleans_its_process_group_once() {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        reset_process_group_cleanup_calls_for_test();
        let (request, protected_descriptors) = one_shot_request();
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
        let root = crate::tests::empty_workspace();
        let path = root.join("executor");
        fs::copy("/bin/sh", &path).expect("private shell executor is copied");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("private shell executor is executable");
        let executor = File::open(&path).expect("shell executor opens");
        let parent_flags =
            rustix::io::fcntl_getfd(&executor).expect("parent descriptor flags read");
        assert!(parent_flags.contains(rustix::io::FdFlags::CLOEXEC));
        fs::rename(&path, root.join("retained-executor"))
            .expect("opened shell executor is renamed");
        fs::write(&path, b"invalid replacement executable\n").expect("old path is replaced");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("replacement is not executable");

        assert!(matches!(
            preflight_without_wall_clock_deadline(
                &executor,
                &protected_descriptors,
                &request,
                &request_bytes
            )
            .expect("canonical Executor response is accepted"),
            ExecutorPreflightProcess::Rejected(proto::ExecutorErrorCodeV0::Unavailable)
        ));
        assert_eq!(
            rustix::io::fcntl_getfd(&executor).expect("parent descriptor remains open"),
            parent_flags,
            "launch must preserve the parent's close-on-exec protection"
        );
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
            let (request, protected_descriptors) = one_shot_request();
            let executor = File::open("/bin/sh").expect("shell executor opens");
            let request_bytes = writer.as_bytes();
            let error = match preflight_without_wall_clock_deadline(
                &executor,
                &protected_descriptors,
                &request,
                request_bytes,
            ) {
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
        protected_descriptors: &[OwnedFd],
        request: &proto::ExecutorRequestV0,
        request_bytes: &[u8],
    ) -> Result<ExecutorPreflightProcess, crate::runtime::types::RuntimeError> {
        let now = Instant::now();
        preflight_one_shot_at_deadline(
            executor,
            protected_descriptors,
            request,
            request_bytes,
            now + Duration::from_secs(1),
            |_| Ok(()),
            || now,
        )
    }

    pub(super) fn one_shot_request() -> (proto::ExecutorRequestV0, Vec<OwnedFd>) {
        let protected = retained_object(Path::new("/bin/sh"));
        let resolved_policy = proto::ExecutorResolvedPolicyV0 {
            argv: Vec::new(),
            environment: BTreeMap::new(),
            executable: "/bin/sh".to_owned(),
            limits: proto::ExecutorLimitsV0 {
                max_stderr_bytes: 1,
                max_stdout_bytes: 1,
                timeout_ms: 100,
            },
            protected_objects: vec![proto::ExecutorProtectedObjectV0 {
                descriptor: proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0,
                identity: protected.identity,
                path: protected.path,
            }],
            tool_id: "tool".to_owned(),
            tool_kind: "own-script".to_owned(),
            working_directory: "/tmp".to_owned(),
        };
        let request = proto::ExecutorRequestV0 {
            policy_digest: proto::resolved_policy_digest_v0(&resolved_policy)
                .expect("policy has a digest"),
            resolved_policy,
            request_id: "request-1".to_owned(),
            schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
        };
        proto::canonical_executor_request_v0(&request)
            .expect("one-shot metadata has a valid wire shape");
        (request, vec![protected.descriptor])
    }

    fn retained_object(path: &Path) -> ProtectedObject {
        let path = std::fs::canonicalize(path).expect("protected path is canonical");
        let file = File::open(&path).expect("protected object opens");
        let metadata = file.metadata().expect("protected identity reads");
        ProtectedObject {
            descriptor: file.into(),
            identity: proto::UnixObjectIdentityV0 {
                device: metadata.dev(),
                inode: metadata.ino(),
                kind: if metadata.is_dir() {
                    proto::ExecutorObjectKindV0::Directory
                } else {
                    proto::ExecutorObjectKindV0::File
                },
            },
            path: path.to_str().expect("fixture path is UTF-8").to_owned(),
        }
    }

    fn preparation_fixture(protected_objects: Vec<ProtectedObject>) -> PreparedExecutor {
        // Request preparation needs retained objects, but never probes or launches this selection.
        PreparedExecutor {
            selection: ExecutorSelection::new(
                "/unused-executor".into(),
                ExecutorSelectionSource::Custom,
            ),
            protected_objects,
            _protected_directories: Vec::new(),
        }
    }

    fn executor_policy() -> core_policy::PolicyArtifact {
        let policy = core_policy::PolicyArtifact {
            commands: vec![core_policy::CommandPolicy {
                allowed_parameters: Vec::new(),
                argv: vec!["configured argument".to_owned()],
                command_id: "agent-echo".to_owned(),
                environment: core_policy::EnvironmentPolicy {
                    allow: Vec::new(),
                    default: core_policy::EnvironmentDefault::Clear,
                },
                executable: "registry:agent-echo".to_owned(),
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
        };
        policy.validate().expect("preparation policy is valid");
        policy
    }
}
