mod artifact;
mod compile;

pub use artifact::{
    AllowedParameterPolicy, CommandPolicy, DenyReasonCode, EnvironmentDefault, EnvironmentPolicy,
    PhaseScope, PolicyArtifact, PolicyArtifactError, PolicyArtifactValidationError, RuntimeLimits,
    canonical_artifact_json,
};
pub use compile::{PolicyCompileError, compile_policy_artifact};
pub use core_script::{ParameterValueType, ScriptRuntime, ToolKind};

pub const POLICY_VERSION_V0: &str = "0";
pub(crate) const OWN_SCRIPT_RUNNER_POSIX_SH: &str = "runner:posix-sh";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustedPredefinedCommand {
    Echo,
    Negative,
    Read,
}

impl TrustedPredefinedCommand {
    pub const ALL: [Self; 3] = [Self::Echo, Self::Negative, Self::Read];

    pub fn parse(command_id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| command.as_str() == command_id)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Echo => "agent-echo",
            Self::Negative => "agent-negative",
            Self::Read => "agent-read",
        }
    }

    pub const fn productive_executable(self) -> Option<&'static str> {
        match self {
            Self::Echo => Some("/bin/echo"),
            Self::Negative => None,
            Self::Read => Some("/bin/cat"),
        }
    }

    pub fn executable(self) -> String {
        format!("registry:{}", self.as_str())
    }
}

#[cfg(test)]
mod tests;
