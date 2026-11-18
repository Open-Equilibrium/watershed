use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorObjectKindV0 {
    File,
    Directory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnixObjectIdentityV0 {
    pub device: u64,
    pub inode: u64,
    pub kind: ExecutorObjectKindV0,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorProtectedObjectV0 {
    pub descriptor: u32,
    pub identity: UnixObjectIdentityV0,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorLimitsV0 {
    pub max_stderr_bytes: u64,
    pub max_stdout_bytes: u64,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorResolvedPolicyV0 {
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub executable: String,
    pub limits: ExecutorLimitsV0,
    pub protected_objects: Vec<ExecutorProtectedObjectV0>,
    pub tool_id: String,
    pub tool_kind: String,
    pub working_directory: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRequestV0 {
    pub resolved_policy: ExecutorResolvedPolicyV0,
    pub policy_digest: String,
    pub request_id: String,
    pub schema: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorPreflightV0 {
    Ready {
        request_id: String,
        schema: String,
    },
    Error {
        code: ExecutorErrorCodeV0,
        message: String,
        request_id: String,
        schema: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorStartV0 {
    pub request_id: String,
    pub schema: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnforcementReceiptV0 {
    pub applied_policy_digest: String,
    pub backend: String,
    pub backend_version: String,
    pub executor: String,
    pub executor_version: String,
    pub flow_owned_write_protection_active: bool,
    pub platform: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorErrorCodeV0 {
    #[serde(rename = "executor_unavailable")]
    Unavailable,
    #[serde(rename = "executor_protocol_mismatch")]
    ProtocolMismatch,
    #[serde(rename = "executor_invalid_response")]
    InvalidResponse,
    #[serde(rename = "executor_policy_unsupported")]
    PolicyUnsupported,
    FlowOwnedWriteProtectionSetupFailed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorToolStatusV0 {
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorToolClassificationV0 {
    NonzeroExit,
    SignalTermination,
    StderrCapExceeded,
    StdoutCapExceeded,
    StdoutStderrCapExceeded,
    ToolTimedOut,
    OutputCollectorFailed,
    OutputDrainTimeout,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorToolResultV0 {
    pub classification: Option<ExecutorToolClassificationV0>,
    pub exit_code: Option<i32>,
    pub status: ExecutorToolStatusV0,
    pub stderr_base64: String,
    pub stdout_base64: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorResponseV0 {
    Completed {
        enforcement: EnforcementReceiptV0,
        request_id: String,
        schema: String,
        tool_result: ExecutorToolResultV0,
    },
    Error {
        code: ExecutorErrorCodeV0,
        message: String,
        request_id: String,
        schema: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorProbeV0 {
    pub backend: String,
    pub backend_version: String,
    pub executor: String,
    pub executor_version: String,
    pub platform: String,
    pub protocol_versions: Vec<String>,
    pub ready: bool,
    pub schema: String,
    pub supported_policy_features: Vec<String>,
}
