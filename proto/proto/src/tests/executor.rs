use crate::{
    EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0, EXECUTOR_PROBE_SCHEMA_V0, EXECUTOR_REQUEST_SCHEMA_V0,
    EXECUTOR_RESPONSE_SCHEMA_V0, EnforcementReceiptV0, ExecutorErrorCodeV0, ExecutorLimitsV0,
    ExecutorMountAccessV0, ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0,
    ExecutorProbeV0, ExecutorRequestV0, ExecutorResolvedMountV0, ExecutorResolvedPolicyV0,
    ExecutorResponseV0, ExecutorRuntimeMountV0, ExecutorToolClassificationV0, ExecutorToolResultV0,
    ExecutorToolStatusV0, MAX_EXECUTOR_TOOL_STREAM_BYTES_V0, RuntimeReadProfileV0,
    UnixObjectIdentityV0, canonical_executor_probe_v0, canonical_executor_request_v0,
    canonical_executor_response_v0, decode_executor_stream_v0, encode_executor_stream_v0,
    parse_executor_probe_v0, parse_executor_request_v0, parse_executor_response_v0,
    resolved_policy_digest_v0, validate_enforcement_receipt_v0,
};
use serde::Serialize;
use std::collections::BTreeMap;

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
            target: "/workspace".to_owned(),
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
            source: "workspace".to_owned(),
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
        supported_policy_features: vec!["deny-all-network".to_owned()],
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
        platform: "linux-x86_64".to_owned(),
        runtime_profile: crate::RuntimeReadProfileV0::Exact,
    }
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
    let mut text = crate::canonical_json(&serde_json::to_value(response).unwrap()).unwrap();
    text.push('\n');

    let parsed = parse_executor_response_v0(text.as_bytes(), "run-1-tool-1", &"a".repeat(64))
        .expect("matching terminal evidence");
    assert!(matches!(parsed, ExecutorResponseV0::Completed { .. }));

    let mismatch =
        parse_executor_response_v0(text.as_bytes(), "run-1-tool-1", &"b".repeat(64)).unwrap_err();
    assert_eq!(
        mismatch.to_string(),
        "Executor applied the wrong policy digest"
    );

    let unknown = text.replacen("{", "{\"unexpected\":true,", 1);
    assert!(
        parse_executor_response_v0(unknown.as_bytes(), "run-1-tool-1", &"a".repeat(64),).is_err()
    );
}

#[test]
fn executor_response_preserves_bounded_non_utf8_tool_streams() {
    for (bytes, expected) in [
        (&[][..], ""),
        (&[0][..], "AA=="),
        (&[0, 0xff][..], "AP8="),
        (&[0, 0xff, b'\n'][..], "AP8K"),
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
fn executor_response_requires_one_canonical_lf_terminated_document() {
    let response = serde_json::json!({
        "outcome": "error",
        "schema": EXECUTOR_RESPONSE_SCHEMA_V0,
        "request_id": "request-1",
        "code": ExecutorErrorCodeV0::PolicyUnsupported,
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
    assert!(
        parse_executor_response_v0(canonical.as_bytes(), "request-1", &"a".repeat(64),).is_err()
    );
    assert!(
        parse_executor_response_v0(
            format!("{canonical}\n{canonical}\n").as_bytes(),
            "request-1",
            &"a".repeat(64),
        )
        .is_err()
    );
}

#[test]
fn executor_protocol_schema_names_are_private_v0_contracts() {
    assert_eq!(EXECUTOR_REQUEST_SCHEMA_V0, "flow-executor-request-v0");
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
    let mut cases = Vec::new();

    let mut candidate = request();
    candidate.request_id.clear();
    cases.push(("empty request id", candidate));

    let mut candidate = request();
    candidate.limits.max_stdout_bytes = 0;
    candidate.resolved_policy.limits = candidate.limits.clone();
    candidate.policy_digest = resolved_policy_digest_v0(&candidate.resolved_policy).unwrap();
    cases.push(("zero output limit", candidate));

    let mut candidate = request();
    candidate.limits.max_stderr_bytes = MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1;
    candidate.resolved_policy.limits = candidate.limits.clone();
    candidate.policy_digest = resolved_policy_digest_v0(&candidate.resolved_policy).unwrap();
    cases.push(("oversized output limit", candidate));

    let mut candidate = request();
    candidate.mounts[1].descriptor += 1;
    candidate.resolved_policy.mounts[1].descriptor = candidate.mounts[1].descriptor;
    candidate.policy_digest = resolved_policy_digest_v0(&candidate.resolved_policy).unwrap();
    cases.push(("non-contiguous descriptor", candidate));

    let mut candidate = request();
    candidate.mounts[0].target = "/outside".to_owned();
    candidate.resolved_policy.mounts[0].target = candidate.mounts[0].target.clone();
    candidate.policy_digest = resolved_policy_digest_v0(&candidate.resolved_policy).unwrap();
    cases.push(("workspace target outside workspace", candidate));

    let mut candidate = request();
    candidate.mounts[1].access = ExecutorMountAccessV0::ReadWrite;
    candidate.resolved_policy.mounts[1].access = candidate.mounts[1].access;
    candidate.policy_digest = resolved_policy_digest_v0(&candidate.resolved_policy).unwrap();
    cases.push(("writable runtime mount", candidate));

    let mut candidate = request();
    candidate
        .environment
        .insert("BAD\nNAME".to_owned(), "value".to_owned());
    cases.push(("control character in environment", candidate));

    for (name, candidate) in cases {
        assert!(
            parse_executor_request_v0(&canonical_wire(&candidate)).is_err(),
            "accepted request with {name}"
        );
    }
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
fn executor_receipt_proves_the_requested_policy_and_profile() {
    let valid = receipt();
    validate_enforcement_receipt_v0(&valid, &"a".repeat(64), RuntimeReadProfileV0::Exact)
        .expect("matching active receipt");

    let mut cases = Vec::new();
    let mut candidate = valid.clone();
    candidate.isolation_active = false;
    cases.push(("inactive isolation", candidate));
    let mut candidate = valid.clone();
    candidate.applied_policy_digest = "b".repeat(64);
    cases.push(("wrong policy", candidate));
    let mut candidate = valid;
    candidate.runtime_profile = RuntimeReadProfileV0::HostSystemRead;
    cases.push(("wrong runtime profile", candidate));

    for (name, candidate) in cases {
        assert!(
            validate_enforcement_receipt_v0(
                &candidate,
                &"a".repeat(64),
                RuntimeReadProfileV0::Exact,
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
