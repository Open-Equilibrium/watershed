//! Protocol v0 runtime event contracts.

#![deny(missing_docs)]

mod canonical;
mod error;
mod event;
mod executor;
mod flow_value;
mod metadata;
mod session_object;

pub use canonical::{canonical_json, parse_unique_json};
pub use error::{CanonicalJsonError, EventValidationError};
pub use event::{
    EventEnvelope, EventStateIdentifierKind, MAX_EVENT_PAYLOAD_STATE_IDENTIFIERS_V0,
    MAX_EVENT_STATE_IDENTIFIERS_V0, PhaseKind, ToolKind, ToolNetworkAccess, UnknownPhaseKind,
    UnknownToolKind, UnknownToolNetworkAccess,
};
pub use executor::{
    EXECUTOR_BACKEND_V0, EXECUTOR_EXACT_EXECUTABLES_V0, EXECUTOR_FEATURE_DENY_NETWORK_V0,
    EXECUTOR_FEATURE_DESCRIPTOR_MOUNTS_V0, EXECUTOR_FEATURE_MOUNT_IDENTITY_V0,
    EXECUTOR_FEATURE_PROCESS_CAPACITY_V0, EXECUTOR_FEATURE_PROCESS_CONTAINMENT_V0,
    EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0, EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0, EXECUTOR_NAME_V0,
    EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0, EXECUTOR_PLATFORM_V0, EXECUTOR_PROBE_SCHEMA_V0,
    EXECUTOR_PROTOCOL_VERSION_V0, EXECUTOR_REQUEST_SCHEMA_V0, EXECUTOR_RESPONSE_SCHEMA_V0,
    EnforcementReceiptV0, ExecutorErrorCodeV0, ExecutorExecVectorErrorV0, ExecutorLimitsV0,
    ExecutorMountAccessV0, ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0,
    ExecutorProbeV0, ExecutorProtocolError, ExecutorRequestV0, ExecutorResolvedMountV0,
    ExecutorResolvedPolicyV0, ExecutorResponseV0, ExecutorRuntimeMountV0,
    ExecutorToolClassificationV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    MAX_EXECUTOR_EXEC_VECTOR_BYTES_V0, MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0, MAX_EXECUTOR_MOUNTS_V0,
    MAX_EXECUTOR_PROBE_BYTES_V0, MAX_EXECUTOR_REQUEST_BYTES_V0, MAX_EXECUTOR_RESPONSE_BYTES_V0,
    MAX_EXECUTOR_RUNTIME_MOUNTS_V0, MAX_EXECUTOR_TOOL_STREAM_BYTES_V0,
    MAX_EXECUTOR_WORKSPACE_MOUNTS_V0, RuntimeReadProfileV0, TOOL_FORCED_REAP_DEADLINE_V0,
    TOOL_OUTPUT_DRAIN_DEADLINE_V0, TOOL_TERMINATION_GRACE_V0, UnixObjectIdentityV0,
    canonical_executor_probe_v0, canonical_executor_request_v0, canonical_executor_response_v0,
    decode_executor_stream_v0, encode_executor_stream_v0, parse_executor_probe_v0,
    parse_executor_request_v0, parse_executor_response_v0, resolved_policy_digest_v0,
    validate_enforcement_receipt_v0, validate_executor_exec_vector_v0,
};
pub use flow_value::{
    CanonicalIntegerError, FlowValueValidationError, parse_canonical_i64, validate_flow_value_v0,
};
pub use metadata::{
    EventMetadataError, EventType, MAX_SESSION_ID_BYTES, UnknownEventType,
    format_rfc3339_utc_timestamp, is_valid_session_id, parse_rfc3339_utc_timestamp,
};
pub use session_object::{
    SessionObjectUriError, build_session_object_uri, decode_lowercase_sha256_hex,
    parse_session_object_uri,
};

/// Protocol version string emitted by all v0 event envelopes.
pub const PROTOCOL_VERSION_V0: &str = "0";

/// Exclusive v0 recursion limit for nested JSON arrays and objects.
///
/// A value may contain at most 127 nested containers. Entering the 128th
/// container is rejected, matching the default `serde_json` wire boundary.
pub const JSON_NESTING_LIMIT_V0: usize = 128;

/// Maximum recursive depth of one `flow-value-v0`, including its root.
pub const FLOW_VALUE_MAX_DEPTH_V0: usize = 16;
/// Maximum direct members in one `flow-value-v0` list or map.
pub const FLOW_VALUE_MAX_MEMBERS_V0: usize = 1_024;
/// Maximum tagged nodes in one `flow-value-v0`.
pub const FLOW_VALUE_MAX_NODES_V0: usize = 4_096;
/// Maximum canonical JSON bytes in one inline `flow-value-v0`.
pub const FLOW_VALUE_MAX_BYTES_V0: usize = 64 * 1024;
/// Maximum Unicode scalar values in one `flow-value-v0` map key.
pub const FLOW_VALUE_MAX_KEY_CHARS_V0: usize = 256;

const JSON_NESTING_REQUIREMENT_V0: &str = "must stay below the protocol v0 JSON nesting limit";

#[cfg(test)]
mod tests;
