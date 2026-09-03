use crate::{
    EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0, EXECUTOR_PREFLIGHT_SCHEMA_V0, EXECUTOR_PROBE_SCHEMA_V0,
    EXECUTOR_REQUEST_SCHEMA_V0, EXECUTOR_RESPONSE_SCHEMA_V0, EXECUTOR_START_SCHEMA_V0,
    EnforcementReceiptV0, ExecutorErrorCodeV0, ExecutorLimitsV0, ExecutorMountAccessV0,
    ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0, ExecutorPreflightV0,
    ExecutorProbeV0, ExecutorRequestV0, ExecutorResolvedMountV0, ExecutorResolvedPolicyV0,
    ExecutorResponseV0, ExecutorRuntimeMountV0, ExecutorStartV0, ExecutorToolClassificationV0,
    ExecutorToolResultV0, ExecutorToolStatusV0, MAX_EXECUTOR_CONTROL_BYTES_V0,
    MAX_EXECUTOR_EXEC_VECTOR_BYTES_V0, MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0, MAX_EXECUTOR_MOUNTS_V0,
    MAX_EXECUTOR_REQUEST_BYTES_V0, MAX_EXECUTOR_RESPONSE_BYTES_V0,
    MAX_EXECUTOR_TOOL_STREAM_BYTES_V0, MAX_EXECUTOR_WORKSPACE_MOUNTS_V0, RuntimeReadProfileV0,
    TOOL_FORCED_REAP_DEADLINE_V0, TOOL_OUTPUT_DRAIN_DEADLINE_V0, TOOL_TERMINATION_GRACE_V0,
    UnixObjectIdentityV0, canonical_executor_preflight_v0, canonical_executor_probe_v0,
    canonical_executor_request_v0, canonical_executor_response_v0, canonical_executor_start_v0,
    decode_executor_stream_v0, encode_executor_stream_v0, parse_executor_preflight_v0,
    parse_executor_probe_v0, parse_executor_request_v0, parse_executor_response_v0,
    parse_executor_start_v0, resolved_policy_digest_v0, validate_enforcement_receipt_v0,
};
use serde::Serialize;
use std::{collections::BTreeMap, time::Duration};

#[test]
fn tool_cleanup_uses_the_canonical_ordered_deadlines() {
    assert_eq!(TOOL_TERMINATION_GRACE_V0, Duration::from_secs(1));
    assert_eq!(TOOL_FORCED_REAP_DEADLINE_V0, Duration::from_secs(1));
    assert_eq!(TOOL_OUTPUT_DRAIN_DEADLINE_V0, Duration::from_secs(1));
}

fn canonical_wire(document: &impl Serialize) -> Vec<u8> {
    let mut wire = crate::canonical_json(&serde_json::to_value(document).unwrap())
        .unwrap()
        .into_bytes();
    wire.push(b'\n');
    wire
}

fn identity(device: u64, inode: u64, kind: ExecutorObjectKindV0) -> UnixObjectIdentityV0 {
    UnixObjectIdentityV0 {
        device,
        inode,
        kind,
    }
}

fn request() -> ExecutorRequestV0 {
    let limits = ExecutorLimitsV0 {
        max_concurrent_processes_and_threads: 32,
        max_stderr_bytes: 2,
        max_stdout_bytes: 3,
        timeout_ms: 1_000,
    };
    let mounts = vec![
        ExecutorMountV0 {
            access: ExecutorMountAccessV0::ReadWrite,
            descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0,
            origin: ExecutorMountOriginV0::Workspace,
            source_identity: identity(1, 2, ExecutorObjectKindV0::Directory),
            target: "/workspace/input".to_owned(),
        },
        ExecutorMountV0 {
            access: ExecutorMountAccessV0::ReadOnly,
            descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + 1,
            origin: ExecutorMountOriginV0::Runtime,
            source_identity: identity(3, 4, ExecutorObjectKindV0::File),
            target: "/bin/echo".to_owned(),
        },
    ];
    let resolved_mounts = vec![
        ExecutorResolvedMountV0 {
            access: mounts[0].access,
            descriptor: mounts[0].descriptor,
            origin: mounts[0].origin,
            source: "workspace/input".to_owned(),
            source_identity: mounts[0].source_identity.clone(),
            target: mounts[0].target.clone(),
        },
        ExecutorResolvedMountV0 {
            access: mounts[1].access,
            descriptor: mounts[1].descriptor,
            origin: mounts[1].origin,
            source: "/usr/bin/echo".to_owned(),
            source_identity: mounts[1].source_identity.clone(),
            target: mounts[1].target.clone(),
        },
    ];
    let resolved_policy = ExecutorResolvedPolicyV0 {
        artifact: serde_json::json!({"schema":"flow-tool-policy-v0"}),
        command: serde_json::json!({"executable":"/bin/echo"}),
        limits: limits.clone(),
        mounts: resolved_mounts,
        runtime_profile: RuntimeReadProfileV0::Exact,
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
    };
    ExecutorRequestV0 {
        argv: vec!["hello".to_owned()],
        environment: BTreeMap::from([("LANG".to_owned(), "C".to_owned())]),
        executable: "/bin/echo".to_owned(),
        limits,
        mounts,
        policy_digest: resolved_policy_digest_v0(&resolved_policy).unwrap(),
        resolved_policy,
        request_id: "request-1".to_owned(),
        runtime_profile: RuntimeReadProfileV0::Exact,
        schema: EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
        working_directory: "/workspace".to_owned(),
    }
}

fn refresh_policy_digest(request: &mut ExecutorRequestV0) {
    request.policy_digest = resolved_policy_digest_v0(&request.resolved_policy).unwrap();
}

fn probe() -> ExecutorProbeV0 {
    ExecutorProbeV0 {
        backend: "bubblewrap-seccomp".to_owned(),
        backend_version: "0.11.0".to_owned(),
        executor: "flow-executor".to_owned(),
        executor_version: "0.1.0".to_owned(),
        platform: "ubuntu-24.04-x86_64".to_owned(),
        protocol_versions: vec!["0".to_owned()],
        ready: true,
        runtime_mounts: vec![
            ExecutorRuntimeMountV0 {
                executable: Some("/bin/echo".to_owned()),
                runtime_profile: RuntimeReadProfileV0::Exact,
                source: "/usr/bin/echo".to_owned(),
                target: "/bin/echo".to_owned(),
            },
            ExecutorRuntimeMountV0 {
                executable: None,
                runtime_profile: RuntimeReadProfileV0::HostSystemRead,
                source: "/usr".to_owned(),
                target: "/usr".to_owned(),
            },
        ],
        schema: EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
        supported_policy_features: vec![crate::EXECUTOR_FEATURE_PROCESS_CAPACITY_V0.to_owned()],
    }
}

fn receipt() -> EnforcementReceiptV0 {
    EnforcementReceiptV0 {
        applied_policy_digest: "a".repeat(64),
        backend: "bubblewrap".to_owned(),
        backend_version: "0.11.0".to_owned(),
        executor: "flow-executor".to_owned(),
        executor_version: "0.1.0".to_owned(),
        isolation_active: true,
        max_concurrent_processes_and_threads: 32,
        platform: "linux-x86_64".to_owned(),
        runtime_profile: crate::RuntimeReadProfileV0::Exact,
    }
}

#[test]
fn executor_preflight_is_closed_bounded_and_bound_to_the_request() {
    for preflight in [
        ExecutorPreflightV0::Ready {
            request_id: "request-1".to_owned(),
            schema: EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
        },
        ExecutorPreflightV0::Error {
            code: ExecutorErrorCodeV0::PolicyUnsupported,
            message: "policy cannot be enforced".to_owned(),
            request_id: "request-1".to_owned(),
            schema: EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
        },
    ] {
        let wire = canonical_executor_preflight_v0(&preflight).expect("preflight is canonical");
        assert!(wire.len() <= MAX_EXECUTOR_CONTROL_BYTES_V0);
        assert_eq!(
            parse_executor_preflight_v0(&wire, "request-1").expect("preflight matches"),
            preflight
        );
        assert!(parse_executor_preflight_v0(&wire, "other-request").is_err());

        let mut open = serde_json::to_value(&preflight).unwrap();
        open.as_object_mut()
            .unwrap()
            .insert("extra".to_owned(), serde_json::Value::Null);
        assert!(parse_executor_preflight_v0(&canonical_wire(&open), "request-1").is_err());
    }
}

#[test]
fn executor_start_is_one_closed_bounded_record_bound_to_the_request() {
    let start = ExecutorStartV0 {
        request_id: "request-1".to_owned(),
        schema: EXECUTOR_START_SCHEMA_V0.to_owned(),
    };
    let wire = canonical_executor_start_v0(&start).expect("start is canonical");
    assert!(wire.len() <= MAX_EXECUTOR_CONTROL_BYTES_V0);
    assert_eq!(
        parse_executor_start_v0(&wire, "request-1").expect("start matches"),
        start
    );
    assert!(parse_executor_start_v0(&wire, "other-request").is_err());
    assert!(
        parse_executor_start_v0(
            br#"{"request_id":"request-1","schema":"flow-executor-start-v0","start":true}\n"#,
            "request-1"
        )
        .is_err()
    );
}

#[test]
fn executor_response_is_closed_and_bound_to_request_and_policy() {
    let response = ExecutorResponseV0::Completed {
        schema: EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
        request_id: "run-1-tool-1".to_owned(),
        tool_result: ExecutorToolResultV0 {
            classification: None,
            exit_code: Some(0),
            status: ExecutorToolStatusV0::Completed,
            stderr_base64: encode_executor_stream_v0(&[]),
            stdout_base64: encode_executor_stream_v0(&[0, 0xff, b'\n']),
        },
        enforcement: receipt(),
    };
    let text = canonical_wire(&response);

    let parsed = parse_executor_response_v0(&text, "run-1-tool-1", &"a".repeat(64))
        .expect("matching terminal evidence");
    assert!(matches!(parsed, ExecutorResponseV0::Completed { .. }));

    let mismatch = parse_executor_response_v0(&text, "run-1-tool-1", &"b".repeat(64)).unwrap_err();
    assert_eq!(
        mismatch.to_string(),
        "Executor applied the wrong policy digest"
    );

    let mismatch = parse_executor_response_v0(&text, "other-request", &"a".repeat(64)).unwrap_err();
    assert_eq!(
        mismatch.to_string(),
        "Executor response request id does not match"
    );

    let mut inactive = response.clone();
    let ExecutorResponseV0::Completed { enforcement, .. } = &mut inactive else {
        unreachable!("fixture is a completed response");
    };
    enforcement.isolation_active = false;
    let error =
        parse_executor_response_v0(&canonical_wire(&inactive), "run-1-tool-1", &"a".repeat(64))
            .unwrap_err();
    assert_eq!(error.to_string(), "Executor isolation was not active");

    let mut unknown = serde_json::to_value(&response).expect("response serializes");
    unknown
        .as_object_mut()
        .expect("response is an object")
        .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
    let error =
        parse_executor_response_v0(&canonical_wire(&unknown), "run-1-tool-1", &"a".repeat(64))
            .unwrap_err();
    assert!(error.to_string().contains("unknown field `unexpected`"));
}

#[test]
fn executor_response_preserves_bounded_non_utf8_tool_streams() {
    for (bytes, expected) in [
        (&[][..], ""),
        (&[0][..], "AA=="),
        (&[0, 0xff][..], "AP8="),
        (&[0, 0xff, b'\n'][..], "AP8K"),
        (&[0x6b, 0xef, 0xf3][..], "a+/z"),
    ] {
        let encoded = encode_executor_stream_v0(bytes);
        assert_eq!(encoded, expected);
        assert_eq!(decode_executor_stream_v0(&encoded).unwrap(), bytes);
    }

    for encoded in ["A", "AA=", "AA", "AA=A", "AB==", "AAF=", "AA-_", "AAAA="] {
        assert!(
            decode_executor_stream_v0(encoded).is_err(),
            "accepted non-canonical base64: {encoded}"
        );
    }
}

#[test]
fn executor_wire_framing_matrix_is_platform_neutral() {
    let response = serde_json::json!({
        "outcome": "error",
        "schema": EXECUTOR_RESPONSE_SCHEMA_V0,
        "request_id": "request-1",
        "code": ExecutorErrorCodeV0::SandboxSetupFailed,
        "message": "unsupported capability"
    });
    let canonical = crate::canonical_json(&response).unwrap();

    assert!(
        parse_executor_response_v0(
            format!("{canonical}\n").as_bytes(),
            "request-1",
            &"a".repeat(64),
        )
        .is_ok()
    );
    let invalid = [
        ("missing LF", canonical.as_bytes().to_vec()),
        ("malformed JSON", b"not-json\n".to_vec()),
        (
            "duplicate member",
            format!(
                "{}\n",
                canonical.replace("\"message\":", "\"message\":\"duplicate\",\"message\":")
            )
            .into_bytes(),
        ),
        (
            "multiple documents",
            format!("{canonical}\n{canonical}\n").into_bytes(),
        ),
        (
            "oversized document",
            vec![b' '; MAX_EXECUTOR_RESPONSE_BYTES_V0 + 1],
        ),
    ];
    for (name, wire) in invalid {
        assert!(
            parse_executor_response_v0(&wire, "request-1", &"a".repeat(64)).is_err(),
            "accepted {name}"
        );
    }
}

#[test]
fn executor_protocol_schema_names_are_private_v0_contracts() {
    assert_eq!(EXECUTOR_REQUEST_SCHEMA_V0, "flow-executor-request-v0");
    assert_eq!(EXECUTOR_PREFLIGHT_SCHEMA_V0, "flow-executor-preflight-v0");
    assert_eq!(EXECUTOR_START_SCHEMA_V0, "flow-executor-start-v0");
    assert_eq!(EXECUTOR_RESPONSE_SCHEMA_V0, "flow-executor-result-v0");
}

#[test]
fn executor_request_distinguishes_workspace_and_runtime_capabilities() {
    let mut request = request();

    canonical_executor_request_v0(&request).expect("runtime capability origin is explicit");

    request.mounts[1].target = "/bin/../bin/echo".to_owned();
    request.resolved_policy.mounts[1].target = request.mounts[1].target.clone();
    request.policy_digest = resolved_policy_digest_v0(&request.resolved_policy).unwrap();
    assert!(canonical_executor_request_v0(&request).is_err());
    request.mounts[1].target = "/bin/echo".to_owned();
    request.resolved_policy.mounts[1].target = request.mounts[1].target.clone();

    request.limits.max_stdout_bytes = MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1;
    request.resolved_policy.limits = request.limits.clone();
    request.policy_digest = resolved_policy_digest_v0(&request.resolved_policy).unwrap();
    assert!(canonical_executor_request_v0(&request).is_err());
}

#[test]
fn executor_request_round_trips_the_bound_policy_and_explicit_capabilities() {
    let request = request();
    let bytes = canonical_executor_request_v0(&request).expect("valid request");
    assert_eq!(parse_executor_request_v0(&bytes).unwrap(), request);
}

#[test]
fn executor_request_preserves_literal_argument_strings() {
    let mut request = request();
    request.argv = vec![
        "-c".to_owned(),
        "printf 'first\\nsecond\\n'\n".to_owned(),
        String::new(),
        "x".repeat(4_097),
    ];

    let bytes = canonical_executor_request_v0(&request).expect("literal arguments are valid");
    assert_eq!(parse_executor_request_v0(&bytes).unwrap(), request);
}

#[test]
fn executor_request_enforces_complete_exec_vector_entry_boundary() {
    let mut exact = request();
    exact.argv = vec![String::new(); MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0 - 1];
    canonical_executor_request_v0(&exact).expect("2,048 complete exec-vector entries are valid");

    let mut over = request();
    over.argv = vec![String::new(); MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0];
    let error = canonical_executor_request_v0(&over)
        .expect_err("2,049 complete exec-vector entries must be rejected");
    assert_eq!(error.to_string(), "Executor argv entry bound is invalid");
}

#[test]
fn executor_request_enforces_complete_exec_vector_byte_boundary() {
    let mut exact = request();
    let entries = exact.argv.len() + 1;
    let pointer_bytes = (entries + 1 + exact.environment.len() + 1) * std::mem::size_of::<usize>();
    let environment_bytes = exact
        .environment
        .iter()
        .map(|(name, value)| name.len() + 1 + value.len() + 1)
        .sum::<usize>();
    let exact_argument_bytes = MAX_EXECUTOR_EXEC_VECTOR_BYTES_V0
        - pointer_bytes
        - (exact.executable.len() + 1)
        - environment_bytes
        - 1;
    exact.argv = vec!["x".repeat(exact_argument_bytes)];
    canonical_executor_request_v0(&exact).expect("131,072 encoded exec-vector bytes are valid");

    let mut over = exact;
    over.argv[0].push('x');
    let error = canonical_executor_request_v0(&over)
        .expect_err("131,073 encoded exec-vector bytes must be rejected");
    assert_eq!(error.to_string(), "Executor argv exceeds its byte limit");
}

#[test]
fn executor_request_rejects_unbound_policy_fields() {
    let mut cases = Vec::new();

    let mut candidate = request();
    candidate.tool_id = "other".to_owned();
    cases.push(("tool identity", candidate));

    let mut candidate = request();
    candidate.runtime_profile = RuntimeReadProfileV0::HostSystemRead;
    cases.push(("runtime profile", candidate));

    let mut candidate = request();
    candidate.limits.timeout_ms += 1;
    cases.push(("limits", candidate));

    let mut candidate = request();
    candidate.mounts[1].source_identity.inode += 1;
    cases.push(("mount identity", candidate));

    let mut candidate = request();
    candidate.policy_digest = "b".repeat(64);
    cases.push(("policy digest", candidate));

    for (name, candidate) in cases {
        assert!(
            parse_executor_request_v0(&canonical_wire(&candidate)).is_err(),
            "accepted request with unbound {name}"
        );
    }
}

#[test]
fn executor_request_rejects_invalid_limits_mounts_environment_and_ids() {
    type Mutation = fn(&mut ExecutorRequestV0);
    let cases: &[(&str, Mutation, &str)] = &[
        (
            "empty request id",
            |candidate| candidate.request_id.clear(),
            "Executor request_id is invalid",
        ),
        (
            "zero output limit",
            |candidate| {
                candidate.limits.max_stdout_bytes = 0;
                candidate.resolved_policy.limits = candidate.limits.clone();
                refresh_policy_digest(candidate);
            },
            "Executor limits must be nonzero",
        ),
        (
            "zero process capacity",
            |candidate| {
                candidate.limits.max_concurrent_processes_and_threads = 0;
                candidate.resolved_policy.limits = candidate.limits.clone();
                refresh_policy_digest(candidate);
            },
            "Executor limits must be nonzero",
        ),
        (
            "oversized output limit",
            |candidate| {
                candidate.limits.max_stderr_bytes = MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1;
                candidate.resolved_policy.limits = candidate.limits.clone();
                refresh_policy_digest(candidate);
            },
            "Executor stream limits exceed the protocol bound",
        ),
        (
            "non-contiguous descriptor",
            |candidate| {
                candidate.mounts[1].descriptor += 1;
                candidate.resolved_policy.mounts[1].descriptor = candidate.mounts[1].descriptor;
                refresh_policy_digest(candidate);
            },
            "Executor mount descriptor is invalid",
        ),
        (
            "workspace target outside workspace",
            |candidate| {
                candidate.mounts[0].target = "/outside".to_owned();
                candidate.resolved_policy.mounts[0].target = candidate.mounts[0].target.clone();
                refresh_policy_digest(candidate);
            },
            "Executor workspace mount target is outside /workspace",
        ),
        (
            "writable runtime mount",
            |candidate| {
                candidate.mounts[1].access = ExecutorMountAccessV0::ReadWrite;
                candidate.resolved_policy.mounts[1].access = candidate.mounts[1].access;
                refresh_policy_digest(candidate);
            },
            "Executor runtime mount capability is invalid",
        ),
        (
            "control character in environment",
            |candidate| {
                candidate
                    .environment
                    .insert("BAD\nNAME".to_owned(), "value".to_owned());
            },
            "Executor environment name is invalid",
        ),
        (
            "too many argv entries",
            |candidate| candidate.argv = vec![String::new(); MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0],
            "Executor argv entry bound is invalid",
        ),
        (
            "NUL in argv",
            |candidate| candidate.argv = vec!["invalid\0argument".to_owned()],
            "Executor argv is invalid",
        ),
        (
            "oversized exec vector",
            |candidate| candidate.argv = vec!["x".repeat(4_096); 33],
            "Executor argv exceeds its byte limit",
        ),
        (
            "too many environment entries",
            |candidate| {
                candidate.environment = (0..257)
                    .map(|index| (format!("KEY_{index}"), "value".to_owned()))
                    .collect();
            },
            "Executor environment has too many entries",
        ),
        (
            "too many mounts",
            |candidate| {
                let template = candidate.mounts[0].clone();
                candidate.mounts = (0..=MAX_EXECUTOR_MOUNTS_V0)
                    .map(|index| ExecutorMountV0 {
                        descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + index as u32,
                        target: format!("/workspace/mount-{index}"),
                        ..template.clone()
                    })
                    .collect();
            },
            "Executor mount list exceeds its limit",
        ),
        (
            "duplicate mount target",
            |candidate| {
                candidate.mounts.push(ExecutorMountV0 {
                    descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + candidate.mounts.len() as u32,
                    ..candidate.mounts[0].clone()
                });
            },
            "Executor mount target is invalid",
        ),
        (
            "too many workspace mounts",
            |candidate| {
                let template = candidate.mounts[0].clone();
                candidate.mounts = (0..=MAX_EXECUTOR_WORKSPACE_MOUNTS_V0)
                    .map(|index| ExecutorMountV0 {
                        descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + index as u32,
                        target: format!("/workspace/mount-{index}"),
                        ..template.clone()
                    })
                    .collect();
            },
            "Executor mount provenance bounds are invalid",
        ),
        (
            "traversing workspace source",
            |candidate| {
                candidate.resolved_policy.mounts[0].source = "workspace/../outside".to_owned();
                refresh_policy_digest(candidate);
            },
            "Executor resolved mount does not match the inherited capability",
        ),
        (
            "absolute workspace source",
            |candidate| {
                candidate.resolved_policy.mounts[0].source = "/workspace/input".to_owned();
                refresh_policy_digest(candidate);
            },
            "Executor resolved mount does not match the inherited capability",
        ),
        (
            "non-object resolved policy artifact",
            |candidate| {
                candidate.resolved_policy.artifact = serde_json::json!([]);
                refresh_policy_digest(candidate);
            },
            "Executor resolved policy artifacts must be objects",
        ),
        (
            "unsupported request schema",
            |candidate| candidate.schema = "flow-executor-request-v1".to_owned(),
            "unsupported Executor request schema",
        ),
        (
            "non-canonical policy digest",
            |candidate| candidate.policy_digest = "A".repeat(64),
            "Executor policy_digest is not lowercase SHA-256",
        ),
    ];

    for &(name, mutate, expected) in cases {
        let mut candidate = request();
        mutate(&mut candidate);
        let error = parse_executor_request_v0(&canonical_wire(&candidate))
            .expect_err(&format!("accepted request with {name}"));
        assert_eq!(error.to_string(), expected, "{name}");
    }
}

#[test]
fn executor_request_byte_limit_is_enforced_on_encode_and_parse() {
    let mut oversized = request();
    oversized.resolved_policy.artifact = serde_json::json!({
        "padding": "x".repeat(MAX_EXECUTOR_REQUEST_BYTES_V0)
    });
    oversized.policy_digest = resolved_policy_digest_v0(&oversized.resolved_policy).unwrap();

    let error = canonical_executor_request_v0(&oversized).unwrap_err();
    assert_eq!(error.to_string(), "Executor request exceeds its byte limit");

    let error =
        parse_executor_request_v0(&vec![b' '; MAX_EXECUTOR_REQUEST_BYTES_V0 + 1]).unwrap_err();
    assert_eq!(error.to_string(), "Executor request exceeds its byte limit");
}

#[test]
fn executor_terminal_evidence_accepts_only_coherent_states() {
    use ExecutorToolClassificationV0 as Classification;
    use ExecutorToolStatusV0 as Status;

    let valid = [
        (Status::Completed, None, Some(0)),
        (Status::Failed, Some(Classification::NonzeroExit), Some(1)),
        (
            Status::Failed,
            Some(Classification::SignalTermination),
            None,
        ),
        (
            Status::Failed,
            Some(Classification::ProcessCapacityExceeded),
            None,
        ),
        (
            Status::Failed,
            Some(Classification::ProcessCapacityExceeded),
            Some(0),
        ),
        (
            Status::Failed,
            Some(Classification::ProcessCapacityExceeded),
            Some(1),
        ),
        (
            Status::Failed,
            Some(Classification::StdoutCapExceeded),
            None,
        ),
        (
            Status::Failed,
            Some(Classification::StderrCapExceeded),
            Some(1),
        ),
        (
            Status::Failed,
            Some(Classification::StdoutStderrCapExceeded),
            None,
        ),
        (
            Status::Failed,
            Some(Classification::OutputCollectorFailed),
            None,
        ),
        (
            Status::Failed,
            Some(Classification::OutputDrainTimeout),
            None,
        ),
        (Status::TimedOut, Some(Classification::ToolTimedOut), None),
        (Status::Cancelled, Some(Classification::Cancelled), None),
    ];
    for (status, classification, exit_code) in valid {
        let response = ExecutorResponseV0::Completed {
            schema: EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
            request_id: "request-1".to_owned(),
            tool_result: ExecutorToolResultV0 {
                classification,
                exit_code,
                status,
                stderr_base64: String::new(),
                stdout_base64: String::new(),
            },
            enforcement: receipt(),
        };
        canonical_executor_response_v0(&response).expect("coherent terminal evidence");
    }

    for (status, classification, exit_code) in [
        (Status::Completed, None, Some(1)),
        (Status::Failed, None, Some(1)),
        (Status::Failed, Some(Classification::NonzeroExit), Some(0)),
        (
            Status::TimedOut,
            Some(Classification::ToolTimedOut),
            Some(1),
        ),
        (Status::Cancelled, Some(Classification::Cancelled), Some(1)),
    ] {
        let response = ExecutorResponseV0::Completed {
            schema: EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
            request_id: "request-1".to_owned(),
            tool_result: ExecutorToolResultV0 {
                classification,
                exit_code,
                status,
                stderr_base64: String::new(),
                stdout_base64: String::new(),
            },
            enforcement: receipt(),
        };
        assert!(canonical_executor_response_v0(&response).is_err());
    }
}

#[test]
fn executor_contract_names_process_capacity_explicitly() {
    assert_eq!(
        crate::EXECUTOR_FEATURE_PROCESS_CAPACITY_V0,
        "process-capacity"
    );
    let mut request_value = serde_json::to_value(request()).expect("request serializes");
    request_value["limits"]["max_concurrent_processes_and_threads"] = serde_json::json!(32);
    request_value["resolved_policy"]["limits"]["max_concurrent_processes_and_threads"] =
        serde_json::json!(32);
    assert!(serde_json::from_value::<ExecutorRequestV0>(request_value).is_ok());

    let mut missing_request = serde_json::to_value(request()).expect("request serializes");
    missing_request["limits"]
        .as_object_mut()
        .expect("limits is an object")
        .remove("max_concurrent_processes_and_threads");
    assert!(serde_json::from_value::<ExecutorRequestV0>(missing_request).is_err());

    let mut receipt_value = serde_json::to_value(receipt()).expect("receipt serializes");
    receipt_value["max_concurrent_processes_and_threads"] = serde_json::json!(32);
    assert!(serde_json::from_value::<EnforcementReceiptV0>(receipt_value).is_ok());

    let mut missing_receipt = serde_json::to_value(receipt()).expect("receipt serializes");
    missing_receipt
        .as_object_mut()
        .expect("receipt is an object")
        .remove("max_concurrent_processes_and_threads");
    assert!(serde_json::from_value::<EnforcementReceiptV0>(missing_receipt).is_err());

    let classification: ExecutorToolClassificationV0 =
        serde_json::from_str("\"process_capacity_exceeded\"")
            .expect("process capacity has a stable classification");
    assert_eq!(
        serde_json::to_string(&classification).expect("classification serializes"),
        "\"process_capacity_exceeded\""
    );
}

#[test]
fn executor_receipt_proves_requested_policy_profile_and_process_capacity() {
    let valid = receipt();
    validate_enforcement_receipt_v0(&valid, &"a".repeat(64), RuntimeReadProfileV0::Exact, 32)
        .expect("matching active receipt");

    let mut cases = Vec::new();
    let mut candidate = valid.clone();
    candidate.isolation_active = false;
    cases.push(("inactive isolation", candidate));
    let mut candidate = valid.clone();
    candidate.applied_policy_digest = "b".repeat(64);
    cases.push(("wrong policy", candidate));
    let mut candidate = valid.clone();
    candidate.runtime_profile = RuntimeReadProfileV0::HostSystemRead;
    cases.push(("wrong runtime profile", candidate));
    let mut candidate = valid;
    candidate.max_concurrent_processes_and_threads = 31;
    cases.push(("wrong process capacity", candidate));

    for (name, candidate) in cases {
        assert!(
            validate_enforcement_receipt_v0(
                &candidate,
                &"a".repeat(64),
                RuntimeReadProfileV0::Exact,
                32,
            )
            .is_err(),
            "accepted receipt with {name}"
        );
    }
}

#[test]
fn executor_setup_error_is_terminal_without_a_policy_receipt() {
    let response = ExecutorResponseV0::Error {
        code: ExecutorErrorCodeV0::SandboxSetupFailed,
        message: "sandbox setup failed".to_owned(),
        request_id: "request-1".to_owned(),
        schema: EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
    };
    let bytes = canonical_executor_response_v0(&response).expect("bounded setup error");
    assert_eq!(
        parse_executor_response_v0(&bytes, "request-1", &"b".repeat(64)).unwrap(),
        response
    );

    let mut invalid = response.clone();
    let ExecutorResponseV0::Error { code, .. } = &mut invalid else {
        unreachable!();
    };
    *code = ExecutorErrorCodeV0::PolicyUnsupported;
    assert!(canonical_executor_response_v0(&invalid).is_err());
}

#[test]
fn executor_paths_are_canonical_object_identities() {
    let mut probe = probe();
    canonical_executor_probe_v0(&probe).expect("canonical runtime identity");

    probe.runtime_mounts[0].source = "/usr/bin/../bin/echo".to_owned();
    assert!(canonical_executor_probe_v0(&probe).is_err());
}

#[test]
fn executor_probe_round_trips_exact_and_host_system_runtime_manifests() {
    let probe = probe();
    let bytes = canonical_executor_probe_v0(&probe).expect("valid runtime manifest");
    assert_eq!(parse_executor_probe_v0(&bytes).unwrap(), probe);
}

#[test]
fn executor_probe_requires_at_least_one_protocol_version() {
    let mut probe = probe();
    probe.protocol_versions.clear();

    let error = parse_executor_probe_v0(&canonical_wire(&probe)).unwrap_err();
    assert_eq!(error.to_string(), "Executor probe list bounds are invalid");
}

#[test]
fn executor_probe_rejects_ambiguous_runtime_manifest_entries() {
    let mut cases = Vec::new();

    let mut candidate = probe();
    candidate.runtime_mounts[0].executable = None;
    cases.push(("exact mount without executable", candidate));

    let mut candidate = probe();
    candidate.runtime_mounts[0].executable = Some("/bin/python".to_owned());
    cases.push(("unsupported exact executable", candidate));

    let mut candidate = probe();
    candidate
        .runtime_mounts
        .push(candidate.runtime_mounts[0].clone());
    cases.push(("duplicate exact source and target", candidate));

    let mut candidate = probe();
    let mut duplicate_target = candidate.runtime_mounts[0].clone();
    duplicate_target.source = "/opt/echo".to_owned();
    candidate.runtime_mounts.push(duplicate_target);
    cases.push(("duplicate exact target", candidate));

    let mut candidate = probe();
    candidate.runtime_mounts[1].target = "/usr/../usr".to_owned();
    cases.push(("non-canonical host-system target", candidate));

    for (name, candidate) in cases {
        assert!(
            parse_executor_probe_v0(&canonical_wire(&candidate)).is_err(),
            "accepted manifest with {name}"
        );
    }
}
