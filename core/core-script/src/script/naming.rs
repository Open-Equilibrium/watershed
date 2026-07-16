fn insert_named_block<T>(
    kind: &'static str,
    identity: BlockIdentity,
    blocks: &mut BTreeMap<String, T>,
    name_ids: &mut BTreeMap<&'static str, BTreeMap<String, String>>,
    block: T,
) -> Result<(), RegistryError> {
    let names_for_kind = name_ids.entry(kind).or_default();
    if !is_valid_block_id(&identity.id) {
        return Err(RegistryError::InvalidBlockId(identity.id));
    }
    if identity.name.is_empty() {
        return Err(RegistryError::InvalidBlockName {
            kind,
            id: identity.id,
        });
    }
    if blocks.contains_key(&identity.id) {
        return Err(RegistryError::DuplicateId {
            kind,
            id: identity.id,
        });
    }
    if names_for_kind.contains_key(&identity.id) {
        return Err(RegistryError::AmbiguousReference {
            kind,
            reference: identity.id,
        });
    }
    if blocks.contains_key(&identity.name) {
        return Err(RegistryError::AmbiguousReference {
            kind,
            reference: identity.name,
        });
    }
    let normalized_name = normalize_string(&identity.name);
    if names_for_kind.contains_key(&normalized_name) {
        return Err(RegistryError::DuplicateName {
            kind,
            name: identity.name,
        });
    }
    names_for_kind.insert(normalized_name, identity.id.clone());
    blocks.insert(identity.id, block);
    Ok(())
}

fn normalize_string(value: &str) -> String {
    value.nfc().collect()
}

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
    /// Command id was not a valid v0 command token.
    InvalidCommandId(String),
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
    /// Recursive loop references contain a cycle.
    LoopCycle {
        /// Loop id involved in the cycle.
        loop_id: String,
    },
    /// Recursive loop references exceed the nesting cap.
    LoopDepthExceeded {
        /// Loop id where the cap was exceeded.
        loop_id: String,
        /// Observed nesting depth.
        depth: usize,
        /// Maximum allowed nesting depth.
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
            Self::InvalidCommandId(value) => write!(f, "invalid command id: {value}"),
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
            Self::LoopCycle { loop_id } => write!(f, "loop cycle includes {loop_id}"),
            Self::LoopDepthExceeded {
                loop_id,
                depth,
                max,
            } => write!(
                f,
                "loop nesting depth {depth} for {loop_id} exceeds max {max}"
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
            | Self::InvalidCommandId(_)
            | Self::Parse { .. }
            | Self::DuplicateId { .. }
            | Self::DuplicateName { .. }
            | Self::AmbiguousReference { .. }
            | Self::MissingReference { .. }
            | Self::LoopCycle { .. }
            | Self::LoopDepthExceeded { .. } => None,
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
    /// A loop definition violates its semantic constraints.
    InvalidLoopDefinition {
        /// Loop id.
        loop_id: String,
        /// Rejection reason.
        message: String,
    },
}

impl fmt::Display for SemanticValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolCommandKindMismatch { tool_id, tool_kind } => {
                let tool_kind = match tool_kind {
                    ToolKind::PredefinedCommand => "predefined-command",
                    ToolKind::OwnScript => "own-script",
                };
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
            Self::InvalidLoopDefinition { loop_id, message } => {
                write!(f, "invalid loop definition {loop_id}: {message}")
            }
        }
    }
}

impl std::error::Error for SemanticValidationError {}
