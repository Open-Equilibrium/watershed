use std::{collections::BTreeMap, fmt, time::Duration};

mod codec;
mod stream;
mod types;
mod validation;

pub use codec::*;
pub use stream::*;
pub use types::*;

/// Closed schema name for one M1.2 Executor request.
pub const EXECUTOR_REQUEST_SCHEMA_V0: &str = "flow-executor-request-v0";
/// Closed schema name for one M1.2 Executor preflight response.
pub const EXECUTOR_PREFLIGHT_SCHEMA_V0: &str = "flow-executor-preflight-v0";
/// Closed schema name for one M1.2 Executor start record.
pub const EXECUTOR_START_SCHEMA_V0: &str = "flow-executor-start-v0";
/// Closed schema name for one M1.2 Executor terminal response.
pub const EXECUTOR_RESPONSE_SCHEMA_V0: &str = "flow-executor-result-v0";
/// Closed schema name for one M1.2 Executor readiness response.
pub const EXECUTOR_PROBE_SCHEMA_V0: &str = "flow-executor-probe-v0";
/// Private one-shot protocol version supported by the official Executor.
pub const EXECUTOR_PROTOCOL_VERSION_V0: &str = "0";
/// Canonical official Executor identity.
pub const EXECUTOR_NAME_V0: &str = "flow-executor";
/// Productive executable used for POSIX own-script Tools.
pub const EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0: &str = "/bin/sh";
/// Required feature for write protection of this Flow Agent installation's objects.
pub const EXECUTOR_FEATURE_SELF_PROTECTION_V0: &str = "flow-owned-write-protection";
/// Maximum canonical request document bytes, including its final line feed.
pub const MAX_EXECUTOR_REQUEST_BYTES_V0: usize = 1024 * 1024;
/// Maximum canonical bytes in either preflight or start control record.
pub const MAX_EXECUTOR_CONTROL_BYTES_V0: usize = 8 * 1024;
/// Maximum raw bytes in either terminal Tool stream.
pub const MAX_EXECUTOR_TOOL_STREAM_BYTES_V0: usize = 4 * 1024 * 1024;
/// Maximum executable-plus-argument entries in one Tool exec vector.
pub const MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0: usize = 2_048;
/// Maximum encoded bytes in one Tool exec vector.
pub const MAX_EXECUTOR_EXEC_VECTOR_BYTES_V0: usize = 128 * 1024;
const MAX_EXECUTOR_ENCODED_STREAM_BYTES_V0: usize =
    MAX_EXECUTOR_TOOL_STREAM_BYTES_V0.div_ceil(3) * 4;
/// Maximum canonical response bytes for two encoded streams plus bounded metadata.
pub const MAX_EXECUTOR_RESPONSE_BYTES_V0: usize =
    2 * MAX_EXECUTOR_ENCODED_STREAM_BYTES_V0 + 64 * 1024;
/// Maximum canonical probe document bytes, including its final line feed.
pub const MAX_EXECUTOR_PROBE_BYTES_V0: usize = 64 * 1024;
/// Maximum pre-opened protected filesystem objects in one request.
pub const MAX_EXECUTOR_PROTECTED_OBJECTS_V0: usize = 128;
/// First inherited descriptor reserved for pre-opened protected objects.
pub const EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0: u32 = 32;
/// Grace after requesting Tool termination before forced root-process termination.
pub const TOOL_TERMINATION_GRACE_V0: Duration = Duration::from_secs(1);
/// Deadline for forced root-process termination and reaping; descendants may survive.
pub const TOOL_FORCED_REAP_DEADLINE_V0: Duration = Duration::from_secs(1);
/// Deadline for output EOF after root-process termination, even if descendants retain pipes.
pub const TOOL_OUTPUT_DRAIN_DEADLINE_V0: Duration = Duration::from_secs(1);

const MAX_ID_CHARS: usize = 256;
const MAX_NAME_CHARS: usize = 256;
const MAX_PATH_CHARS: usize = 4_096;
const MAX_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_FEATURES: usize = 256;
const MAX_ERROR_MESSAGE_CHARS: usize = 4_000;

/// Reason a process execution vector violates its v0 protocol bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutorExecVectorErrorV0 {
    /// An executable, argument, environment name or environment value contains a NUL byte.
    NulByte,
    /// The complete vector contains too many entries.
    EntryBudget {
        /// Complete entry count, including the executable.
        actual: usize,
    },
    /// The complete encoded vector contains too many bytes.
    ByteBudget {
        /// Encoded byte count, including NUL terminators and pointer arrays.
        actual: usize,
    },
}

/// Validates and accounts for one complete v0 Tool exec vector.
///
/// The returned size includes every executable, argument and `name=value`
/// environment byte, one NUL terminator per string, and the complete
/// null-terminated argument and environment pointer arrays on the current
/// pointer width.
pub fn validate_executor_exec_vector_v0(
    executable: &str,
    argv: &[String],
    environment: &BTreeMap<String, String>,
) -> Result<usize, ExecutorExecVectorErrorV0> {
    if executable.contains('\0')
        || argv.iter().any(|value| value.contains('\0'))
        || environment
            .iter()
            .any(|(name, value)| name.contains('\0') || value.contains('\0'))
    {
        return Err(ExecutorExecVectorErrorV0::NulByte);
    }
    let entries = argv
        .len()
        .checked_add(1)
        .ok_or(ExecutorExecVectorErrorV0::EntryBudget { actual: usize::MAX })?;
    if entries > MAX_EXECUTOR_EXEC_VECTOR_ENTRIES_V0 {
        return Err(ExecutorExecVectorErrorV0::EntryBudget { actual: entries });
    }
    let string_bytes = std::iter::once(executable)
        .chain(argv.iter().map(String::as_str))
        .try_fold(0_usize, |total, value| {
            total.checked_add(value.len().checked_add(1)?)
        })
        .ok_or(ExecutorExecVectorErrorV0::ByteBudget { actual: usize::MAX })?;
    let string_bytes = environment
        .iter()
        .try_fold(string_bytes, |total, (name, value)| {
            total
                .checked_add(name.len())?
                .checked_add(1)?
                .checked_add(value.len())?
                .checked_add(1)
        })
        .ok_or(ExecutorExecVectorErrorV0::ByteBudget { actual: usize::MAX })?;
    let pointer_bytes = entries
        .checked_add(environment.len())
        .and_then(|count| count.checked_add(2))
        .and_then(|count| count.checked_mul(std::mem::size_of::<usize>()))
        .ok_or(ExecutorExecVectorErrorV0::ByteBudget { actual: usize::MAX })?;
    let encoded_bytes = string_bytes
        .checked_add(pointer_bytes)
        .ok_or(ExecutorExecVectorErrorV0::ByteBudget { actual: usize::MAX })?;
    if encoded_bytes > MAX_EXECUTOR_EXEC_VECTOR_BYTES_V0 {
        return Err(ExecutorExecVectorErrorV0::ByteBudget {
            actual: encoded_bytes,
        });
    }
    Ok(encoded_bytes)
}

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
