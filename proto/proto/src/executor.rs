use crate::{canonical_json, parse_unique_json};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt};

/// Closed schema name for one M1.2 Executor request.
pub const EXECUTOR_REQUEST_SCHEMA_V0: &str = "flow-executor-request-v0";
/// Closed schema name for one M1.2 Executor terminal response.
pub const EXECUTOR_RESPONSE_SCHEMA_V0: &str = "flow-executor-result-v0";
/// Closed schema name for one M1.2 Executor readiness response.
pub const EXECUTOR_PROBE_SCHEMA_V0: &str = "flow-executor-probe-v0";
/// Private one-shot protocol version supported by the official Executor.
pub const EXECUTOR_PROTOCOL_VERSION_V0: &str = "0";
/// Canonical official Executor identity.
pub const EXECUTOR_NAME_V0: &str = "flow-executor";
/// Canonical official isolation backend identity.
pub const EXECUTOR_BACKEND_V0: &str = "bubblewrap-seccomp";
/// Canonical official host platform identity.
pub const EXECUTOR_PLATFORM_V0: &str = "ubuntu-24.04-x86_64";
/// Closed Sandbox executable surface supported by the private v0 protocol.
pub const EXECUTOR_EXACT_EXECUTABLES_V0: [&str; 3] = ["/bin/sh", "/bin/cat", "/bin/echo"];
/// Required official feature proving the static trusted inner-stage image.
pub const EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0: &str = "static-self-reexec";
/// Required official feature proving descriptor-backed mount sources.
pub const EXECUTOR_FEATURE_DESCRIPTOR_MOUNTS_V0: &str = "descriptor-backed-mounts";
/// Required official feature proving source/destination identity checks.
pub const EXECUTOR_FEATURE_MOUNT_IDENTITY_V0: &str = "mount-identity-verification";
/// Required official feature proving a denied ambient network namespace.
pub const EXECUTOR_FEATURE_DENY_NETWORK_V0: &str = "deny-all-network";
/// Required official feature proving descendant process containment.
pub const EXECUTOR_FEATURE_PROCESS_CONTAINMENT_V0: &str = "pid-descendant-containment";
/// Maximum canonical request document bytes, including its final line feed.
pub const MAX_EXECUTOR_REQUEST_BYTES_V0: usize = 1024 * 1024;
/// Maximum raw bytes in either terminal Tool stream.
pub const MAX_EXECUTOR_TOOL_STREAM_BYTES_V0: usize = 4 * 1024 * 1024;
const MAX_EXECUTOR_ENCODED_STREAM_BYTES_V0: usize =
    MAX_EXECUTOR_TOOL_STREAM_BYTES_V0.div_ceil(3) * 4;
/// Maximum canonical response bytes for two encoded streams plus bounded metadata.
pub const MAX_EXECUTOR_RESPONSE_BYTES_V0: usize =
    2 * MAX_EXECUTOR_ENCODED_STREAM_BYTES_V0 + 64 * 1024;
/// Maximum canonical probe document bytes, including its final line feed.
pub const MAX_EXECUTOR_PROBE_BYTES_V0: usize = 64 * 1024;
/// Maximum pre-opened filesystem objects in one request.
pub const MAX_EXECUTOR_MOUNTS_V0: usize = 128;
/// Maximum configured workspace objects in one request.
pub const MAX_EXECUTOR_WORKSPACE_MOUNTS_V0: usize = 64;
/// Maximum read-only runtime objects advertised by one readiness probe.
pub const MAX_EXECUTOR_RUNTIME_MOUNTS_V0: usize = 64;
/// First inherited descriptor reserved for pre-opened filesystem objects.
pub const EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0: u32 = 32;

const MAX_ID_CHARS: usize = 256;
const MAX_NAME_CHARS: usize = 256;
const MAX_PATH_CHARS: usize = 4_096;
const MAX_ARGV_ENTRIES: usize = 2_048;
const MAX_EXEC_VECTOR_BYTES: usize = 128 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_FEATURES: usize = 256;
const MAX_ERROR_MESSAGE_CHARS: usize = 4_000;

/// Error returned when a private Executor v0 document violates its closed contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutorProtocolError(String);

impl ExecutorProtocolError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ExecutorProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ExecutorProtocolError {}

/// Runtime-read profile selected by the validated Tool policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeReadProfileV0 {
    /// Only the resolved executable, interpreter and library objects are exposed.
    Exact,
    /// The reviewed official Executor system-root set is exposed read-only.
    HostSystemRead,
}

/// Access granted to one inherited filesystem object.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorMountAccessV0 {
    /// Read-only access.
    ReadOnly,
    /// Read and write access.
    ReadWrite,
}

/// Provenance of one requested capability, kept explicit for closed union validation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorMountOriginV0 {
    /// A configured Tool workspace mount.
    Workspace,
    /// A readiness-manifest runtime object selected for the Tool profile.
    Runtime,
}

/// Filesystem object kind proven before descriptor inheritance.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorObjectKindV0 {
    /// Regular file.
    File,
    /// Directory.
    Directory,
}

/// Closed Unix identity record for one pre-opened filesystem object.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnixObjectIdentityV0 {
    /// Device identifier observed from the opened object.
    pub device: u64,
    /// Inode identifier observed from the opened object.
    pub inode: u64,
    /// Object kind observed from the opened object.
    pub kind: ExecutorObjectKindV0,
}

/// One pre-opened filesystem object inherited by the Executor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorMountV0 {
    /// Access granted at the Sandbox target.
    pub access: ExecutorMountAccessV0,
    /// Inherited descriptor number carrying the already-open object.
    pub descriptor: u32,
    /// Closed capability source class.
    pub origin: ExecutorMountOriginV0,
    /// Identity the Executor must verify on the inherited descriptor and mounted destination.
    pub source_identity: UnixObjectIdentityV0,
    /// Absolute Sandbox target path.
    pub target: String,
}

/// One fully resolved capability bound into the applied policy digest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorResolvedMountV0 {
    /// Access granted at the Sandbox target.
    pub access: ExecutorMountAccessV0,
    /// Inherited descriptor number carrying the already-open object.
    pub descriptor: u32,
    /// Closed capability source class.
    pub origin: ExecutorMountOriginV0,
    /// Manifest source path or canonical workspace policy path used to open the object.
    pub source: String,
    /// Identity proven on the retained source object.
    pub source_identity: UnixObjectIdentityV0,
    /// Absolute Sandbox target path.
    pub target: String,
}

/// Existing fixed process and output limits for one Tool execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorLimitsV0 {
    /// Maximum stderr bytes returned by the Executor.
    pub max_stderr_bytes: u64,
    /// Maximum stdout bytes returned by the Executor.
    pub max_stdout_bytes: u64,
    /// Tool deadline relative to accepted execution, in milliseconds.
    pub timeout_ms: u64,
}

/// Canonical fully resolved target policy whose digest is attested by the receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorResolvedPolicyV0 {
    /// Validated canonical core policy artifact.
    pub artifact: serde_json::Value,
    /// Exact selected command policy from that artifact.
    pub command: serde_json::Value,
    /// Applied process and output limits.
    pub limits: ExecutorLimitsV0,
    /// Exact retained capability union, including manifest/workspace source identities.
    pub mounts: Vec<ExecutorResolvedMountV0>,
    /// Selected runtime-read profile.
    pub runtime_profile: RuntimeReadProfileV0,
    /// Stable Tool identity.
    pub tool_id: String,
    /// Stable Tool kind.
    pub tool_kind: String,
}

/// One validated Tool execution request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRequestV0 {
    /// Arguments passed after the executable name; an argument-free Tool uses an empty vector.
    pub argv: Vec<String>,
    /// Environment explicitly granted by policy.
    pub environment: BTreeMap<String, String>,
    /// Prevalidated executable name or Sandbox path.
    pub executable: String,
    /// Fixed execution limits.
    pub limits: ExecutorLimitsV0,
    /// Pre-opened filesystem objects and their Sandbox targets.
    pub mounts: Vec<ExecutorMountV0>,
    /// Fully resolved target policy represented by the digest and receipt.
    pub resolved_policy: ExecutorResolvedPolicyV0,
    /// Lowercase SHA-256 of the exact canonical resolved-policy bytes plus LF.
    pub policy_digest: String,
    /// Opaque per-attempt identifier.
    pub request_id: String,
    /// Selected runtime-read profile.
    pub runtime_profile: RuntimeReadProfileV0,
    /// Fixed schema name.
    pub schema: String,
    /// Stable Tool identity.
    pub tool_id: String,
    /// Stable Tool kind.
    pub tool_kind: String,
    /// Absolute Sandbox working directory.
    pub working_directory: String,
}

/// Minimal terminal evidence that the requested isolation policy was active.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnforcementReceiptV0 {
    /// Lowercase SHA-256 of the exact policy artifact applied by the Sandbox.
    pub applied_policy_digest: String,
    /// Sandbox backend identity.
    pub backend: String,
    /// Sandbox backend version.
    pub backend_version: String,
    /// Executor identity.
    pub executor: String,
    /// Executor version.
    pub executor_version: String,
    /// Whether the isolation boundary was active for the Tool lifecycle.
    pub isolation_active: bool,
    /// Exact supported platform tuple.
    pub platform: String,
    /// Runtime-read profile applied by the Sandbox.
    pub runtime_profile: RuntimeReadProfileV0,
}

/// Stable private Executor integration error code.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorErrorCodeV0 {
    /// No usable configured Executor or readiness dependency exists.
    #[serde(rename = "executor_unavailable")]
    Unavailable,
    /// No mutually supported protocol version exists.
    #[serde(rename = "executor_protocol_mismatch")]
    ProtocolMismatch,
    /// Executor output violated the closed response contract.
    #[serde(rename = "executor_invalid_response")]
    InvalidResponse,
    /// The Executor cannot enforce the requested canonical policy.
    #[serde(rename = "executor_policy_unsupported")]
    PolicyUnsupported,
    /// Sandbox preparation failed before a proven Tool launch.
    SandboxSetupFailed,
}

/// Terminal status produced inside the active Executor isolation boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorToolStatusV0 {
    /// Tool process exited successfully.
    Completed,
    /// Tool execution ended unsuccessfully.
    Failed,
    /// Tool process exceeded its declared deadline.
    TimedOut,
    /// Flow requested controlled cancellation and the Tool tree was reaped.
    Cancelled,
}

/// Closed terminal classification produced by the Executor after Tool launch.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorToolClassificationV0 {
    /// Tool process returned a nonzero exit status.
    NonzeroExit,
    /// Tool process terminated from a signal.
    SignalTermination,
    /// Stderr exceeded its declared bound.
    StderrCapExceeded,
    /// Stdout exceeded its declared bound.
    StdoutCapExceeded,
    /// Both streams exceeded their declared bounds.
    StdoutStderrCapExceeded,
    /// Tool process exceeded its declared deadline.
    ToolTimedOut,
    /// A bounded output collector failed after Tool launch.
    OutputCollectorFailed,
    /// Bounded output collection did not drain after Tool termination.
    OutputDrainTimeout,
    /// Flow requested controlled cancellation and the Tool tree was reaped.
    Cancelled,
}

/// Bounded binary-safe Tool result produced inside the active isolation boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorToolResultV0 {
    /// Stable classification required for non-success terminals.
    pub classification: Option<ExecutorToolClassificationV0>,
    /// Visible Tool exit code, when a process exit produced one.
    pub exit_code: Option<i32>,
    /// Tool terminal status.
    pub status: ExecutorToolStatusV0,
    /// Canonical padded base64 stderr bytes.
    pub stderr_base64: String,
    /// Canonical padded base64 stdout bytes.
    pub stdout_base64: String,
}

/// Terminal response for exactly one Executor request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorResponseV0 {
    /// The Sandbox was active and produced a terminal Tool result.
    Completed {
        /// Minimal enforcement evidence.
        enforcement: EnforcementReceiptV0,
        /// Matching opaque request identifier.
        request_id: String,
        /// Fixed schema name.
        schema: String,
        /// Bounded binary-safe Tool result.
        tool_result: ExecutorToolResultV0,
    },
    /// A definitive setup failure occurred before any Tool process was launched.
    Error {
        /// Stable bounded failure code.
        code: ExecutorErrorCodeV0,
        /// Bounded redacted diagnostic.
        message: String,
        /// Matching opaque request identifier.
        request_id: String,
        /// Fixed schema name.
        schema: String,
    },
}

/// Result of a no-Tool-spawn Executor readiness probe.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorProbeV0 {
    /// Sandbox backend identity.
    pub backend: String,
    /// Sandbox backend version.
    pub backend_version: String,
    /// Executor identity.
    pub executor: String,
    /// Executor version.
    pub executor_version: String,
    /// Exact supported platform tuple.
    pub platform: String,
    /// Supported private protocol version strings.
    pub protocol_versions: Vec<String>,
    /// Whether the no-Tool-spawn readiness self-test passed.
    pub ready: bool,
    /// Runtime objects Flow Agent must pre-open for supported profile/executable pairs.
    pub runtime_mounts: Vec<ExecutorRuntimeMountV0>,
    /// Fixed schema name.
    pub schema: String,
    /// Supported canonical policy features.
    pub supported_policy_features: Vec<String>,
}

/// One read-only runtime object declared by a successful Executor probe.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRuntimeMountV0 {
    /// Executable Sandbox path this mount serves, or none for a profile-wide root.
    pub executable: Option<String>,
    /// Runtime-read profile selecting this mount.
    pub runtime_profile: RuntimeReadProfileV0,
    /// Absolute administrator-owned host source path.
    pub source: String,
    /// Absolute Sandbox target path.
    pub target: String,
}

/// Returns the lowercase SHA-256 of the canonical resolved target policy plus its required LF.
pub fn resolved_policy_digest_v0(
    policy: &ExecutorResolvedPolicyV0,
) -> Result<String, ExecutorProtocolError> {
    let value = serde_json::to_value(policy).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid resolved Executor policy: {error}"))
    })?;
    let mut canonical = canonical_json(&value).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid resolved Executor policy: {error}"))
    })?;
    canonical.push('\n');
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Validates one receipt against the resolved policy and runtime profile Flow requested.
pub fn validate_enforcement_receipt_v0(
    receipt: &EnforcementReceiptV0,
    expected_policy_digest: &str,
    expected_runtime_profile: RuntimeReadProfileV0,
) -> Result<(), ExecutorProtocolError> {
    validate_receipt(receipt)?;
    if !receipt.isolation_active
        || receipt.applied_policy_digest != expected_policy_digest
        || receipt.runtime_profile != expected_runtime_profile
    {
        return Err(ExecutorProtocolError::new(
            "Executor enforcement receipt does not match the requested isolation policy",
        ));
    }
    Ok(())
}

/// Serializes and validates one canonical Executor request plus LF.
pub fn canonical_executor_request_v0(
    request: &ExecutorRequestV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    validate_request(request)?;
    let value = serde_json::to_value(request).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid Executor request: {error}"))
    })?;
    let mut bytes = canonical_json(&value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor request: {error}")))?
        .into_bytes();
    bytes.push(b'\n');
    if bytes.len() > MAX_EXECUTOR_REQUEST_BYTES_V0 {
        return Err(ExecutorProtocolError::new(
            "Executor request exceeds its byte limit",
        ));
    }
    Ok(bytes)
}

/// Serializes and validates one canonical Executor response plus LF.
pub fn canonical_executor_response_v0(
    response: &ExecutorResponseV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let (request_id, policy_digest) = match response {
        ExecutorResponseV0::Completed {
            request_id,
            enforcement,
            ..
        } => (
            request_id.as_str(),
            enforcement.applied_policy_digest.as_str(),
        ),
        ExecutorResponseV0::Error { request_id, .. } => (request_id.as_str(), ""),
    };
    let bytes = canonical_document(response, MAX_EXECUTOR_RESPONSE_BYTES_V0, "response")?;
    parse_executor_response_v0(&bytes, request_id, policy_digest)?;
    Ok(bytes)
}

/// Serializes and validates one canonical Executor probe plus LF.
pub fn canonical_executor_probe_v0(
    probe: &ExecutorProbeV0,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let bytes = canonical_document(probe, MAX_EXECUTOR_PROBE_BYTES_V0, "probe")?;
    parse_executor_probe_v0(&bytes)?;
    Ok(bytes)
}

/// Parses one exact canonical LF-terminated Executor request.
pub fn parse_executor_request_v0(bytes: &[u8]) -> Result<ExecutorRequestV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_REQUEST_BYTES_V0, "request")?;
    let request: ExecutorRequestV0 = serde_json::from_value(value).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid Executor request: {error}"))
    })?;
    validate_request(&request)?;
    Ok(request)
}

/// Parses and validates one canonical Executor terminal response.
pub fn parse_executor_response_v0(
    bytes: &[u8],
    expected_request_id: &str,
    expected_policy_digest: &str,
) -> Result<ExecutorResponseV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_RESPONSE_BYTES_V0, "response")?;
    let response: ExecutorResponseV0 = serde_json::from_value(value).map_err(|error| {
        ExecutorProtocolError::new(format!("invalid Executor response: {error}"))
    })?;
    let (schema, request_id) = match &response {
        ExecutorResponseV0::Completed {
            schema, request_id, ..
        }
        | ExecutorResponseV0::Error {
            schema, request_id, ..
        } => (schema, request_id),
    };
    validate_schema(schema, EXECUTOR_RESPONSE_SCHEMA_V0, "response")?;
    validate_text(request_id, "request_id", MAX_ID_CHARS)?;
    if request_id != expected_request_id {
        return Err(ExecutorProtocolError::new(
            "Executor response request id does not match",
        ));
    }
    match &response {
        ExecutorResponseV0::Completed {
            enforcement,
            tool_result,
            ..
        } => {
            validate_receipt(enforcement)?;
            validate_tool_result(tool_result)?;
            if enforcement.applied_policy_digest != expected_policy_digest {
                return Err(ExecutorProtocolError::new(
                    "Executor applied the wrong policy digest",
                ));
            }
            if !enforcement.isolation_active {
                return Err(ExecutorProtocolError::new(
                    "Executor isolation was not active",
                ));
            }
        }
        ExecutorResponseV0::Error { message, .. } => {
            validate_text(message, "error message", MAX_ERROR_MESSAGE_CHARS)?;
        }
    }
    Ok(response)
}

/// Parses and validates one canonical Executor readiness response.
pub fn parse_executor_probe_v0(bytes: &[u8]) -> Result<ExecutorProbeV0, ExecutorProtocolError> {
    let value = parse_canonical_document(bytes, MAX_EXECUTOR_PROBE_BYTES_V0, "probe")?;
    let probe: ExecutorProbeV0 = serde_json::from_value(value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor probe: {error}")))?;
    validate_schema(&probe.schema, EXECUTOR_PROBE_SCHEMA_V0, "probe")?;
    for (name, value) in [
        ("executor", &probe.executor),
        ("executor_version", &probe.executor_version),
        ("backend", &probe.backend),
        ("backend_version", &probe.backend_version),
        ("platform", &probe.platform),
    ] {
        validate_text(value, name, MAX_NAME_CHARS)?;
    }
    if probe.protocol_versions.is_empty()
        || probe.protocol_versions.len() > MAX_FEATURES
        || probe.supported_policy_features.len() > MAX_FEATURES
        || probe.runtime_mounts.len() > MAX_EXECUTOR_RUNTIME_MOUNTS_V0
    {
        return Err(ExecutorProtocolError::new(
            "Executor probe list bounds are invalid",
        ));
    }
    for value in probe
        .protocol_versions
        .iter()
        .chain(&probe.supported_policy_features)
    {
        validate_text(value, "probe feature", MAX_NAME_CHARS)?;
    }
    let mut mount_keys = std::collections::BTreeSet::new();
    let mut mount_targets = std::collections::BTreeSet::new();
    for mount in &probe.runtime_mounts {
        validate_text(&mount.source, "runtime mount source", MAX_PATH_CHARS)?;
        validate_text(&mount.target, "runtime mount target", MAX_PATH_CHARS)?;
        validate_absolute_path(&mount.source, "runtime mount source")?;
        validate_absolute_path(&mount.target, "runtime mount target")?;
        if let Some(executable) = &mount.executable {
            validate_text(executable, "runtime mount executable", MAX_PATH_CHARS)?;
            validate_absolute_path(executable, "runtime mount executable")?;
            if !EXECUTOR_EXACT_EXECUTABLES_V0.contains(&executable.as_str()) {
                return Err(ExecutorProtocolError::new(
                    "Executor runtime mount names an unsupported executable",
                ));
            }
        } else if mount.runtime_profile == RuntimeReadProfileV0::Exact {
            return Err(ExecutorProtocolError::new(
                "exact Executor runtime mounts must name an executable",
            ));
        }
        let key = (
            mount.runtime_profile as u8,
            mount.executable.as_deref(),
            mount.source.as_str(),
        );
        let target_key = (
            mount.runtime_profile as u8,
            mount.executable.as_deref(),
            mount.target.as_str(),
        );
        if !mount_keys.insert(key) || !mount_targets.insert(target_key) {
            return Err(ExecutorProtocolError::new(
                "Executor runtime mount manifest contains duplicates",
            ));
        }
    }
    Ok(probe)
}

fn validate_request(request: &ExecutorRequestV0) -> Result<(), ExecutorProtocolError> {
    validate_schema(&request.schema, EXECUTOR_REQUEST_SCHEMA_V0, "request")?;
    validate_text(&request.request_id, "request_id", MAX_ID_CHARS)?;
    validate_text(&request.tool_id, "tool_id", MAX_ID_CHARS)?;
    validate_text(&request.tool_kind, "tool_kind", MAX_NAME_CHARS)?;
    validate_text(&request.executable, "executable", MAX_PATH_CHARS)?;
    validate_absolute_path(&request.executable, "executable")?;
    validate_text(
        &request.working_directory,
        "working_directory",
        MAX_PATH_CHARS,
    )?;
    validate_absolute_path(&request.working_directory, "working_directory")?;
    if request.argv.len() > MAX_ARGV_ENTRIES {
        return Err(ExecutorProtocolError::new(
            "Executor argv entry bound is invalid",
        ));
    }
    let exec_bytes = request
        .argv
        .iter()
        .try_fold(request.executable.len(), |total, value| {
            validate_argv_entry(value)?;
            total
                .checked_add(value.len() + 1)
                .ok_or_else(|| ExecutorProtocolError::new("Executor argv byte count overflow"))
        })?;
    if exec_bytes > MAX_EXEC_VECTOR_BYTES {
        return Err(ExecutorProtocolError::new(
            "Executor argv exceeds its byte limit",
        ));
    }
    if request.environment.len() > MAX_ENVIRONMENT_ENTRIES {
        return Err(ExecutorProtocolError::new(
            "Executor environment has too many entries",
        ));
    }
    for (name, value) in &request.environment {
        validate_text(name, "environment name", MAX_NAME_CHARS)?;
        validate_text(value, "environment value", MAX_PATH_CHARS)?;
    }
    if request.mounts.len() > MAX_EXECUTOR_MOUNTS_V0 {
        return Err(ExecutorProtocolError::new(
            "Executor mount list exceeds its limit",
        ));
    }
    let mut descriptors = std::collections::BTreeSet::new();
    let mut targets = std::collections::BTreeSet::new();
    let mut runtime_mounts = 0_usize;
    let mut workspace_mounts = 0_usize;
    for (index, mount) in request.mounts.iter().enumerate() {
        let expected_descriptor = EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0
            .checked_add(u32::try_from(index).expect("mount limit fits u32"))
            .expect("mount descriptor range is bounded");
        if mount.descriptor != expected_descriptor || !descriptors.insert(mount.descriptor) {
            return Err(ExecutorProtocolError::new(
                "Executor mount descriptor is invalid",
            ));
        }
        validate_text(&mount.target, "mount target", MAX_PATH_CHARS)?;
        validate_absolute_path(&mount.target, "mount target")?;
        if !targets.insert(mount.target.as_str()) {
            return Err(ExecutorProtocolError::new(
                "Executor mount target is invalid",
            ));
        }
        match mount.origin {
            ExecutorMountOriginV0::Workspace => {
                workspace_mounts += 1;
                if mount.target != "/workspace" && !mount.target.starts_with("/workspace/") {
                    return Err(ExecutorProtocolError::new(
                        "Executor workspace mount target is outside /workspace",
                    ));
                }
            }
            ExecutorMountOriginV0::Runtime => {
                runtime_mounts += 1;
                if mount.access != ExecutorMountAccessV0::ReadOnly
                    || mount.target == "/workspace"
                    || mount.target.starts_with("/workspace/")
                {
                    return Err(ExecutorProtocolError::new(
                        "Executor runtime mount capability is invalid",
                    ));
                }
            }
        }
    }
    if workspace_mounts > MAX_EXECUTOR_WORKSPACE_MOUNTS_V0
        || runtime_mounts > MAX_EXECUTOR_RUNTIME_MOUNTS_V0
    {
        return Err(ExecutorProtocolError::new(
            "Executor mount provenance bounds are invalid",
        ));
    }
    validate_resolved_policy(request)?;
    validate_digest(&request.policy_digest, "policy_digest")?;
    if resolved_policy_digest_v0(&request.resolved_policy)? != request.policy_digest {
        return Err(ExecutorProtocolError::new(
            "Executor request policy digest does not match",
        ));
    }
    if request.limits.timeout_ms == 0
        || request.limits.max_stdout_bytes == 0
        || request.limits.max_stderr_bytes == 0
    {
        return Err(ExecutorProtocolError::new(
            "Executor limits must be nonzero",
        ));
    }
    if request.limits.max_stdout_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
        || request.limits.max_stderr_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
    {
        return Err(ExecutorProtocolError::new(
            "Executor stream limits exceed the protocol bound",
        ));
    }
    Ok(())
}

fn validate_resolved_policy(request: &ExecutorRequestV0) -> Result<(), ExecutorProtocolError> {
    let policy = &request.resolved_policy;
    validate_text(&policy.tool_id, "resolved policy tool_id", MAX_ID_CHARS)?;
    validate_text(
        &policy.tool_kind,
        "resolved policy tool_kind",
        MAX_NAME_CHARS,
    )?;
    if !policy.artifact.is_object() || !policy.command.is_object() {
        return Err(ExecutorProtocolError::new(
            "Executor resolved policy artifacts must be objects",
        ));
    }
    if policy.tool_id != request.tool_id
        || policy.tool_kind != request.tool_kind
        || policy.runtime_profile != request.runtime_profile
        || policy.limits != request.limits
        || policy.mounts.len() != request.mounts.len()
    {
        return Err(ExecutorProtocolError::new(
            "Executor resolved policy does not match the request",
        ));
    }
    for (resolved, requested) in policy.mounts.iter().zip(&request.mounts) {
        validate_text(&resolved.source, "resolved mount source", MAX_PATH_CHARS)?;
        let source_is_valid = match resolved.origin {
            ExecutorMountOriginV0::Workspace => validate_workspace_source(&resolved.source).is_ok(),
            ExecutorMountOriginV0::Runtime => {
                validate_absolute_path(&resolved.source, "resolved runtime mount source").is_ok()
            }
        };
        if !source_is_valid
            || resolved.access != requested.access
            || resolved.descriptor != requested.descriptor
            || resolved.origin != requested.origin
            || resolved.source_identity != requested.source_identity
            || resolved.target != requested.target
        {
            return Err(ExecutorProtocolError::new(
                "Executor resolved mount does not match the inherited capability",
            ));
        }
    }
    Ok(())
}

fn validate_receipt(receipt: &EnforcementReceiptV0) -> Result<(), ExecutorProtocolError> {
    validate_digest(&receipt.applied_policy_digest, "applied_policy_digest")?;
    for (name, value) in [
        ("executor", &receipt.executor),
        ("executor_version", &receipt.executor_version),
        ("backend", &receipt.backend),
        ("backend_version", &receipt.backend_version),
        ("platform", &receipt.platform),
    ] {
        validate_text(value, name, MAX_NAME_CHARS)?;
    }
    Ok(())
}

fn validate_tool_result(result: &ExecutorToolResultV0) -> Result<(), ExecutorProtocolError> {
    use ExecutorToolClassificationV0 as Classification;
    use ExecutorToolStatusV0 as Status;

    let valid_terminal = match (result.status, result.classification, result.exit_code) {
        (Status::Completed, None, Some(0)) => true,
        (Status::Failed, Some(Classification::NonzeroExit), Some(code)) => code != 0,
        (Status::Failed, Some(Classification::SignalTermination), None) => true,
        (
            Status::Failed,
            Some(
                Classification::StderrCapExceeded
                | Classification::StdoutCapExceeded
                | Classification::StdoutStderrCapExceeded
                | Classification::OutputCollectorFailed
                | Classification::OutputDrainTimeout,
            ),
            _,
        ) => true,
        (Status::TimedOut, Some(Classification::ToolTimedOut), None) => true,
        (Status::Cancelled, Some(Classification::Cancelled), None) => true,
        _ => false,
    };
    if !valid_terminal {
        return Err(ExecutorProtocolError::new(
            "Executor Tool result has an invalid terminal state",
        ));
    }
    for (name, encoded) in [
        ("stdout", &result.stdout_base64),
        ("stderr", &result.stderr_base64),
    ] {
        let decoded = decode_executor_stream_v0(encoded)?;
        if decoded.len() > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 {
            return Err(ExecutorProtocolError::new(format!(
                "Executor Tool {name} exceeds its byte limit"
            )));
        }
    }
    Ok(())
}

/// Encodes arbitrary Tool stream bytes as canonical padded standard base64.
pub fn encode_executor_stream_v0(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(ALPHABET[usize::from(first >> 2)] as char);
        encoded.push(ALPHABET[usize::from((first & 0x03) << 4 | second >> 4)] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[usize::from((second & 0x0f) << 2 | third >> 6)] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[usize::from(third & 0x3f)] as char
        } else {
            '='
        });
    }
    encoded
}

/// Decodes canonical padded standard base64 Tool stream bytes.
pub fn decode_executor_stream_v0(encoded: &str) -> Result<Vec<u8>, ExecutorProtocolError> {
    if !encoded.len().is_multiple_of(4) || !encoded.is_ascii() {
        return Err(invalid_base64());
    }
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    let (chunks, remainder) = bytes.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    for (index, chunk) in chunks.iter().enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' || b & 0x0f != 0 {
                return Err(invalid_base64());
            }
            None
        } else {
            Some(base64_value(chunk[2])?)
        };
        let d = if chunk[3] == b'=' {
            if !last || c.is_some_and(|value| value & 0x03 != 0) {
                return Err(invalid_base64());
            }
            None
        } else {
            Some(base64_value(chunk[3])?)
        };
        decoded.push(a << 2 | b >> 4);
        if let Some(c) = c {
            decoded.push(b << 4 | c >> 2);
            if let Some(d) = d {
                decoded.push(c << 6 | d);
            }
        }
    }
    if encode_executor_stream_v0(&decoded) != encoded {
        return Err(invalid_base64());
    }
    Ok(decoded)
}

fn base64_value(byte: u8) -> Result<u8, ExecutorProtocolError> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(invalid_base64()),
    }
}

fn invalid_base64() -> ExecutorProtocolError {
    ExecutorProtocolError::new("Executor Tool stream is not canonical base64")
}

fn parse_canonical_document(
    bytes: &[u8],
    limit: usize,
    kind: &str,
) -> Result<serde_json::Value, ExecutorProtocolError> {
    if bytes.len() > limit {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} exceeds its byte limit"
        )));
    }
    if !bytes.ends_with(b"\n") || bytes[..bytes.len().saturating_sub(1)].contains(&b'\n') {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} must be one LF-terminated canonical JSON document"
        )));
    }
    let body = std::str::from_utf8(&bytes[..bytes.len() - 1])
        .map_err(|_| ExecutorProtocolError::new(format!("Executor {kind} is not UTF-8")))?;
    let value = parse_unique_json(body)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?;
    let canonical = canonical_json(&value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?;
    if canonical.as_bytes() != body.as_bytes() {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} is not canonical JSON"
        )));
    }
    Ok(value)
}

fn canonical_document<T: Serialize>(
    document: &T,
    limit: usize,
    kind: &str,
) -> Result<Vec<u8>, ExecutorProtocolError> {
    let value = serde_json::to_value(document)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?;
    let mut bytes = canonical_json(&value)
        .map_err(|error| ExecutorProtocolError::new(format!("invalid Executor {kind}: {error}")))?
        .into_bytes();
    bytes.push(b'\n');
    if bytes.len() > limit {
        return Err(ExecutorProtocolError::new(format!(
            "Executor {kind} exceeds its byte limit"
        )));
    }
    Ok(bytes)
}

fn validate_schema(actual: &str, expected: &str, kind: &str) -> Result<(), ExecutorProtocolError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ExecutorProtocolError::new(format!(
            "unsupported Executor {kind} schema"
        )))
    }
}

fn validate_digest(value: &str, name: &str) -> Result<(), ExecutorProtocolError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ExecutorProtocolError::new(format!(
            "Executor {name} is not lowercase SHA-256"
        )))
    }
}

fn validate_text(value: &str, name: &str, max_chars: usize) -> Result<(), ExecutorProtocolError> {
    let count = value.chars().count();
    if count == 0 || count > max_chars || value.chars().any(char::is_control) {
        Err(ExecutorProtocolError::new(format!(
            "Executor {name} is invalid"
        )))
    } else {
        Ok(())
    }
}

fn validate_argv_entry(value: &str) -> Result<(), ExecutorProtocolError> {
    if value.contains('\0') {
        Err(ExecutorProtocolError::new("Executor argv is invalid"))
    } else {
        Ok(())
    }
}

fn validate_absolute_path(value: &str, name: &str) -> Result<(), ExecutorProtocolError> {
    if value == "/"
        || !value.starts_with('/')
        || value.ends_with('/')
        || value.contains('\\')
        || value[1..]
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        Err(ExecutorProtocolError::new(format!(
            "Executor {name} is not a canonical absolute path"
        )))
    } else {
        Ok(())
    }
}

fn validate_workspace_source(value: &str) -> Result<(), ExecutorProtocolError> {
    if value == "workspace"
        || value.strip_prefix("workspace/").is_some_and(|relative| {
            !relative.is_empty()
                && !relative.contains('\\')
                && relative
                    .split('/')
                    .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
        })
    {
        Ok(())
    } else {
        Err(ExecutorProtocolError::new(
            "Executor workspace mount source is not canonical",
        ))
    }
}
