use crate::{
    backend::ProbeState,
    lifecycle::{CleanupAction, CleanupController},
    protocol,
};
use proto::{
    ExecutorLimitsV0, ExecutorObjectKindV0, ExecutorProtectedObjectV0, ExecutorRequestV0,
    ExecutorResolvedPolicyV0, UnixObjectIdentityV0, resolved_policy_digest_v0,
};
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
    time::Instant,
};

#[test]
fn cleanup_controller_orders_term_kill_reap_and_output_drain() {
    let started = Instant::now();
    let term_deadline = started + proto::TOOL_TERMINATION_GRACE_V0;
    let forced_deadline = term_deadline + proto::TOOL_FORCED_REAP_DEADLINE_V0;
    let mut forced = CleanupController::new(started);

    assert_eq!(
        forced.advance(
            term_deadline - std::time::Duration::from_nanos(1),
            false,
            false
        ),
        CleanupAction::Wait
    );
    assert_eq!(
        forced.advance(term_deadline, false, false),
        CleanupAction::ForceKill
    );
    assert_eq!(
        forced.advance(
            forced_deadline - std::time::Duration::from_nanos(1),
            false,
            false
        ),
        CleanupAction::Wait
    );
    assert_eq!(
        forced.advance(forced_deadline, false, false),
        CleanupAction::FailClosed
    );

    let cleanup_finished = started + std::time::Duration::from_millis(500);
    let drain_deadline = cleanup_finished + proto::TOOL_OUTPUT_DRAIN_DEADLINE_V0;
    let mut drained = CleanupController::new(started);
    assert_eq!(
        drained.advance(cleanup_finished, true, false),
        CleanupAction::Wait
    );
    assert_eq!(
        drained.advance(
            drain_deadline - std::time::Duration::from_nanos(1),
            true,
            false
        ),
        CleanupAction::Wait
    );
    assert_eq!(
        drained.advance(drain_deadline, true, false),
        CleanupAction::OutputDrainTimeout
    );

    let mut complete = CleanupController::new(started);
    assert_eq!(
        complete.advance(cleanup_finished, true, true),
        CleanupAction::Complete
    );
}

#[test]
fn protocol_probe_is_one_canonical_document() {
    if run_isolated("WATERSHED_EXECUTOR_PROTOCOL_PROBE_CHILD") {
        return;
    }
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    protocol::run_with_diagnostics(
        &["--probe".to_owned()],
        Cursor::new([]),
        &mut output,
        &mut diagnostics,
    )
    .expect("probe writes");

    let probe = proto::parse_executor_probe_v0(&output).expect("probe is exact protocol JSON");
    assert_eq!(probe.schema, proto::EXECUTOR_PROBE_SCHEMA_V0);
    assert_eq!(probe.executor, proto::EXECUTOR_NAME_V0);
    assert_eq!(probe.backend, crate::backend::BACKEND);
    assert_eq!(probe.platform, crate::backend::PLATFORM);
    assert_eq!(
        output,
        proto::canonical_executor_probe_v0(&probe).expect("probe canonicalizes")
    );
    assert_eq!(diagnostics.is_empty(), probe.ready);
    assert_eq!(
        probe.supported_policy_features,
        if probe.ready {
            vec![proto::EXECUTOR_FEATURE_SELF_PROTECTION_V0.to_owned()]
        } else {
            Vec::new()
        }
    );
}

#[test]
fn readiness_diagnostic_is_single_line_sanitized_and_bounded() {
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let reason = format!(
        "first\n\t\0{}",
        "x".repeat(protocol::MAX_READINESS_DIAGNOSTIC_BYTES * 2)
    );

    protocol::write_probe(
        ProbeState {
            backend_version: "unavailable".to_owned(),
            ready: false,
            features: Vec::new(),
            readiness_error: Some(reason),
        },
        &mut output,
        &mut diagnostics,
    )
    .expect("probe and diagnostic write");

    let probe = proto::parse_executor_probe_v0(&output).expect("probe remains canonical");
    assert!(!probe.ready);
    assert_eq!(
        output,
        proto::canonical_executor_probe_v0(&probe).expect("probe canonicalizes")
    );
    assert!(diagnostics.len() <= protocol::MAX_READINESS_DIAGNOSTIC_BYTES);
    let diagnostic = String::from_utf8(diagnostics).expect("diagnostic is UTF-8");
    assert!(diagnostic.starts_with("flow-executor readiness: first "));
    assert_eq!(diagnostic.lines().count(), 1);
    assert!(!diagnostic.contains('\0'));
}

#[test]
fn protocol_rejects_oversized_and_malformed_requests_without_output() {
    let mut output = Vec::new();
    let oversized = vec![b' '; proto::MAX_EXECUTOR_REQUEST_BYTES_V0 + 1];

    let oversized_error = protocol::run_with(&[], Cursor::new(oversized), &mut output)
        .expect_err("oversized request must fail before dispatch");
    assert!(oversized_error.contains("exceeds its byte limit"));
    assert!(output.is_empty());

    let malformed_error = protocol::run_with(&[], Cursor::new(b"{\n"), &mut output)
        .expect_err("malformed request must fail before dispatch");
    assert!(!malformed_error.is_empty());
    assert!(output.is_empty());
}

#[derive(Default)]
struct FlushTrackingOutput {
    bytes: Vec<u8>,
    flushes: usize,
}

impl Write for FlushTrackingOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn absent_or_invalid_start_never_dispatches_after_a_flushed_ready() {
    let cases = [
        Vec::new(),
        vec![b' '; proto::MAX_EXECUTOR_CONTROL_BYTES_V0 + 1],
        b"{\"request_id\":\"other\",\"schema\":\"flow-executor-start-v0\"}\n".to_vec(),
        b"{\"request_id\":\"request-1\",\"schema\":\"flow-executor-start-v0\",\"start\":true}\n"
            .to_vec(),
        b"{\"request_id\":\"request-1\",\"schema\":\"flow-executor-start-v0\"}\n{\"request_id\":\"request-1\",\"schema\":\"flow-executor-start-v0\"}\n"
            .to_vec(),
    ];

    for start in cases {
        let mut output = FlushTrackingOutput::default();
        let mut dispatched = false;
        let error =
            protocol::complete_preflight(Cursor::new(start), &mut output, "request-1", || {
                dispatched = true;
                unreachable!("invalid start must not dispatch")
            })
            .expect_err("missing or invalid start aborts");

        assert!(!error.is_empty());
        assert!(!dispatched);
        assert_eq!(output.flushes, 1, "Ready must be flushed before waiting");
        assert!(matches!(
            proto::parse_executor_preflight_v0(&output.bytes, "request-1").unwrap(),
            proto::ExecutorPreflightV0::Ready { .. }
        ));
    }
}

#[test]
fn one_exact_start_dispatches_once_and_flushes_the_bound_receipt() {
    let request = request();
    let start = proto::canonical_executor_start_v0(&proto::ExecutorStartV0 {
        request_id: request.request_id.clone(),
        schema: proto::EXECUTOR_START_SCHEMA_V0.to_owned(),
    })
    .expect("Start canonicalizes");
    let response = proto::ExecutorResponseV0::Completed {
        enforcement: proto::EnforcementReceiptV0 {
            applied_policy_digest: request.policy_digest.clone(),
            backend: crate::backend::BACKEND.to_owned(),
            backend_version: "test".to_owned(),
            executor: proto::EXECUTOR_NAME_V0.to_owned(),
            executor_version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: crate::backend::PLATFORM.to_owned(),
            self_protection_active: true,
        },
        request_id: request.request_id.clone(),
        schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
        tool_result: proto::ExecutorToolResultV0 {
            classification: None,
            exit_code: Some(0),
            status: proto::ExecutorToolStatusV0::Completed,
            stderr_base64: String::new(),
            stdout_base64: proto::encode_executor_stream_v0(b"ok\n"),
        },
    };
    let mut output = FlushTrackingOutput::default();
    let mut dispatches = 0;
    protocol::complete_preflight(Cursor::new(start), &mut output, &request.request_id, || {
        dispatches += 1;
        Ok(response.clone())
    })
    .expect("exact Start dispatches");
    assert_eq!(dispatches, 1);
    assert_eq!(
        output.flushes, 2,
        "Ready and the terminal response are flushed"
    );
    let ready = proto::canonical_executor_preflight_v0(&proto::ExecutorPreflightV0::Ready {
        request_id: request.request_id.clone(),
        schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
    })
    .unwrap();
    assert!(output.bytes.starts_with(&ready));
    let terminal = proto::parse_executor_response_v0(
        &output.bytes[ready.len()..],
        &request.request_id,
        &request.policy_digest,
    )
    .expect("terminal response is bound to the exact request");
    assert_eq!(terminal, response);
}

#[test]
fn changed_invocation_or_protected_objects_fail_before_readiness() {
    type Mutate = fn(&mut ExecutorResolvedPolicyV0);
    let cases: [(&str, Mutate); 12] = [
        ("argv", |policy| {
            policy.argv.push("different argument".to_owned())
        }),
        ("environment", |policy| {
            policy
                .environment
                .insert("DECLARED".to_owned(), "different".to_owned());
        }),
        ("executable", |policy| {
            policy.executable = "/opt/tool/bin/tool".to_owned()
        }),
        ("working directory", |policy| {
            policy.working_directory = "/other-workspace".to_owned()
        }),
        ("timeout", |policy| policy.limits.timeout_ms += 1),
        ("stdout bound", |policy| policy.limits.max_stdout_bytes += 1),
        ("stderr bound", |policy| policy.limits.max_stderr_bytes += 1),
        ("protected path", |policy| {
            policy.protected_objects[0].path = "/other-owned".to_owned()
        }),
        ("protected identity", |policy| {
            policy.protected_objects[0].identity.inode += 1
        }),
        ("protected kind", |policy| {
            policy.protected_objects[0].identity.kind = ExecutorObjectKindV0::File
        }),
        ("Tool id", |policy| policy.tool_id = "different".to_owned()),
        ("Tool kind", |policy| {
            policy.tool_kind = "own-script".to_owned()
        }),
    ];
    let original = request();
    proto::canonical_executor_request_v0(&original).expect("baseline request is valid");
    for (name, mutate) in cases {
        let mut altered = original.clone();
        mutate(&mut altered.resolved_policy);
        let error = rejected_request(&altered);
        assert!(error.contains("digest does not match"), "{name}: {error}");
    }
}

#[test]
fn legacy_security_and_duplicate_invocation_fields_fail_before_readiness() {
    let original = request();
    for (field, value) in [
        ("argv", serde_json::json!(["duplicate"])),
        ("mounts", serde_json::json!([])),
        ("runtime_profile", serde_json::json!("exact")),
    ] {
        let mut document = serde_json::to_value(&original).unwrap();
        document[field] = value;
        assert_rejected_document(&document);
    }
    for (field, value) in [
        ("artifact", serde_json::json!({})),
        ("command", serde_json::json!({})),
        ("mounts", serde_json::json!([])),
        ("runtime_profile", serde_json::json!("host-system-read")),
    ] {
        let mut document = serde_json::to_value(&original).unwrap();
        document["resolved_policy"][field] = value;
        assert_rejected_document(&document);
    }
    let mut document = serde_json::to_value(&original).unwrap();
    document["resolved_policy"]["limits"]["max_concurrent_processes_and_threads"] = 16.into();
    assert_rejected_document(&document);
}

#[test]
fn protected_objects_are_nonempty_unique_and_bounded_before_readiness() {
    let mut exact = request();
    exact.resolved_policy.protected_objects = (0..proto::MAX_EXECUTOR_PROTECTED_OBJECTS_V0)
        .map(|index| ExecutorProtectedObjectV0 {
            descriptor: proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + index as u32,
            path: format!("/flow-owned-{index}"),
            identity: UnixObjectIdentityV0 {
                device: 1,
                inode: index as u64 + 1,
                kind: ExecutorObjectKindV0::Directory,
            },
        })
        .collect();
    bind_digest(&mut exact);
    proto::canonical_executor_request_v0(&exact)
        .expect("the full protected descriptor capacity is representable");

    let mut overflow = exact.clone();
    let mut object = overflow
        .resolved_policy
        .protected_objects
        .last()
        .unwrap()
        .clone();
    object.descriptor += 1;
    object.path = "/one-too-many".to_owned();
    overflow.resolved_policy.protected_objects.push(object);
    let mut empty = request();
    empty.resolved_policy.protected_objects.clear();
    let mut duplicate = request();
    let mut object = duplicate.resolved_policy.protected_objects[0].clone();
    object.descriptor += 1;
    duplicate.resolved_policy.protected_objects.push(object);
    let mut wrong_slot = request();
    wrong_slot.resolved_policy.protected_objects[0].descriptor += 1;
    let mut relative = request();
    relative.resolved_policy.protected_objects[0].path = "relative-owned".to_owned();
    for mut invalid in [overflow, empty, duplicate, wrong_slot, relative] {
        bind_digest(&mut invalid);
        assert!(!rejected_request(&invalid).is_empty());
    }
}

#[test]
fn response_stream_limits_cannot_exceed_the_protocol_capacity() {
    for stdout in [true, false] {
        for limit in [0, proto::MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1] {
            let mut invalid = request();
            if stdout {
                invalid.resolved_policy.limits.max_stdout_bytes = limit;
            } else {
                invalid.resolved_policy.limits.max_stderr_bytes = limit;
            }
            bind_digest(&mut invalid);
            assert!(rejected_request(&invalid).contains("limits"));
        }
    }
}

fn request() -> ExecutorRequestV0 {
    let resolved_policy = ExecutorResolvedPolicyV0 {
        argv: vec!["literal host argument".to_owned()],
        environment: BTreeMap::from([("DECLARED".to_owned(), "literal value".to_owned())]),
        executable: "/bin/echo".to_owned(),
        limits: ExecutorLimitsV0 {
            max_stderr_bytes: 1_024,
            max_stdout_bytes: 1_024,
            timeout_ms: 1_000,
        },
        protected_objects: vec![ExecutorProtectedObjectV0 {
            descriptor: proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0,
            path: "/flow-owned".to_owned(),
            identity: UnixObjectIdentityV0 {
                device: 1,
                inode: 1,
                kind: ExecutorObjectKindV0::Directory,
            },
        }],
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
        working_directory: "/workspace".to_owned(),
    };
    ExecutorRequestV0 {
        policy_digest: resolved_policy_digest_v0(&resolved_policy).expect("policy digest"),
        resolved_policy,
        request_id: "request-1".to_owned(),
        schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
    }
}

fn bind_digest(request: &mut ExecutorRequestV0) {
    request.policy_digest =
        resolved_policy_digest_v0(&request.resolved_policy).expect("policy digest");
}

fn rejected_request(request: &ExecutorRequestV0) -> String {
    assert_rejected_document(&serde_json::to_value(request).unwrap())
}

fn assert_rejected_document(document: &serde_json::Value) -> String {
    // Raw JSON must reach the entry point; the canonical encoder already rejects these requests.
    let mut input = serde_json::to_vec(document).unwrap();
    input.push(b'\n');
    let mut output = Vec::new();
    let error = protocol::run_with(&[], Cursor::new(input), &mut output)
        .expect_err("invalid request must fail before dispatch");
    assert!(output.is_empty(), "invalid request must not publish Ready");
    error
}

pub(crate) fn run_isolated(child_env: &str) -> bool {
    if std::env::var_os(child_env).is_some() {
        return false;
    }
    let name = std::thread::current()
        .name()
        .expect("test thread has a name")
        .to_owned();
    let output =
        std::process::Command::new(std::env::current_exe().expect("test executable resolves"))
            .args(["--exact", &name, "--nocapture"])
            .env(child_env, "1")
            .output()
            .expect("isolated test starts");
    assert!(
        output.status.success(),
        "isolated test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
