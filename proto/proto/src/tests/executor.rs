use crate::{
    EXECUTOR_FEATURE_SELF_PROTECTION_V0, EXECUTOR_PREFLIGHT_SCHEMA_V0, EXECUTOR_PROBE_SCHEMA_V0,
    EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0, EXECUTOR_REQUEST_SCHEMA_V0, EXECUTOR_RESPONSE_SCHEMA_V0,
    EXECUTOR_START_SCHEMA_V0, EnforcementReceiptV0, ExecutorErrorCodeV0, ExecutorLimitsV0,
    ExecutorObjectKindV0, ExecutorPreflightV0, ExecutorProbeV0, ExecutorProtectedObjectV0,
    ExecutorRequestV0, ExecutorResolvedPolicyV0, ExecutorResponseV0, ExecutorStartV0,
    ExecutorToolClassificationV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    MAX_EXECUTOR_CONTROL_BYTES_V0, MAX_EXECUTOR_EXEC_VECTOR_BYTES_V0,
    MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0, MAX_EXECUTOR_PROBE_BYTES_V0,
    MAX_EXECUTOR_PROTECTED_OBJECTS_V0, MAX_EXECUTOR_REQUEST_BYTES_V0,
    MAX_EXECUTOR_RESPONSE_BYTES_V0, MAX_EXECUTOR_TOOL_STREAM_BYTES_V0,
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

#[test]
fn executor_request_binds_host_invocation_and_own_file_protection_once() {
    use sha2::{Digest, Sha256};

    let policy = serde_json::json!({
        "argv": ["run", "build"],
        "environment": {"PATH": "/usr/bin:/bin"},
        "executable": "/opt/node/bin/npm",
        "limits": {"max_stderr_bytes": 1024, "max_stdout_bytes": 1024, "timeout_ms": 1000},
        "protected_objects": [
            {"descriptor": 32, "path": "/home/engineer/.flow-agent",
             "identity": {"device": 1, "inode": 2, "kind": "directory"}},
            {"descriptor": 33, "path": "/opt/flow/bin/flow",
             "identity": {"device": 1, "inode": 3, "kind": "file"}}
        ],
        "tool_id": "build",
        "tool_kind": "own-script",
        "working_directory": "/home/engineer/project"
    });
    let digest = Sha256::digest(canonical_wire(&policy))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let document = serde_json::json!({
        "policy_digest": digest,
        "request_id": "build-1",
        "resolved_policy": policy,
        "schema": EXECUTOR_REQUEST_SCHEMA_V0
    });
    let bytes = canonical_wire(&document);
    let request = parse_executor_request_v0(&bytes)
        .expect("host invocation with mandatory own-file protection is supported");
    assert_eq!(canonical_executor_request_v0(&request).unwrap(), bytes);

    for field in [
        "argv",
        "environment",
        "executable",
        "limits",
        "protected_objects",
        "working_directory",
    ] {
        let mut changed = document.clone();
        changed["resolved_policy"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            parse_executor_request_v0(&canonical_wire(&changed)).is_err(),
            "missing {field} must not silently change the invocation"
        );
    }
    let mut changed = document.clone();
    changed["resolved_policy"]["argv"] = serde_json::json!(["run", "publish"]);
    assert!(
        parse_executor_request_v0(&canonical_wire(&changed)).is_err(),
        "the receipt digest must bind the actual requested arguments"
    );
}

#[test]
fn executor_receipt_wire_attests_only_bound_self_protection() {
    let document = serde_json::json!({
        "applied_policy_digest": "a".repeat(64),
        "backend": "native-protection",
        "backend_version": "1",
        "executor": "flow-executor",
        "executor_version": "0.1.0",
        "platform": "test-platform",
        "self_protection_active": true
    });
    let receipt: EnforcementReceiptV0 = serde_json::from_value(document.clone())
        .expect("receipt requires only the bound self-protection evidence");
    assert_eq!(serde_json::to_value(receipt).unwrap(), document);
    for (field, value) in [
        ("isolation_active", serde_json::json!(true)),
        ("runtime_profile", serde_json::json!("exact")),
        (
            "max_concurrent_processes_and_threads",
            serde_json::json!(32),
        ),
    ] {
        let mut legacy = document.clone();
        legacy[field] = value;
        assert!(
            serde_json::from_value::<EnforcementReceiptV0>(legacy).is_err(),
            "accepted {field}"
        );
    }
    let mut missing = document;
    missing
        .as_object_mut()
        .unwrap()
        .remove("self_protection_active");
    assert!(serde_json::from_value::<EnforcementReceiptV0>(missing).is_err());
}

#[test]
fn executor_probe_wire_has_no_runtime_manifest() {
    let document = serde_json::json!({
        "backend": "native-protection",
        "backend_version": "1",
        "executor": "flow-executor",
        "executor_version": "0.1.0",
        "platform": "test-platform",
        "protocol_versions": ["0"],
        "ready": true,
        "schema": EXECUTOR_PROBE_SCHEMA_V0,
        "supported_policy_features": ["flow-owned-write-protection"]
    });
    let bytes = canonical_wire(&document);
    let parsed = parse_executor_probe_v0(&bytes).expect("readiness needs no runtime manifest");
    assert_eq!(canonical_executor_probe_v0(&parsed).unwrap(), bytes);
    let mut legacy = document;
    legacy["runtime_mounts"] = serde_json::json!([]);
    assert!(parse_executor_probe_v0(&canonical_wire(&legacy)).is_err());
}

#[test]
fn executor_rejects_process_capacity_classification() {
    assert!(
        serde_json::from_str::<ExecutorToolClassificationV0>("\"process_capacity_exceeded\"")
            .is_err()
    );
}

fn identity(device: u64, inode: u64, kind: ExecutorObjectKindV0) -> UnixObjectIdentityV0 {
    UnixObjectIdentityV0 {
        device,
        inode,
        kind,
    }
}

fn request() -> ExecutorRequestV0 {
    let resolved_policy = ExecutorResolvedPolicyV0 {
        argv: vec!["hello".to_owned()],
        environment: BTreeMap::from([("LANG".to_owned(), "C".to_owned())]),
        executable: "/bin/echo".to_owned(),
        limits: ExecutorLimitsV0 {
            max_stderr_bytes: 2,
            max_stdout_bytes: 3,
            timeout_ms: 1_000,
        },
        protected_objects: vec![
            ExecutorProtectedObjectV0 {
                descriptor: EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0,
                identity: identity(1, 2, ExecutorObjectKindV0::Directory),
                path: "/home/engineer/.flow-agent".to_owned(),
            },
            ExecutorProtectedObjectV0 {
                descriptor: EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + 1,
                identity: identity(3, 4, ExecutorObjectKindV0::File),
                path: "/opt/flow/bin/flow".to_owned(),
            },
        ],
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
        working_directory: "/workspace".to_owned(),
    };
    ExecutorRequestV0 {
        policy_digest: resolved_policy_digest_v0(&resolved_policy).unwrap(),
        resolved_policy,
        request_id: "request-1".to_owned(),
        schema: EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
    }
}

fn refresh_policy_digest(request: &mut ExecutorRequestV0) {
    request.policy_digest = resolved_policy_digest_v0(&request.resolved_policy).unwrap();
}

fn probe() -> ExecutorProbeV0 {
    ExecutorProbeV0 {
        backend: "native-protection".to_owned(),
        backend_version: "1".to_owned(),
        executor: "flow-executor".to_owned(),
        executor_version: "0.1.0".to_owned(),
        platform: "test-platform".to_owned(),
        protocol_versions: vec!["0".to_owned()],
        ready: true,
        schema: EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
        supported_policy_features: vec![EXECUTOR_FEATURE_SELF_PROTECTION_V0.to_owned()],
    }
}

fn receipt() -> EnforcementReceiptV0 {
    EnforcementReceiptV0 {
        applied_policy_digest: "a".repeat(64),
        backend: "native-protection".to_owned(),
        backend_version: "1".to_owned(),
        executor: "flow-executor".to_owned(),
        executor_version: "0.1.0".to_owned(),
        self_protection_active: true,
        platform: "test-platform".to_owned(),
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
    enforcement.self_protection_active = false;
    let error =
        parse_executor_response_v0(&canonical_wire(&inactive), "run-1-tool-1", &"a".repeat(64))
            .unwrap_err();
    assert_eq!(error.to_string(), "Executor self-protection was not active");

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
fn executor_request_accepts_the_complete_protected_descriptor_capacity() {
    let mut request = request();
    let template = request.resolved_policy.protected_objects[0].clone();
    request.resolved_policy.protected_objects = (0..MAX_EXECUTOR_PROTECTED_OBJECTS_V0)
        .map(|index| ExecutorProtectedObjectV0 {
            descriptor: EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + index as u32,
            path: format!("/protected/object-{index}"),
            ..template.clone()
        })
        .collect();
    refresh_policy_digest(&mut request);
    let bytes =
        canonical_executor_request_v0(&request).expect("all 128 descriptor slots are valid");
    assert_eq!(parse_executor_request_v0(&bytes).unwrap(), request);

    request.resolved_policy.protected_objects[0].path = "/workspace".to_owned();
    request.resolved_policy.protected_objects[1].path = "/workspace/nested/file".to_owned();
    refresh_policy_digest(&mut request);
    canonical_executor_request_v0(&request).expect("overlapping protected paths are valid");
}

#[test]
fn executor_request_round_trips_the_bound_invocation_and_protected_objects() {
    let request = request();
    let bytes = canonical_executor_request_v0(&request).expect("valid request");
    assert_eq!(parse_executor_request_v0(&bytes).unwrap(), request);
}

#[test]
fn executor_request_preserves_literal_argument_strings() {
    let mut request = request();
    request.resolved_policy.argv = vec![
        "-c".to_owned(),
        "printf 'first\\nsecond\\n'\n".to_owned(),
        String::new(),
        "x".repeat(4_097),
    ];

    refresh_policy_digest(&mut request);
    let bytes = canonical_executor_request_v0(&request).expect("literal arguments are valid");
    assert_eq!(parse_executor_request_v0(&bytes).unwrap(), request);
}

#[test]
fn executor_request_enforces_complete_exec_vector_entry_boundary() {
    let mut exact = request();
    exact.resolved_policy.argv = vec![String::new(); MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0 - 1];
    refresh_policy_digest(&mut exact);
    canonical_executor_request_v0(&exact).expect("2,048 complete exec-vector entries are valid");

    let mut over = request();
    over.resolved_policy.argv = vec![String::new(); MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0];
    let error = canonical_executor_request_v0(&over)
        .expect_err("2,049 complete exec-vector entries must be rejected");
    assert_eq!(error.to_string(), "Executor argv entry bound is invalid");
}

#[test]
fn executor_request_enforces_complete_exec_vector_byte_boundary() {
    let mut exact = request();
    let entries = exact.resolved_policy.argv.len() + 1;
    let pointer_bytes =
        (entries + 1 + exact.resolved_policy.environment.len() + 1) * std::mem::size_of::<usize>();
    let environment_bytes = exact
        .resolved_policy
        .environment
        .iter()
        .map(|(name, value)| name.len() + 1 + value.len() + 1)
        .sum::<usize>();
    let exact_argument_bytes = MAX_EXECUTOR_EXEC_VECTOR_BYTES_V0
        - pointer_bytes
        - (exact.resolved_policy.executable.len() + 1)
        - environment_bytes
        - 1;
    exact.resolved_policy.argv = vec!["x".repeat(exact_argument_bytes)];
    refresh_policy_digest(&mut exact);
    canonical_executor_request_v0(&exact).expect("131,072 encoded exec-vector bytes are valid");

    let mut over = exact;
    over.resolved_policy.argv[0].push('x');
    let error = canonical_executor_request_v0(&over)
        .expect_err("131,073 encoded exec-vector bytes must be rejected");
    assert_eq!(error.to_string(), "Executor argv exceeds its byte limit");
}

#[test]
fn executor_request_rejects_unbound_policy_fields() {
    type Mutation = fn(&mut ExecutorResolvedPolicyV0);
    let cases: &[(&str, Mutation)] = &[
        ("arguments", |policy| policy.argv.push("other".to_owned())),
        ("environment", |policy| {
            policy
                .environment
                .insert("LANG".to_owned(), "other".to_owned());
        }),
        ("executable", |policy| {
            policy.executable = "/opt/bin/other".to_owned()
        }),
        ("working directory", |policy| {
            policy.working_directory = "/other".to_owned()
        }),
        ("tool identity", |policy| {
            policy.tool_id = "other".to_owned()
        }),
        ("tool kind", |policy| {
            policy.tool_kind = "own-script".to_owned()
        }),
        ("deadline", |policy| policy.limits.timeout_ms += 1),
        ("stdout limit", |policy| policy.limits.max_stdout_bytes += 1),
        ("stderr limit", |policy| policy.limits.max_stderr_bytes += 1),
        ("protected path", |policy| {
            policy.protected_objects[0].path = "/other".to_owned()
        }),
        ("protected device", |policy| {
            policy.protected_objects[0].identity.device += 1
        }),
        ("protected inode", |policy| {
            policy.protected_objects[0].identity.inode += 1
        }),
        ("protected kind", |policy| {
            policy.protected_objects[0].identity.kind = ExecutorObjectKindV0::File
        }),
        ("protected inventory", |policy| {
            policy.protected_objects.pop();
        }),
    ];
    for &(name, mutate) in cases {
        let mut candidate = request();
        mutate(&mut candidate.resolved_policy);
        assert_ne!(
            resolved_policy_digest_v0(&candidate.resolved_policy).unwrap(),
            candidate.policy_digest,
            "{name}"
        );
        let error = parse_executor_request_v0(&canonical_wire(&candidate)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Executor request policy digest does not match",
            "{name}"
        );
        refresh_policy_digest(&mut candidate);
        canonical_executor_request_v0(&candidate)
            .expect("rebinding a valid policy permits the change");
    }

    let mut candidate = request();
    candidate.resolved_policy.protected_objects[0].descriptor += 1;
    assert_ne!(
        resolved_policy_digest_v0(&candidate.resolved_policy).unwrap(),
        candidate.policy_digest
    );
    assert!(parse_executor_request_v0(&canonical_wire(&candidate)).is_err());

    let mut candidate = request();
    candidate.policy_digest = "b".repeat(64);
    assert!(parse_executor_request_v0(&canonical_wire(&candidate)).is_err());
}

#[test]
fn executor_request_rejects_invalid_limits_protection_environment_and_ids() {
    type Mutation = fn(&mut ExecutorResolvedPolicyV0);
    let cases: &[(&str, Mutation, &str)] = &[
        (
            "empty tool id",
            |policy| policy.tool_id.clear(),
            "Executor tool_id is invalid",
        ),
        (
            "empty tool kind",
            |policy| policy.tool_kind.clear(),
            "Executor tool_kind is invalid",
        ),
        (
            "zero stdout limit",
            |policy| policy.limits.max_stdout_bytes = 0,
            "Executor limits must be nonzero",
        ),
        (
            "zero stderr limit",
            |policy| policy.limits.max_stderr_bytes = 0,
            "Executor limits must be nonzero",
        ),
        (
            "zero deadline",
            |policy| policy.limits.timeout_ms = 0,
            "Executor limits must be nonzero",
        ),
        (
            "oversized stdout limit",
            |policy| policy.limits.max_stdout_bytes = MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1,
            "Executor stream limits exceed the protocol bound",
        ),
        (
            "oversized stderr limit",
            |policy| policy.limits.max_stderr_bytes = MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1,
            "Executor stream limits exceed the protocol bound",
        ),
        (
            "non-contiguous descriptor",
            |policy| policy.protected_objects[1].descriptor += 1,
            "Executor protected object descriptor is invalid",
        ),
        (
            "wrong descriptor base",
            |policy| policy.protected_objects[0].descriptor -= 1,
            "Executor protected object descriptor is invalid",
        ),
        (
            "duplicate descriptor",
            |policy| {
                policy.protected_objects[1].descriptor = policy.protected_objects[0].descriptor
            },
            "Executor protected object descriptor is invalid",
        ),
        (
            "no protected objects",
            |policy| policy.protected_objects.clear(),
            "Executor protected object list must be nonempty",
        ),
        (
            "control character in environment",
            |policy| {
                policy
                    .environment
                    .insert("BAD\nNAME".to_owned(), "value".to_owned());
            },
            "Executor environment name is invalid",
        ),
        (
            "too many argv entries",
            |policy| policy.argv = vec![String::new(); MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0],
            "Executor argv entry bound is invalid",
        ),
        (
            "NUL in argv",
            |policy| policy.argv = vec!["invalid\0argument".to_owned()],
            "Executor argv is invalid",
        ),
        (
            "oversized exec vector",
            |policy| policy.argv = vec!["x".repeat(4_096); 33],
            "Executor argv exceeds its byte limit",
        ),
        (
            "too many environment entries",
            |policy| {
                policy.environment = (0..257)
                    .map(|index| (format!("KEY_{index}"), "value".to_owned()))
                    .collect();
            },
            "Executor environment has too many entries",
        ),
        (
            "too many protected objects",
            |policy| {
                let template = policy.protected_objects[0].clone();
                policy.protected_objects = (0..=MAX_EXECUTOR_PROTECTED_OBJECTS_V0)
                    .map(|index| ExecutorProtectedObjectV0 {
                        descriptor: EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + index as u32,
                        path: format!("/protected/object-{index}"),
                        ..template.clone()
                    })
                    .collect();
            },
            "Executor protected object list exceeds its limit",
        ),
        (
            "duplicate protected path",
            |policy| {
                policy.protected_objects[1].path = policy.protected_objects[0].path.clone();
            },
            "Executor protected object path is duplicated",
        ),
    ];

    for &(name, mutate, expected) in cases {
        let mut candidate = request();
        mutate(&mut candidate.resolved_policy);
        refresh_policy_digest(&mut candidate);
        let error = parse_executor_request_v0(&canonical_wire(&candidate))
            .expect_err(&format!("accepted request with {name}"));
        assert_eq!(error.to_string(), expected, "{name}");
    }
    let mut candidate = request();
    candidate.request_id.clear();
    assert_eq!(
        parse_executor_request_v0(&canonical_wire(&candidate))
            .unwrap_err()
            .to_string(),
        "Executor request_id is invalid"
    );
    let mut candidate = request();
    candidate.schema = "flow-executor-request-v1".to_owned();
    assert_eq!(
        parse_executor_request_v0(&canonical_wire(&candidate))
            .unwrap_err()
            .to_string(),
        "unsupported Executor request schema"
    );
    let mut candidate = request();
    candidate.policy_digest = "A".repeat(64);
    assert_eq!(
        parse_executor_request_v0(&canonical_wire(&candidate))
            .unwrap_err()
            .to_string(),
        "Executor policy_digest is not lowercase SHA-256"
    );
}

#[test]
fn executor_request_byte_limit_is_enforced_on_encode_and_parse() {
    let mut oversized = request();
    let template = oversized.resolved_policy.protected_objects[0].clone();
    oversized.resolved_policy.protected_objects = (0..MAX_EXECUTOR_PROTECTED_OBJECTS_V0)
        .map(|index| ExecutorProtectedObjectV0 {
            descriptor: EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 + index as u32,
            path: format!("/{}{index}", "界".repeat(3_000)),
            ..template.clone()
        })
        .collect();
    refresh_policy_digest(&mut oversized);

    let error = canonical_executor_request_v0(&oversized).unwrap_err();
    assert_eq!(error.to_string(), "Executor request exceeds its byte limit");
    let error = parse_executor_request_v0(&canonical_wire(&oversized)).unwrap_err();
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
fn executor_request_rejects_legacy_and_duplicate_security_fields() {
    let request = serde_json::to_value(request()).unwrap();
    for (pointer, fields) in [
        (
            "",
            &[
                "argv",
                "environment",
                "executable",
                "limits",
                "mounts",
                "runtime_profile",
                "tool_id",
                "tool_kind",
                "working_directory",
                "protected_objects",
            ][..],
        ),
        (
            "/resolved_policy",
            &["artifact", "command", "mounts", "runtime_profile"][..],
        ),
        (
            "/resolved_policy/limits",
            &["max_concurrent_processes_and_threads"][..],
        ),
        (
            "/resolved_policy/protected_objects/0",
            &["access", "origin", "source", "source_identity", "target"][..],
        ),
        (
            "/resolved_policy/protected_objects/0/identity",
            &["extra"][..],
        ),
    ] {
        for field in fields {
            let mut legacy = request.clone();
            legacy.pointer_mut(pointer).unwrap()[*field] = serde_json::Value::Null;
            let error = parse_executor_request_v0(&canonical_wire(&legacy)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("unknown field \x60{field}\x60")),
                "{pointer}/{field}: {error}"
            );
        }
    }
    let mut invalid_kind = request;
    invalid_kind["resolved_policy"]["protected_objects"][0]["identity"]["kind"] =
        serde_json::json!("symlink");
    let error = parse_executor_request_v0(&canonical_wire(&invalid_kind)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unknown variant \x60symlink\x60")
    );
}

#[test]
fn executor_receipt_proves_requested_policy_and_active_self_protection() {
    let valid = receipt();
    validate_enforcement_receipt_v0(&valid, &"a".repeat(64)).expect("matching active receipt");

    let mut cases = Vec::new();
    let mut candidate = valid.clone();
    candidate.self_protection_active = false;
    cases.push(("inactive self-protection", candidate));
    let mut candidate = valid.clone();
    candidate.applied_policy_digest = "b".repeat(64);
    cases.push(("wrong policy", candidate));
    let mut candidate = valid.clone();
    candidate.applied_policy_digest = "A".repeat(64);
    cases.push(("non-canonical digest", candidate));
    let mut candidate = valid;
    candidate.backend.clear();
    cases.push(("missing backend identity", candidate));

    for (name, candidate) in cases {
        assert!(
            validate_enforcement_receipt_v0(&candidate, &"a".repeat(64)).is_err(),
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
    for path in [
        "relative",
        "/",
        "/trailing/",
        "//double",
        "/double//slash",
        "/dot/./path",
        "/parent/../path",
        "/back\\slash",
        "/bad\npath",
        "",
    ] {
        for field in ["executable", "working_directory", "protected object"] {
            let mut candidate = request();
            match field {
                "executable" => candidate.resolved_policy.executable = path.to_owned(),
                "working_directory" => {
                    candidate.resolved_policy.working_directory = path.to_owned()
                }
                _ => candidate.resolved_policy.protected_objects[0].path = path.to_owned(),
            }
            refresh_policy_digest(&mut candidate);
            assert!(
                canonical_executor_request_v0(&candidate).is_err(),
                "accepted {field} path {path:?}"
            );
            assert!(
                parse_executor_request_v0(&canonical_wire(&candidate)).is_err(),
                "parsed {field} path {path:?}"
            );
        }
    }
}

#[test]
fn executor_probe_round_trips_readiness_and_features() {
    assert_eq!(
        EXECUTOR_FEATURE_SELF_PROTECTION_V0,
        "flow-owned-write-protection"
    );
    for ready in [false, true] {
        let mut probe = probe();
        probe.ready = ready;
        let bytes = canonical_executor_probe_v0(&probe).expect("valid readiness record");
        assert_eq!(parse_executor_probe_v0(&bytes).unwrap(), probe);
    }
}

#[test]
fn executor_probe_requires_at_least_one_protocol_version() {
    let mut probe = probe();
    probe.protocol_versions.clear();

    let error = parse_executor_probe_v0(&canonical_wire(&probe)).unwrap_err();
    assert_eq!(error.to_string(), "Executor probe list bounds are invalid");
}

#[test]
fn executor_probe_rejects_invalid_identity_and_feature_bounds() {
    type Mutation = fn(&mut ExecutorProbeV0);
    let cases: &[(&str, Mutation)] = &[
        ("empty backend", |probe| probe.backend.clear()),
        ("empty backend version", |probe| {
            probe.backend_version.clear()
        }),
        ("empty executor", |probe| probe.executor.clear()),
        ("empty executor version", |probe| {
            probe.executor_version.clear()
        }),
        ("empty platform", |probe| probe.platform.clear()),
        ("unsupported schema", |probe| {
            probe.schema = "flow-executor-probe-v1".to_owned()
        }),
        ("too many protocols", |probe| {
            probe.protocol_versions = vec!["0".to_owned(); 257]
        }),
        ("too many features", |probe| {
            probe.supported_policy_features = vec!["feature".to_owned(); 257]
        }),
        ("empty protocol", |probe| {
            probe.protocol_versions = vec![String::new()]
        }),
        ("invalid feature", |probe| {
            probe.supported_policy_features = vec!["bad\nfeature".to_owned()]
        }),
    ];
    for &(name, mutate) in cases {
        let mut candidate = probe();
        mutate(&mut candidate);
        assert!(
            parse_executor_probe_v0(&canonical_wire(&candidate)).is_err(),
            "accepted {name}"
        );
    }
    let oversized = vec![b' '; MAX_EXECUTOR_PROBE_BYTES_V0 + 1];
    assert_eq!(
        parse_executor_probe_v0(&oversized).unwrap_err().to_string(),
        "Executor probe exceeds its byte limit"
    );
}
