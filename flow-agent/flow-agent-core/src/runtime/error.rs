use std::{fmt, io, path::PathBuf};

pub(crate) const MAX_PROVIDER_ERROR_MESSAGE_CHARS: usize = 4_000;
pub(crate) const PROVIDER_ERROR_REASON: &str = "provider_error";

/// A bounded provider failure reported without rewriting the provider's message.
#[derive(Debug)]
pub struct ProviderFailure {
    http_status: Option<u16>,
    message: String,
    definitive: bool,
}

impl ProviderFailure {
    pub(crate) fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn is_definitive(&self) -> bool {
        self.definitive
    }
}

/// Error returned by Flow Agent runtime operations.
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
    /// Persisted workspace state prevents the requested operation.
    PersistedState(String),
    /// Global Flow configuration initialization refused an existing owned output.
    GlobalConfigAlreadyInitialized {
        /// Existing authoring path that prevents initialization.
        path: PathBuf,
    },
    /// Definition publication refused an existing target.
    DefinitionExists {
        /// Registry definition kind.
        definition_kind: &'static str,
        /// Registry definition id.
        definition_id: String,
        /// Existing definition path.
        path: PathBuf,
    },
    /// A registry definition was rejected during authoring.
    InvalidDefinition {
        /// Registry definition kind, when known.
        definition_kind: Option<&'static str>,
        /// Registry definition id, when known.
        definition_id: Option<String>,
        /// Safe registry path, when known.
        path: Option<PathBuf>,
        /// Detailed validation failure.
        source: Box<RuntimeError>,
    },
    /// An authoring validation reference could not be resolved.
    InvalidReference {
        /// Registry reference kind.
        reference_kind: &'static str,
        /// Rejected reference.
        reference: String,
        /// Registry root searched for the reference.
        path: PathBuf,
        /// Detailed resolution failure.
        source: Box<RuntimeError>,
    },
    /// Mandatory provider context exceeded the selected model profile's input budget.
    ContextBudgetExceeded {
        /// Available input-token budget under the selected model profile.
        input_budget_tokens: usize,
        /// Canonical mandatory context bytes required by the turn.
        required_bytes: usize,
    },
    /// In-memory replay output exceeded its fixed byte limit.
    ReplayOutputLimitExceeded {
        /// Maximum canonical JSONL bytes accepted by the in-memory replay API.
        limit_bytes: usize,
    },
    /// M1 has no productive provider or external-tool execution backend.
    ExecutionBackendUnavailable,
    /// Productive execution has no supported backend on this platform.
    ProductiveExecutionUnavailable,
    /// A provider rejected or could not conclusively finish an attempted request.
    Provider(ProviderFailure),
    /// The active productive CLI operation was cancelled by the user.
    Cancelled,
    /// The per-session event writer failed after event construction.
    EventWriter(Box<RuntimeError>),
    /// Multiple independent event-writer failures occurred.
    EventWriterFailures(Vec<Box<RuntimeError>>),
    /// A temporary replacement operation and its cleanup both failed.
    TemporaryReplacementFailures {
        /// Failure from writing or publishing the replacement.
        operation: Box<RuntimeError>,
        /// Failure from removing the temporary replacement path.
        cleanup: Box<RuntimeError>,
    },
    /// An own-script output was published, but its temporary hard link remains.
    PublishedOutputCleanupFailure {
        /// Committed output path.
        output: PathBuf,
        /// Residual temporary hard-link path.
        temporary: PathBuf,
        /// Final temporary-link cleanup failure.
        source: Box<RuntimeError>,
    },
    /// An output was published, but its durable finalization failed.
    PublishedOutputFinalizationFailure {
        /// Committed output path.
        output: PathBuf,
        /// Finalization failure after publication.
        source: Box<RuntimeError>,
    },
    /// A complete credential replacement was published, but its finalization failed.
    PublishedCredentialFinalizationFailure {
        /// Redacted finalization failure after credential publication.
        source: Box<RuntimeError>,
    },
    /// Multiple controlled operation stages failed.
    ControlledStageFailures {
        /// Runtime, validation, or output-generation failure.
        operation: Option<Box<RuntimeError>>,
        /// Event-writer finalization failure.
        finalization: Option<Box<RuntimeError>>,
        /// Session ownership or lock-cleanup failure.
        cleanup: Option<Box<RuntimeError>>,
    },
    /// Multiple reservation-cleanup operations failed.
    SessionCleanupFailures(Vec<Box<RuntimeError>>),
    /// A persisted session failed during runtime execution.
    SessionFailed {
        /// Identifier of the authoritative failed session.
        session_id: String,
        /// Typed runtime cause recorded by the session.
        source: Box<RuntimeError>,
    },
    /// A host-local ownership lease is already held for the requested session.
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
    /// Productive execution requires interactive authentication.
    AuthenticationRequired(String),
}

impl RuntimeError {
    /// Returns the process exit code associated with this runtime error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 64,
            Self::SessionFailed { source, .. } => source.exit_code(),
            _ => 65,
        }
    }

    pub(crate) fn session_failed(session_id: &str, source: Self) -> Self {
        Self::SessionFailed {
            session_id: session_id.to_owned(),
            source: Box::new(source),
        }
    }

    pub(crate) fn denied(reason: core_policy::DenyReasonCode, message: String) -> Self {
        Self::Denied { reason, message }
    }

    pub(crate) fn protocol_or_denied(
        denied_reason: Option<core_policy::DenyReasonCode>,
        message: String,
    ) -> Self {
        match denied_reason {
            Some(reason) => Self::denied(reason, message),
            None => Self::Protocol(message),
        }
    }

    pub(crate) fn definitive_provider_error(
        http_status: Option<u16>,
        message: impl Into<String>,
    ) -> Self {
        Self::Provider(ProviderFailure {
            http_status,
            message: bounded_provider_message(message.into()),
            definitive: true,
        })
    }

    pub(crate) fn uncertain_provider_error(message: impl Into<String>) -> Self {
        Self::Provider(ProviderFailure {
            http_status: None,
            message: bounded_provider_message(message.into()),
            definitive: false,
        })
    }

    pub(crate) fn provider_failure(&self) -> Option<&ProviderFailure> {
        match self {
            Self::Provider(failure) => Some(failure),
            _ => None,
        }
    }

    fn active_session_message(lock_path: &std::path::Path, session_id: &str) -> String {
        format!(
            "session {session_id} is already active under a host-local ownership lease; {} is its non-authoritative workspace marker. Retry after the owning Flow Agent process exits.",
            lock_path.display()
        )
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
            Self::Protocol(message)
            | Self::PersistedState(message)
            | Self::Usage(message)
            | Self::AuthenticationRequired(message) => f.write_str(message),
            Self::GlobalConfigAlreadyInitialized { path } => write!(
                f,
                "global_config_already_initialized: path={}",
                path.display()
            ),
            Self::DefinitionExists {
                definition_kind,
                definition_id,
                path,
            } => write!(
                f,
                "definition_exists: kind={definition_kind} id={definition_id} path={}",
                path.display()
            ),
            Self::InvalidDefinition {
                definition_kind,
                definition_id,
                path,
                source,
            } => {
                f.write_str("invalid_definition")?;
                if let Some(definition_kind) = definition_kind {
                    write!(f, ": kind={definition_kind}")?;
                }
                if let Some(definition_id) = definition_id {
                    write!(f, " id={definition_id}")?;
                }
                if let Some(path) = path {
                    write!(f, " path={}", path.display())?;
                }
                write!(f, ": {source}")
            }
            Self::InvalidReference {
                reference_kind,
                reference,
                path,
                source,
            } => write!(
                f,
                "invalid_reference: kind={reference_kind} id={reference} path={}: {source}",
                path.display()
            ),
            Self::ContextBudgetExceeded {
                input_budget_tokens,
                required_bytes,
            } => write!(
                f,
                "context_budget_exceeded: mandatory context is {required_bytes} canonical bytes (one estimated token per byte), input budget is {input_budget_tokens} tokens"
            ),
            Self::ReplayOutputLimitExceeded { limit_bytes } => write!(
                f,
                "replay_output_limit_exceeded: in-memory replay output exceeds {limit_bytes} bytes"
            ),
            Self::ExecutionBackendUnavailable => f.write_str(
                "execution_backend_unavailable: M1 requires the explicit stub-model fixture profile",
            ),
            Self::ProductiveExecutionUnavailable => f.write_str(
                "execution_backend_unavailable: productive execution is unavailable on this platform",
            ),
            Self::Provider(failure) => {
                f.write_str(PROVIDER_ERROR_REASON)?;
                if let Some(status) = failure.http_status {
                    write!(f, " (HTTP {status})")?;
                }
                if !failure.message.is_empty() {
                    write!(f, ": {}", failure.message)?;
                }
                Ok(())
            }
            Self::Cancelled => f.write_str("productive execution cancelled"),
            Self::EventWriter(source) => write!(f, "event writer: {source}"),
            Self::EventWriterFailures(failures) => {
                f.write_str("event writer failures")?;
                for (index, error) in failures.iter().enumerate() {
                    write!(f, "; failure {}: {error}", index + 1)?;
                }
                Ok(())
            }
            Self::TemporaryReplacementFailures { operation, cleanup } => write!(
                f,
                "temporary replacement operation failed: {operation}; temporary replacement cleanup failed: {cleanup}"
            ),
            Self::PublishedOutputCleanupFailure {
                output,
                temporary,
                source,
            } => write!(
                f,
                "own-script output {} was published, but temporary path {} cleanup failed: {source}",
                output.display(),
                temporary.display()
            ),
            Self::PublishedOutputFinalizationFailure { output, source } => write!(
                f,
                "output {} was published, but finalization failed: {source}",
                output.display()
            ),
            Self::PublishedCredentialFinalizationFailure { .. } => f.write_str(
                "credential_published_not_finalized: replacement credential was published but finalization failed",
            ),
            Self::ControlledStageFailures {
                operation,
                finalization,
                cleanup,
            } => {
                let mut separator = "";
                if let Some(error) = operation {
                    write!(f, "operation failed: {error}")?;
                    separator = "; ";
                }
                if let Some(error) = finalization {
                    write!(f, "{separator}event writer finalization failed: {error}")?;
                    separator = "; ";
                }
                if let Some(error) = cleanup {
                    write!(f, "{separator}ownership cleanup failed: {error}")?;
                }
                Ok(())
            }
            Self::SessionCleanupFailures(failures) => {
                f.write_str("session reservation cleanup failed")?;
                for (index, error) in failures.iter().enumerate() {
                    write!(f, "; cleanup failure {}: {error}", index + 1)?;
                }
                Ok(())
            }
            Self::SessionFailed { session_id, source } => {
                write!(f, "session {session_id} failed: {source}")
            }
            Self::ActiveSession {
                session_id,
                lock_path,
            } => f.write_str(&Self::active_session_message(lock_path, session_id)),
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
            | Self::PersistedState(_)
            | Self::ContextBudgetExceeded { .. }
            | Self::ReplayOutputLimitExceeded { .. }
            | Self::ExecutionBackendUnavailable
            | Self::ProductiveExecutionUnavailable
            | Self::Provider(_)
            | Self::Cancelled
            | Self::GlobalConfigAlreadyInitialized { .. }
            | Self::DefinitionExists { .. }
            | Self::ActiveSession { .. }
            | Self::SessionLogExists(_)
            | Self::TerminalSession(_)
            | Self::Usage(_)
            | Self::AuthenticationRequired(_) => None,
            Self::EventWriter(source)
            | Self::InvalidDefinition { source, .. }
            | Self::InvalidReference { source, .. }
            | Self::SessionFailed { source, .. }
            | Self::PublishedOutputCleanupFailure { source, .. }
            | Self::PublishedOutputFinalizationFailure { source, .. }
            | Self::PublishedCredentialFinalizationFailure { source }
            | Self::TemporaryReplacementFailures {
                operation: source, ..
            } => Some(source.as_ref()),
            Self::EventWriterFailures(failures) => failures
                .first()
                .map(|source| source.as_ref() as &(dyn std::error::Error + 'static)),
            Self::ControlledStageFailures {
                operation,
                finalization,
                cleanup,
            } => operation
                .as_deref()
                .or(finalization.as_deref())
                .or(cleanup.as_deref())
                .map(|source| source as &(dyn std::error::Error + 'static)),
            Self::SessionCleanupFailures(failures) => failures
                .first()
                .map(|source| source.as_ref() as &(dyn std::error::Error + 'static)),
        }
    }
}

fn bounded_provider_message(message: String) -> String {
    message
        .chars()
        .take(MAX_PROVIDER_ERROR_MESSAGE_CHARS)
        .collect()
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
