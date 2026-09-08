use proto::{ExecutorErrorCodeV0, ExecutorPreflightV0, ExecutorRequestV0, ExecutorResponseV0};
use std::fmt;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod native;
mod protection;
mod supervision;
#[cfg(test)]
mod tests;

#[cfg(target_os = "linux")]
use linux as platform_backend;
#[cfg(target_os = "macos")]
use macos as platform_backend;

pub(crate) use native::{PreparedExecution, run_inner};
pub(crate) use platform_backend::{BACKEND, PLATFORM};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendError {
    pub(crate) code: ExecutorErrorCodeV0,
    pub(crate) message: String,
    definitive: bool,
}

impl BackendError {
    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::Unavailable,
            message: message.into(),
            definitive: true,
        }
    }

    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::PolicyUnsupported,
            message: message.into(),
            definitive: true,
        }
    }

    pub(crate) fn setup(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::SandboxSetupFailed,
            message: message.into(),
            definitive: true,
        }
    }

    pub(crate) fn uncertain(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::InvalidResponse,
            message: message.into(),
            definitive: false,
        }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(crate) struct ProbeState {
    pub(crate) backend_version: String,
    pub(crate) ready: bool,
    pub(crate) features: Vec<String>,
    pub(crate) readiness_error: Option<String>,
}

pub(crate) fn probe() -> ProbeState {
    match native::readiness() {
        Ok(backend_version) => ProbeState {
            backend_version,
            ready: true,
            features: vec![proto::EXECUTOR_FEATURE_SELF_PROTECTION_V0.to_owned()],
            readiness_error: None,
        },
        Err(error) => ProbeState {
            backend_version: "unavailable".to_owned(),
            ready: false,
            features: Vec::new(),
            readiness_error: Some(error.message),
        },
    }
}

pub(crate) enum Preflight {
    Ready(Box<PreparedExecution>),
    Error(ExecutorPreflightV0),
}

pub(crate) fn preflight(request: ExecutorRequestV0) -> Result<Preflight, String> {
    let request_id = request.request_id.clone();
    match native::preflight(request) {
        Ok(prepared) => Ok(Preflight::Ready(Box::new(prepared))),
        Err(error) if error.definitive => Ok(Preflight::Error(ExecutorPreflightV0::Error {
            schema: proto::EXECUTOR_PREFLIGHT_SCHEMA_V0.to_owned(),
            request_id,
            code: error.code,
            message: error.message,
        })),
        Err(error) => Err(error.message),
    }
}

pub(crate) fn execute(prepared: PreparedExecution) -> Result<ExecutorResponseV0, String> {
    let request_id = prepared.request_id().to_owned();
    match native::execute(prepared) {
        Ok(response) => Ok(response),
        Err(error) if error.definitive => Ok(ExecutorResponseV0::Error {
            schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
            request_id,
            code: error.code,
            message: error.message,
        }),
        Err(error) => Err(error.message),
    }
}
