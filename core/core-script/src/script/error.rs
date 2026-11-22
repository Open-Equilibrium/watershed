use crate::script::model::ToolKind;
use std::{fmt, path::PathBuf};

#[derive(Debug)]
pub enum RegistryError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    UnsafePath {
        path: PathBuf,
        message: String,
    },
    ReadLimitExceeded {
        path: PathBuf,
        bytes: u64,
        max: u64,
    },
    TraversalLimitExceeded {
        path: PathBuf,
        limit: &'static str,
        observed: usize,
        max: usize,
    },
    InvalidBlockId(String),
    InvalidBlockName {
        kind: &'static str,
        id: String,
    },
    NonNfcDefinitionText {
        kind: &'static str,
        id: String,
        field: String,
    },
    Parse {
        source_name: String,
        message: String,
    },
    DuplicateId {
        kind: &'static str,
        id: String,
    },
    DuplicateName {
        kind: &'static str,
        name: String,
    },
    AmbiguousReference {
        kind: &'static str,
        reference: String,
    },
    MissingReference {
        from_kind: &'static str,
        from_id: String,
        reference_kind: &'static str,
        reference: String,
    },
    InvalidTransition {
        owner_kind: &'static str,
        owner_id: String,
        from_phase_id: String,
        to_phase_id: String,
    },
    PhaseCycle {
        phase_id: String,
    },
    PhaseDepthExceeded {
        phase_id: String,
        depth: usize,
        max: usize,
    },
    PhaseFanoutExceeded {
        phase_id: String,
        count: usize,
        max: usize,
    },
    FlowCycle {
        flow_id: String,
    },
    FlowDepthExceeded {
        flow_id: String,
        depth: usize,
        max: usize,
    },
    SubflowFanoutExceeded {
        phase_id: String,
        count: usize,
        max: usize,
    },
    Semantic(SemanticValidationError),
    CanonicalJson(proto::CanonicalJsonError),
    Serialize(serde_json::Error),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::UnsafePath { path, message } => write!(f, "{}: {message}", path.display()),
            Self::ReadLimitExceeded { path, bytes, max } => write!(
                f,
                "{}: registry read size {bytes} bytes exceeds max {max}",
                path.display()
            ),
            Self::TraversalLimitExceeded {
                path,
                limit,
                observed,
                max,
            } => write!(
                f,
                "{}: registry traversal {limit} {observed} exceeds max {max}",
                path.display()
            ),
            Self::InvalidBlockId(value) => write!(f, "invalid block id: {value}"),
            Self::InvalidBlockName { kind, id } => {
                write!(f, "{kind} {id} name must be non-empty")
            }
            Self::NonNfcDefinitionText { kind, id, field } => {
                write!(f, "{kind} {id} {field} must use NFC text")
            }
            Self::Parse {
                source_name,
                message,
            } => write!(f, "{source_name}: {message}"),
            Self::DuplicateId { kind, id } => write!(f, "duplicate {kind} id: {id}"),
            Self::DuplicateName { kind, name } => write!(f, "duplicate {kind} name: {name}"),
            Self::AmbiguousReference { kind, reference } => write!(
                f,
                "ambiguous {kind} reference {reference} matches both an id and a name"
            ),
            Self::MissingReference {
                from_kind,
                from_id,
                reference_kind,
                reference,
            } => write!(
                f,
                "{from_kind} {from_id} references missing {reference_kind} {reference}"
            ),
            Self::InvalidTransition {
                owner_kind,
                owner_id,
                from_phase_id,
                to_phase_id,
            } => write!(
                f,
                "{owner_kind} {owner_id} transition must move forward between direct child phases: {from_phase_id} -> {to_phase_id}"
            ),
            Self::PhaseCycle { phase_id } => write!(f, "phase cycle includes {phase_id}"),
            Self::PhaseDepthExceeded {
                phase_id,
                depth,
                max,
            } => write!(
                f,
                "phase nesting depth {depth} for {phase_id} exceeds max {max}"
            ),
            Self::PhaseFanoutExceeded {
                phase_id,
                count,
                max,
            } => write!(
                f,
                "phase child fan-out {count} for {phase_id} exceeds max {max}"
            ),
            Self::FlowCycle { flow_id } => write!(f, "flow cycle includes {flow_id}"),
            Self::FlowDepthExceeded {
                flow_id,
                depth,
                max,
            } => write!(
                f,
                "flow nesting depth {depth} for {flow_id} exceeds max {max}"
            ),
            Self::SubflowFanoutExceeded {
                phase_id,
                count,
                max,
            } => write!(
                f,
                "phase subflow fan-out {count} for {phase_id} exceeds max {max}"
            ),
            Self::Semantic(err) => write!(f, "{err}"),
            Self::CanonicalJson(err) => {
                write!(f, "failed to serialize canonical registry JSON: {err}")
            }
            Self::Serialize(err) => write!(f, "failed to serialize resolved registry: {err}"),
        }
    }
}

impl std::error::Error for RegistryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::CanonicalJson(err) => Some(err),
            Self::Semantic(err) => Some(err),
            Self::Serialize(err) => Some(err),
            Self::UnsafePath { .. }
            | Self::ReadLimitExceeded { .. }
            | Self::TraversalLimitExceeded { .. }
            | Self::InvalidBlockId(_)
            | Self::InvalidBlockName { .. }
            | Self::NonNfcDefinitionText { .. }
            | Self::Parse { .. }
            | Self::DuplicateId { .. }
            | Self::DuplicateName { .. }
            | Self::AmbiguousReference { .. }
            | Self::MissingReference { .. }
            | Self::InvalidTransition { .. }
            | Self::PhaseCycle { .. }
            | Self::PhaseDepthExceeded { .. }
            | Self::PhaseFanoutExceeded { .. }
            | Self::FlowCycle { .. }
            | Self::FlowDepthExceeded { .. }
            | Self::SubflowFanoutExceeded { .. } => None,
        }
    }
}

impl From<SemanticValidationError> for RegistryError {
    fn from(err: SemanticValidationError) -> Self {
        Self::Semantic(err)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticValidationError {
    ToolCommandKindMismatch {
        tool_id: String,
        tool_kind: ToolKind,
    },
    InvalidToolDefinition {
        tool_id: String,
        message: String,
    },
    OwnScriptCommandIdMismatch {
        command: String,
        tool_id: String,
    },
    InvalidInstructionDefinition {
        instruction_id: String,
        message: String,
    },
    InvalidPhaseDefinition {
        phase_id: String,
        message: String,
    },
    InvalidFlowDefinition {
        flow_id: String,
        message: String,
    },
}

impl fmt::Display for SemanticValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolCommandKindMismatch { tool_id, tool_kind } => {
                let tool_kind = tool_kind.as_str();
                write!(
                    f,
                    "tool command shape does not match {tool_kind}: {tool_id}"
                )
            }
            Self::InvalidToolDefinition { tool_id, message } => {
                write!(f, "invalid tool definition {tool_id}: {message}")
            }
            Self::OwnScriptCommandIdMismatch { command, tool_id } => write!(
                f,
                "own-script command must be script:<tool-id>: {tool_id} used {command}"
            ),
            Self::InvalidInstructionDefinition {
                instruction_id,
                message,
            } => write!(
                f,
                "invalid instruction definition {instruction_id}: {message}"
            ),
            Self::InvalidPhaseDefinition { phase_id, message } => {
                write!(f, "invalid phase definition {phase_id}: {message}")
            }
            Self::InvalidFlowDefinition { flow_id, message } => {
                write!(f, "invalid flow definition {flow_id}: {message}")
            }
        }
    }
}

impl std::error::Error for SemanticValidationError {}
