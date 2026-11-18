use std::{fmt, io, path::PathBuf};

pub(crate) const MAX_PROVIDER_ERROR_MESSAGE_CHARS: usize = 4_000;
pub(crate) const PROVIDER_ERROR_REASON: &str = "provider_error";
pub(crate) const PROVIDER_REPLY_INVALID_REASON: &str = "provider_reply_invalid";
pub(crate) const PROCESS_CLEANUP_UNKNOWN_REASON: &str = "process_cleanup_unknown";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutorFailure {
    code: proto::ExecutorErrorCodeV0,
    message: String,
}

impl ExecutorFailure {
    pub(crate) fn new(code: proto::ExecutorErrorCodeV0, message: impl Into<String>) -> Self {
        let message = message
            .into()
            .chars()
            .take(MAX_PROVIDER_ERROR_MESSAGE_CHARS)
            .collect();
        Self { code, message }
    }

    pub fn code(&self) -> proto::ExecutorErrorCodeV0 {
        self.code
    }
}

#[derive(Debug)]
pub struct ProviderFailure {
    http_status: Option<u16>,
    kind: ProviderFailureKind,
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderFailureKind {
    Definitive,
    Uncertain,
    ReplyInvalid,
}

impl ProviderFailure {
    pub(crate) fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn kind(&self) -> ProviderFailureKind {
        self.kind
    }

    pub(crate) fn is_definitive(&self) -> bool {
        self.kind == ProviderFailureKind::Definitive
    }

    pub(crate) fn is_reply_invalid(&self) -> bool {
        self.kind == ProviderFailureKind::ReplyInvalid
    }

    pub(crate) fn is_known_outcome(&self) -> bool {
        self.kind != ProviderFailureKind::Uncertain
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Json(serde_json::Error),
    Policy(core_policy::PolicyCompileError),
    Registry(core_script::RegistryError),
    Denied {
        reason: core_policy::DenyReasonCode,
        message: String,
    },
    Protocol(String),
    PersistedState(String),
    GlobalConfigAlreadyInitialized {
        path: PathBuf,
    },
    DefinitionExists {
        definition_kind: &'static str,
        definition_id: String,
        path: PathBuf,
    },
    InvalidDefinition {
        definition_kind: Option<&'static str>,
        definition_id: Option<String>,
        path: Option<PathBuf>,
        source: Box<RuntimeError>,
    },
    InvalidReference {
        reference_kind: &'static str,
        reference: String,
        path: PathBuf,
        source: Box<RuntimeError>,
    },
    ContextBudgetExceeded {
        input_budget_tokens: usize,
        required_bytes: usize,
    },
    ReplayOutputLimitExceeded {
        limit_bytes: usize,
    },
    ExecutionBackendUnavailable,
    ProductiveExecutionUnavailable,
    ProcessCleanupUnknown,
    Provider(ProviderFailure),
    Executor(ExecutorFailure),
    Cancelled,
    EventWriter(Box<RuntimeError>),
    EventWriterFailures(Vec<Box<RuntimeError>>),
    TemporaryReplacementFailures {
        operation: Box<RuntimeError>,
        cleanup: Box<RuntimeError>,
    },
    PublishedOutputCleanupFailure {
        output: PathBuf,
        temporary: PathBuf,
        source: Box<RuntimeError>,
    },
    PublishedOutputFinalizationFailure {
        output: PathBuf,
        source: Box<RuntimeError>,
    },
    PublishedCredentialFinalizationFailure {
        source: Box<RuntimeError>,
    },
    ControlledStageFailures {
        operation: Option<Box<RuntimeError>>,
        finalization: Option<Box<RuntimeError>>,
        cleanup: Option<Box<RuntimeError>>,
    },
    SessionCleanupFailures(Vec<Box<RuntimeError>>),
    SessionFailed {
        session_id: String,
        source: Box<RuntimeError>,
    },
    ActiveSession {
        session_id: String,
        lock_path: PathBuf,
    },
    SessionLogExists(String),
    TerminalSession(String),
    Usage(String),
    AuthenticationRequired(String),
}

impl RuntimeError {
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
            kind: ProviderFailureKind::Definitive,
            message: bounded_provider_message(message.into()),
        })
    }

    pub(crate) fn uncertain_provider_error(message: impl Into<String>) -> Self {
        Self::Provider(ProviderFailure {
            http_status: None,
            kind: ProviderFailureKind::Uncertain,
            message: bounded_provider_message(message.into()),
        })
    }

    pub(crate) fn provider_reply_invalid(message: impl Into<String>) -> Self {
        Self::Provider(ProviderFailure {
            http_status: None,
            kind: ProviderFailureKind::ReplyInvalid,
            message: bounded_provider_message(message.into()),
        })
    }

    pub(crate) fn provider_failure(&self) -> Option<&ProviderFailure> {
        match self {
            Self::Provider(failure) => Some(failure),
            _ => None,
        }
    }

    pub(crate) fn without_secret(self, secret: &str) -> Self {
        match self {
            Self::Provider(failure) => {
                let cut = failure.message.chars().count() >= MAX_PROVIDER_ERROR_MESSAGE_CHARS;
                let mut message = failure.message.replace(secret, "<redacted>");
                if let Some(end) = (1..secret.len()).rev().find(|end| {
                    cut && secret.is_char_boundary(*end) && message.ends_with(&secret[..*end])
                }) {
                    message.truncate(message.len() - end);
                }
                Self::Provider(ProviderFailure {
                    message: bounded_provider_message(message),
                    ..failure
                })
            }
            other => other,
        }
    }

    pub(crate) fn executor(code: proto::ExecutorErrorCodeV0, message: impl Into<String>) -> Self {
        Self::Executor(ExecutorFailure::new(code, message))
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
                "execution_backend_unavailable: Fixture resume requires the explicit stub-model fixture profile",
            ),
            Self::ProductiveExecutionUnavailable => f.write_str(
                "execution_backend_unavailable: productive execution is unavailable on this platform",
            ),
            Self::ProcessCleanupUnknown => f.write_str(PROCESS_CLEANUP_UNKNOWN_REASON),
            Self::Provider(failure) => {
                f.write_str(if failure.is_reply_invalid() {
                    PROVIDER_REPLY_INVALID_REASON
                } else {
                    PROVIDER_ERROR_REASON
                })?;
                if let Some(status) = failure.http_status {
                    write!(f, " (HTTP {status})")?;
                }
                if !failure.message.is_empty() {
                    write!(f, ": {}", failure.message)?;
                }
                Ok(())
            }
            Self::Executor(failure) => {
                let code = serde_json::to_value(failure.code)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "executor_invalid_response".to_owned());
                write!(f, "{code}")?;
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
            | Self::ProcessCleanupUnknown
            | Self::Provider(_)
            | Self::Executor(_)
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
