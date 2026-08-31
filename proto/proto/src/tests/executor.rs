use crate::{
    EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0, EXECUTOR_REQUEST_SCHEMA_V0, EXECUTOR_RESPONSE_SCHEMA_V0,
    EnforcementReceiptV0, ExecutorErrorCodeV0, ExecutorLimitsV0, ExecutorMountAccessV0,
    ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0, ExecutorProbeV0,
    ExecutorRequestV0, ExecutorResolvedMountV0, ExecutorResolvedPolicyV0, ExecutorResponseV0,
    ExecutorRuntimeMountV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    MAX_EXECUTOR_TOOL_STREAM_BYTES_V0, RuntimeReadProfileV0, UnixObjectIdentityV0,
    canonical_executor_probe_v0, canonical_executor_request_v0, decode_executor_stream_v0,
    encode_executor_stream_v0, parse_executor_response_v0, resolved_policy_digest_v0,
};
use std::collections::BTreeMap;

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
    let bytes = [0, 0xff, b'\n'];
    let encoded = encode_executor_stream_v0(&bytes);
    assert_eq!(decode_executor_stream_v0(&encoded).unwrap(), bytes);
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
    let limits = ExecutorLimitsV0 {
        max_stderr_bytes: 1,
        max_stdout_bytes: 1,
        timeout_ms: 1,
    };
    let identity = UnixObjectIdentityV0 {
        device: 1,
        inode: 2,
        kind: ExecutorObjectKindV0::File,
    };
    let resolved_policy = ExecutorResolvedPolicyV0 {
        artifact: serde_json::json!({"schema":"flow-tool-policy-v0"}),
        command: serde_json::json!({"executable":"/bin/echo"}),
        limits: limits.clone(),
        mounts: vec![ExecutorResolvedMountV0 {
            access: ExecutorMountAccessV0::ReadOnly,
            descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0,
            origin: ExecutorMountOriginV0::Runtime,
            source: "/usr/bin/echo".to_owned(),
            source_identity: identity.clone(),
            target: "/bin/echo".to_owned(),
        }],
        runtime_profile: RuntimeReadProfileV0::Exact,
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
    };
    let mut request = ExecutorRequestV0 {
        argv: Vec::new(),
        environment: BTreeMap::new(),
        executable: "/bin/echo".to_owned(),
        limits,
        mounts: vec![ExecutorMountV0 {
            access: ExecutorMountAccessV0::ReadOnly,
            descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0,
            origin: ExecutorMountOriginV0::Runtime,
            source_identity: identity,
            target: "/bin/echo".to_owned(),
        }],
        policy_digest: resolved_policy_digest_v0(&resolved_policy).unwrap(),
        resolved_policy,
        request_id: "request-1".to_owned(),
        runtime_profile: RuntimeReadProfileV0::Exact,
        schema: EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
        working_directory: "/workspace".to_owned(),
    };

    canonical_executor_request_v0(&request).expect("runtime capability origin is explicit");

    request.mounts[0].target = "/bin/../bin/echo".to_owned();
    request.resolved_policy.mounts[0].target = request.mounts[0].target.clone();
    request.policy_digest = resolved_policy_digest_v0(&request.resolved_policy).unwrap();
    assert!(canonical_executor_request_v0(&request).is_err());
    request.mounts[0].target = "/bin/echo".to_owned();
    request.resolved_policy.mounts[0].target = request.mounts[0].target.clone();

    request.limits.max_stdout_bytes = MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1;
    request.resolved_policy.limits = request.limits.clone();
    request.policy_digest = resolved_policy_digest_v0(&request.resolved_policy).unwrap();
    assert!(canonical_executor_request_v0(&request).is_err());
}

#[test]
fn executor_paths_are_canonical_object_identities() {
    let mut probe = ExecutorProbeV0 {
        backend: "bubblewrap-seccomp".to_owned(),
        backend_version: "0.11.0".to_owned(),
        executor: "flow-executor".to_owned(),
        executor_version: "0.1.0".to_owned(),
        platform: "ubuntu-24.04-x86_64".to_owned(),
        protocol_versions: vec!["0".to_owned()],
        ready: true,
        runtime_mounts: vec![ExecutorRuntimeMountV0 {
            executable: Some("/bin/echo".to_owned()),
            runtime_profile: RuntimeReadProfileV0::Exact,
            source: "/usr/bin/echo".to_owned(),
            target: "/bin/echo".to_owned(),
        }],
        schema: crate::EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
        supported_policy_features: Vec::new(),
    };
    canonical_executor_probe_v0(&probe).expect("canonical runtime identity");

    probe.runtime_mounts[0].source = "/usr/bin/../bin/echo".to_owned();
    assert!(canonical_executor_probe_v0(&probe).is_err());
}
