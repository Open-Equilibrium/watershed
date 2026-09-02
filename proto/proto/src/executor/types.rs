use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

/// Fixed capacity, output, and deadline limits for one Tool execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorLimitsV0 {
    /// Maximum concurrent Tool processes and threads, including descendants.
    pub max_concurrent_processes_and_threads: u32,
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
    /// Enforced maximum concurrent Tool processes and threads.
    pub max_concurrent_processes_and_threads: u32,
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
    /// Tool process tree exhausted its configured process-and-thread capacity.
    ProcessCapacityExceeded,
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
