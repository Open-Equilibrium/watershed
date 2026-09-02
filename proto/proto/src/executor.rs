use std::{fmt, time::Duration};

mod codec;
mod stream;
mod types;
mod validation;

pub use codec::*;
pub use stream::*;
pub use types::*;

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
/// Required official feature proving an enforced Tool process-and-thread capacity.
pub const EXECUTOR_FEATURE_PROCESS_CAPACITY_V0: &str = "process-capacity";
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
/// Grace after TERM reaches a live Tool tree before forced cleanup.
pub const TOOL_TERMINATION_GRACE_V0: Duration = Duration::from_secs(1);
/// Deadline for forced Tool-tree cleanup and supervisor reaping.
pub const TOOL_FORCED_REAP_DEADLINE_V0: Duration = Duration::from_secs(1);
/// Deadline for bounded output EOF after Tool-tree cleanup.
pub const TOOL_OUTPUT_DRAIN_DEADLINE_V0: Duration = Duration::from_secs(1);

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
