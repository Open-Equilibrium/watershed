use crate::script::model::ToolKind;
use std::{fmt, path::PathBuf};

/// Error returned while loading, resolving or serializing a registry.
#[derive(Debug)]
pub enum RegistryError {
    /// Filesystem read failed.
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Registry path failed safety checks.
    UnsafePath {
        /// Rejected path.
        path: PathBuf,
        /// Rejection reason.
        message: String,
    },
    /// Registry read exceeded an M1 byte cap.
    ReadLimitExceeded {
        /// Path whose read budget was exceeded.
        path: PathBuf,
        /// Observed byte count.
        bytes: u64,
        /// Maximum allowed byte count.
        max: u64,
    },
    /// Registry traversal exceeded a non-byte work bound.
    TraversalLimitExceeded {
        /// Path where traversal exceeded the bound.
        path: PathBuf,
        /// Name of the traversal limit.
        limit: &'static str,
        /// Observed count or depth.
        observed: usize,
        /// Maximum allowed count or depth.
        max: usize,
    },
    /// Block id was not a valid v0 id token.
    InvalidBlockId(String),
    /// Block name was empty or otherwise invalid.
    InvalidBlockName {
        /// Block kind.
        kind: &'static str,
        /// Block id.
        id: String,
    },
    /// Source text could not be parsed.
    Parse {
        /// Source name used in diagnostics.
        source_name: String,
        /// Parse diagnostic.
        message: String,
    },
    /// A block id was duplicated.
    DuplicateId {
        /// Block kind.
        kind: &'static str,
        /// Duplicated id.
        id: String,
    },
    /// A normalized block name was duplicated.
    DuplicateName {
        /// Block kind.
        kind: &'static str,
        /// Duplicated authored name.
        name: String,
    },
    /// A reference matched both an id and a normalized name.
    AmbiguousReference {
        /// Referenced block kind.
        kind: &'static str,
        /// User-supplied reference.
        reference: String,
    },
    /// A reference could not be resolved.
    MissingReference {
        /// Referencing block kind.
        from_kind: &'static str,
        /// Referencing block id.
        from_id: String,
        /// Referenced block kind.
        reference_kind: &'static str,
        /// User-supplied reference.
        reference: String,
    },
    /// A transition does not connect ordered direct child Phases.
    InvalidTransition {
        /// Transition-owning block kind.
        owner_kind: &'static str,
        /// Transition-owning block id.
        owner_id: String,
        /// Source Phase reference.
        from_phase_id: String,
        /// Destination Phase reference.
        to_phase_id: String,
    },
    /// Recursive Phase references contain a cycle.
    PhaseCycle {
        /// Phase id involved in the cycle.
        phase_id: String,
    },
    /// Recursive Phase references exceed the nesting cap.
    PhaseDepthExceeded {
        /// Phase id where the cap was exceeded.
        phase_id: String,
        /// Observed nesting depth.
        depth: usize,
        /// Maximum allowed nesting depth.
        max: usize,
    },
    /// One Phase declares more direct child Phases than allowed.
    PhaseFanoutExceeded {
        /// Phase whose direct child list exceeded the cap.
        phase_id: String,
        /// Observed direct child count.
        count: usize,
        /// Maximum allowed direct child count.
        max: usize,
    },
    /// Recursive flow references contain a cycle.
    FlowCycle {
        /// Flow id involved in the cycle.
        flow_id: String,
    },
    /// Recursive flow references exceed the nesting cap.
    FlowDepthExceeded {
        /// Flow id where the cap was exceeded.
        flow_id: String,
        /// Observed nesting depth.
        depth: usize,
        /// Maximum allowed nesting depth.
        max: usize,
    },
    /// One Flow declares more direct runtime subflow invocations than allowed.
    FlowFanoutExceeded {
        /// Flow whose direct subflow list exceeded the cap.
        flow_id: String,
        /// Observed direct subflow invocation count.
        count: usize,
        /// Maximum allowed direct subflow invocation count.
        max: usize,
    },
    /// Semantic validation failed.
    Semantic(SemanticValidationError),
    /// Canonical JSON serialization failed.
    CanonicalJson(proto::CanonicalJsonError),
    /// Serde serialization failed before canonicalization.
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
            Self::FlowFanoutExceeded {
                flow_id,
                count,
                max,
            } => write!(
                f,
                "flow subflow fan-out {count} for {flow_id} exceeds max {max}"
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
            | Self::FlowFanoutExceeded { .. } => None,
        }
    }
}

impl From<SemanticValidationError> for RegistryError {
    fn from(err: SemanticValidationError) -> Self {
        Self::Semantic(err)
    }
}

/// Semantic validation failure for a registry block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticValidationError {
    /// The command shape does not match the declared tool kind.
    ToolCommandKindMismatch {
        /// Tool id.
        tool_id: String,
        /// Declared tool kind.
        tool_kind: ToolKind,
    },
    /// A tool definition violates its semantic constraints.
    InvalidToolDefinition {
        /// Tool id.
        tool_id: String,
        /// Rejection reason.
        message: String,
    },
    /// Own-script command was not `script:<tool-id>`.
    OwnScriptCommandIdMismatch {
        /// Declared command string.
        command: String,
        /// Tool id.
        tool_id: String,
    },
    /// Network CIDR entry was not canonical.
    InvalidCanonicalCidr {
        /// Rejected CIDR string.
        cidr: String,
        /// Tool id.
        tool_id: String,
    },
    /// An Instruction definition violates its finite placeholder contract.
    InvalidInstructionDefinition {
        /// Instruction id.
        instruction_id: String,
        /// Rejection reason.
        message: String,
    },
    /// A Phase definition violates its recursive execution contract.
    InvalidPhaseDefinition {
        /// Phase id.
        phase_id: String,
        /// Rejection reason.
        message: String,
    },
    /// A flow definition violates its semantic constraints.
    InvalidFlowDefinition {
        /// Flow id.
        flow_id: String,
        /// Rejection reason.
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
            Self::InvalidCanonicalCidr { cidr, tool_id } => {
                write!(f, "invalid canonical CIDR for tool {tool_id}: {cidr}")
            }
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
