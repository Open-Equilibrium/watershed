//! Policy artifact contracts.

#![deny(missing_docs)]

mod artifact;
mod compile;

pub use artifact::{
    AllowedParameterPolicy, CommandPolicy, DenyReasonCode, EnvironmentDefault, EnvironmentPolicy,
    FilesystemPolicy, NetworkPolicy, PhaseScope, PolicyArtifact, PolicyArtifactError,
    PolicyArtifactValidationError, PolicyTarget, RuntimeLimits, canonical_artifact_json,
};
pub use compile::{PolicyCompileError, compile_policy_artifact};
pub use core_script::{
    MAX_FILESYSTEM_MOUNTS, NetworkAllowEntry, NetworkAllowKind, NetworkDefault, NetworkTransport,
    ParameterValueType, ScriptRuntime, ToolKind, ToolRuntimeProfile,
};

/// Policy artifact version string emitted by the v0 compiler.
pub const POLICY_VERSION_V0: &str = "0";
/// Productive executable used for POSIX own-script Tools.
pub const OWN_SCRIPT_PRODUCTIVE_EXECUTABLE: &str = proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0;
pub(crate) const OWN_SCRIPT_RUNNER_POSIX_SH: &str = "runner:posix-sh";

/// A predefined command implemented and trusted by Flow Agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustedPredefinedCommand {
    /// Echoes its arguments.
    Echo,
    /// Exercises deterministic sandbox denials in fixtures.
    Negative,
    /// Reads one workspace file.
    Read,
}

impl TrustedPredefinedCommand {
    /// All trusted predefined commands.
    pub const ALL: [Self; 3] = [Self::Echo, Self::Negative, Self::Read];

    /// Parses a stable predefined command id.
    pub fn parse(command_id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| command.as_str() == command_id)
    }

    /// Returns the stable predefined command id.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Echo => "agent-echo",
            Self::Negative => "agent-negative",
            Self::Read => "agent-read",
        }
    }

    /// Returns the executable used by productive runtime commands.
    pub const fn productive_executable(self) -> Option<&'static str> {
        match self {
            Self::Echo => Some("/bin/echo"),
            Self::Negative => None,
            Self::Read => Some("/bin/cat"),
        }
    }

    /// Returns the executable identity used by runtime policy.
    pub fn executable(self) -> String {
        format!("registry:{}", self.as_str())
    }
}

#[cfg(test)]
mod tests;
