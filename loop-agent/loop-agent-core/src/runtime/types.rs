use core_policy::{
    protected_path_match_mode_for_policy_target, protected_path_pattern_matches,
    ProtectedPathMatchMode,
};
use proto::{EventEnvelope, EventType};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Workspace-relative directory containing persisted session JSONL logs.
pub const LOCAL_SESSION_DIR: &str = ".loop/sessions";
/// Workspace-relative directory containing structured sidecar run logs.
pub const LOCAL_LOG_DIR: &str = ".loop/logs";
/// Maximum bytes accepted for one persisted session log.
pub const MAX_SESSION_LOG_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum canonical JSONL bytes emitted by one loop run.
pub const MAX_LOOP_EVENT_STREAM_BYTES: usize = 10 * 1024 * 1024;
/// Maximum events emitted by one loop run.
pub const MAX_LOOP_EVENTS: u64 = 64 * 1024;
/// Maximum loop invocations executed by one loop run.
pub const MAX_LOOP_INVOCATIONS: u64 = 8 * 1024;
const MAX_WORKSPACE_CONFIG_BYTES: u64 = core_script::MAX_REGISTRY_FILE_BYTES;
const TAIL_TRANSIENT_READ_RETRY_ATTEMPTS: usize = 200;
const TAIL_TRANSIENT_READ_RETRY_MS: u64 = 5;
const FIXTURE_CLOCK_UNIX_SECONDS: i64 = 1_767_225_600;
const RUNTIME_ERROR_REASON: &str = "runtime_error";
const TRUSTED_PREDEFINED_COMMANDS: &[TrustedPredefinedCommand] = &[
    TrustedPredefinedCommand {
        command_id: "agent-echo",
        progress: None,
    },
    TrustedPredefinedCommand {
        command_id: "agent-negative",
        progress: None,
    },
    TrustedPredefinedCommand {
        command_id: "agent-read",
        progress: Some("stub read completed"),
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TrustedPredefinedCommand {
    command_id: &'static str,
    progress: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EventClock {
    base_unix_seconds: i64,
}

impl EventClock {
    fn fixed_fixture() -> Self {
        Self {
            base_unix_seconds: FIXTURE_CLOCK_UNIX_SECONDS,
        }
    }

    fn wall_clock() -> Self {
        let base_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self { base_unix_seconds }
    }

    fn from_first_event(event: &EventEnvelope) -> Option<Self> {
        parse_rfc3339_utc_timestamp(&event.timestamp).map(|base_unix_seconds| Self {
            base_unix_seconds: base_unix_seconds.saturating_sub(
                i64::try_from(event.sequence.saturating_sub(1)).unwrap_or(i64::MAX),
            ),
        })
    }

    fn timestamp(self, sequence: u64) -> String {
        let offset = i64::try_from(sequence.saturating_sub(1)).unwrap_or(i64::MAX);
        format_unix_timestamp(self.base_unix_seconds.saturating_add(offset))
    }
}

/// Runtime surfaces tracked by the Loop Agent MVP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSurface {
    /// Human CLI commands.
    HumanCli,
    /// Headless JSONL event stream.
    JsonlEventStream,
    /// Local append-only session log.
    LocalSessionLog,
    /// Session tail, replay and resume commands.
    TailReplayResume,
    /// Designed future RPC surface.
    DesignedRpc,
    /// Designed future embedded core API.
    FutureEmbeddedCoreApi,
}

/// Returns the runtime surfaces implemented in M1.
pub fn m1_runtime_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::HumanCli,
        RuntimeSurface::JsonlEventStream,
        RuntimeSurface::LocalSessionLog,
        RuntimeSurface::TailReplayResume,
    ]
}

/// Returns runtime surfaces intentionally deferred beyond M1.
pub fn designed_future_surfaces() -> &'static [RuntimeSurface] {
    &[
        RuntimeSurface::DesignedRpc,
        RuntimeSurface::FutureEmbeddedCoreApi,
    ]
}

/// Output format for CLI/runtime calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitMode {
    /// Human-readable one-line status output.
    Human,
    /// Canonical JSONL event stream output.
    Jsonl,
}

/// Result of a run, replay, tail or resume operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutput {
    /// Number of events represented by this output.
    pub event_count: usize,
    /// Whether the represented session ended or is known to be failed.
    pub failed: bool,
    /// Session id.
    pub session_id: String,
    /// Path to the persisted session log.
    pub session_path: PathBuf,
    /// Captured stdout for non-streaming callers.
    pub stdout: String,
}

/// Tail behavior options.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TailOptions {
    /// Whether to wait for appended events until a terminal event or timeout.
    pub follow: bool,
    /// Optional maximum follow duration.
    pub timeout: Option<Duration>,
}

impl TailOptions {
    /// Follows until the session reaches a terminal event.
    pub fn follow() -> Self {
        Self {
            follow: true,
            timeout: None,
        }
    }

    /// Reads the current complete prefix and exits immediately.
    pub fn no_follow() -> Self {
        Self {
            follow: false,
            timeout: None,
        }
    }
}

/// Error returned by Loop Agent runtime operations.
#[derive(Debug)]
pub enum RuntimeError {
    /// Filesystem I/O failed.
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// JSON parsing or serialization failed.
    Json(serde_json::Error),
    /// Policy compilation failed.
    Policy(core_policy::PolicyCompileError),
    /// Registry loading or validation failed.
    Registry(core_script::RegistryError),
    /// Runtime enforcement denied a side effect.
    Denied {
        /// Structured denial reason.
        reason: core_policy::DenyReasonCode,
        /// Human-readable denial message.
        message: String,
    },
    /// Runtime protocol invariant was violated.
    Protocol(String),
    /// Mandatory provider context exceeded the selected model profile's input budget.
    ContextBudgetExceeded {
        /// Available input budget under the selected model profile.
        input_budget: usize,
        /// Canonical mandatory context bytes required by the turn.
        required_bytes: usize,
    },
    /// The per-session event writer failed after event construction.
    EventWriter(Box<RuntimeError>),
    /// A session lock already exists for the requested session.
    ActiveSession {
        /// Requested session id.
        session_id: String,
        /// Existing lock path.
        lock_path: PathBuf,
    },
    /// A requested new session log already exists.
    SessionLogExists(String),
    /// Resume was requested for a terminal session.
    TerminalSession(String),
    /// CLI/user input was invalid.
    Usage(String),
}

impl RuntimeError {
    /// Returns the process exit code associated with this runtime error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Denied { .. }
            | Self::Protocol(_)
            | Self::ContextBudgetExceeded { .. }
            | Self::EventWriter(_)
            | Self::ActiveSession { .. }
            | Self::SessionLogExists(_)
            | Self::TerminalSession(_) => 65,
            Self::Usage(_) => 64,
            Self::Io { .. } | Self::Json(_) | Self::Policy(_) | Self::Registry(_) => 65,
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Json(err) => write!(f, "{err}"),
            Self::Policy(err) => write!(f, "{err}"),
            Self::Registry(err) => write!(f, "{err}"),
            Self::Denied { message, .. } => f.write_str(message),
            Self::Protocol(message) | Self::Usage(message) => f.write_str(message),
            Self::ContextBudgetExceeded {
                input_budget,
                required_bytes,
            } => write!(
                f,
                "context_budget_exceeded: mandatory context requires {required_bytes} estimated tokens, input budget is {input_budget}"
            ),
            Self::EventWriter(source) => write!(f, "event writer: {source}"),
            Self::ActiveSession {
                session_id,
                lock_path,
            } => f.write_str(&active_session_lock_message(lock_path, session_id)),
            Self::SessionLogExists(session_id) => {
                write!(f, "session log already exists for {session_id}")
            }
            Self::TerminalSession(session_id) => {
                write!(f, "cannot resume terminal session {session_id}")
            }
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(err) => Some(err),
            Self::Policy(err) => Some(err),
            Self::Registry(err) => Some(err),
            Self::Denied { .. }
            | Self::Protocol(_)
            | Self::ContextBudgetExceeded { .. }
            | Self::ActiveSession { .. }
            | Self::SessionLogExists(_)
            | Self::TerminalSession(_)
            | Self::Usage(_) => None,
            Self::EventWriter(source) => Some(source.as_ref()),
        }
    }
}

impl From<core_script::RegistryError> for RuntimeError {
    fn from(err: core_script::RegistryError) -> Self {
        Self::Registry(err)
    }
}

impl From<core_policy::PolicyCompileError> for RuntimeError {
    fn from(err: core_policy::PolicyCompileError) -> Self {
        Self::Policy(err)
    }
}

impl From<serde_json::Error> for RuntimeError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

/// Returns whether `session_id` is a valid path-safe v0 session id.
pub fn validate_session_id(session_id: &str) -> bool {
    proto::is_valid_session_id(session_id)
}

/// Returns the current runtime enforcement notice used by docs/tests.
pub fn m0_runtime_notice() -> &'static str {
    "M1 runs deterministic in-process Loop Agent execution; OS sandbox enforcement is post-M1"
}
