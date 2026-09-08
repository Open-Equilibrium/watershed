use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

/// One Flow Agent object protected from Tool writes, bound to an inherited descriptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorProtectedObjectV0 {
    /// Inherited descriptor number carrying the already-open object.
    pub descriptor: u32,
    /// Identity the Executor must verify against the descriptor and protected path.
    pub identity: UnixObjectIdentityV0,
    /// Canonical absolute host path to protect.
    pub path: String,
}

/// Fixed output and deadline limits for one Tool execution.
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
    /// Arguments after the executable name; an argument-free Tool uses an empty vector.
    pub argv: Vec<String>,
    /// Resolved environment passed to the Tool.
    pub environment: BTreeMap<String, String>,
    /// Canonical absolute host executable path.
    pub executable: String,
    /// Applied output and deadline limits.
    pub limits: ExecutorLimitsV0,
    /// Nonempty, bounded protected inventory in sequential inherited descriptor slots.
    pub protected_objects: Vec<ExecutorProtectedObjectV0>,
    /// Stable Tool identity.
    pub tool_id: String,
    /// Stable Tool kind.
    pub tool_kind: String,
    /// Canonical absolute host working directory.
    pub working_directory: String,
}

/// One validated Tool execution request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRequestV0 {
    /// Fully resolved target policy represented by the digest and receipt.
    pub resolved_policy: ExecutorResolvedPolicyV0,
    /// Lowercase SHA-256 of the exact canonical resolved-policy bytes plus LF.
    pub policy_digest: String,
    /// Opaque per-attempt identifier.
    pub request_id: String,
    /// Fixed schema name.
    pub schema: String,
}

/// Result of validating and preflighting one request before Tool launch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorPreflightV0 {
    /// The request is fully supported and may be started.
    Ready {
        /// Matching opaque request identifier.
        request_id: String,
        /// Fixed schema name.
        schema: String,
    },
    /// A definitive failure rejected the request before launch.
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

/// Exact command authorizing one successfully preflighted request to start.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorStartV0 {
    /// Matching opaque request identifier.
    pub request_id: String,
    /// Fixed schema name.
    pub schema: String,
}

/// Terminal attestation that the requested Flow Agent write protection was active.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnforcementReceiptV0 {
    /// Digest of the canonical resolved policy applied by the Executor; must equal the request's
    /// `policy_digest`.
    pub applied_policy_digest: String,
    /// Native protection backend identity.
    pub backend: String,
    /// Native protection backend version.
    pub backend_version: String,
    /// Executor identity.
    pub executor: String,
    /// Executor version.
    pub executor_version: String,
    /// Whether the requested Flow Agent objects were protected from Tool writes.
    pub self_protection_active: bool,
    /// Exact supported platform tuple.
    pub platform: String,
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
    /// Native protection setup failed before a proven Tool launch.
    SandboxSetupFailed,
}

/// Terminal status reported for the Tool root process and bounded output collection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorToolStatusV0 {
    /// Tool root process exited successfully and output collection completed.
    Completed,
    /// Tool execution ended unsuccessfully.
    Failed,
    /// Tool process exceeded its declared deadline.
    TimedOut,
    /// Flow requested cancellation and the Tool root process was reaped; descendants may survive.
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
    /// Flow requested cancellation and the Tool root process was reaped; descendants may survive.
    Cancelled,
}

/// Bounded binary-safe Tool result; it does not attest to descendant cleanup.
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
    /// Self-protection was active and the Executor produced a terminal Tool result.
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
    /// Native protection backend identity.
    pub backend: String,
    /// Native protection backend version.
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
    /// Fixed schema name.
    pub schema: String,
    /// Supported canonical policy features.
    pub supported_policy_features: Vec<String>,
}
