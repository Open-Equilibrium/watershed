use crate::runtime::{
    digest::prefixed_sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, ensure_anchored_real_file, path_io_error,
        read_anchored_to_string_with_limit,
    },
    session_bundle::SessionBundlePaths,
    types::{MAX_SESSION_METADATA_BYTES, RuntimeError},
};
use std::{io, path::Path};

pub struct SessionDefinitionMetadata {
    pub(crate) flow_definition_id: String,
    pub(crate) registry_hash: String,
    pub(crate) flow_definition_hash: String,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct SessionLogMetadata {
    pub(crate) flow_definition_id: Option<String>,
    pub(crate) registry_hash: Option<String>,
    pub(crate) flow_definition_hash: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) model_profile_id: Option<String>,
    pub(crate) model_context_limit: Option<usize>,
    pub(crate) output_reserve: Option<usize>,
    pub(crate) safety_margin: Option<usize>,
}

pub fn resumable_flow_id(
    path: &Path,
    session_id: &str,
    event_flow_id: Option<&str>,
    metadata: &SessionLogMetadata,
) -> Result<String, RuntimeError> {
    let recorded_flow_id = metadata.flow_definition_id.as_deref().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing flow_definition_id metadata"
        ))
    })?;
    if event_flow_id.is_some_and(|event_flow_id| event_flow_id != recorded_flow_id) {
        return Err(RuntimeError::Protocol(format!(
            "{} session {session_id} flow definition metadata does not match durable events",
            path.display()
        )));
    }
    Ok(recorded_flow_id.to_owned())
}

pub fn session_definition_metadata(
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
) -> Result<SessionDefinitionMetadata, RuntimeError> {
    let registry_json = registry.canonical_json()?;
    let flow_json = proto::canonical_json(&serde_json::to_value(flow_block)?).map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize flow definition hash: {err}"))
    })?;
    Ok(SessionDefinitionMetadata {
        flow_definition_id: flow_block.identity.id.clone(),
        registry_hash: sha256_hash_text(registry_json.as_bytes()),
        flow_definition_hash: sha256_hash_text(flow_json.as_bytes()),
    })
}

pub fn sha256_hash_text(bytes: &[u8]) -> String {
    prefixed_sha256_hex(bytes)
}

pub fn verify_resume_definition_metadata_values(
    session_id: &str,
    metadata: &SessionLogMetadata,
    registry: &core_script::ResolvedRegistry,
    flow_block: &core_script::FlowBlock,
) -> Result<(), RuntimeError> {
    let Some(recorded_registry_hash) = metadata.registry_hash.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing registry_hash metadata"
        )));
    };
    let Some(recorded_flow_definition_hash) = metadata.flow_definition_hash.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing flow_definition_hash metadata"
        )));
    };
    let Some(recorded_flow_definition_id) = metadata.flow_definition_id.as_deref() else {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: missing flow_definition_id metadata"
        )));
    };

    let expected = session_definition_metadata(registry, flow_block)?;
    if recorded_flow_definition_id != expected.flow_definition_id
        || recorded_registry_hash != expected.registry_hash
        || recorded_flow_definition_hash != expected.flow_definition_hash
    {
        return Err(RuntimeError::Protocol(format!(
            "session {session_id} registry drift: recorded definition metadata does not match current registry"
        )));
    }
    Ok(())
}

pub fn require_anchored_session_log_metadata(
    logs: &AnchoredDir,
    session_id: &str,
) -> Result<SessionLogMetadata, RuntimeError> {
    let path = SessionBundlePaths::metadata_in(logs, session_id);
    if let Some(alias) = ascii_case_alias(&path)? {
        return Err(RuntimeError::Protocol(format!(
            "{} contains non-canonical session metadata name {}",
            logs.path.display(),
            alias.leaf.display()
        )));
    }
    ensure_anchored_real_file(&path)
        .map_err(|error| map_missing_definition_metadata(error, session_id))?;
    parse_session_log_metadata(&read_anchored_to_string_with_limit(
        &path,
        MAX_SESSION_METADATA_BYTES,
    )?)
}

pub fn ascii_case_alias(path: &AnchoredFile) -> Result<Option<AnchoredFile>, RuntimeError> {
    let expected = path
        .leaf
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} must have a UTF-8 filename",
                path.diagnostic_path().display()
            ))
        })?;
    for entry in path
        .parent
        .dir
        .entries()
        .map_err(|source| path_io_error(&path.parent.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&path.parent.path, source))?;
        let name = entry.file_name();
        if name
            .to_str()
            .is_some_and(|name| name != expected && name.eq_ignore_ascii_case(expected))
        {
            return Ok(Some(path.parent.file(name)));
        }
    }
    Ok(None)
}

fn map_missing_definition_metadata(error: RuntimeError, session_id: &str) -> RuntimeError {
    if matches!(
        &error,
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound
    ) {
        missing_definition_metadata(session_id)
    } else {
        error
    }
}

pub fn missing_definition_metadata(session_id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "session {session_id} registry drift: missing definition metadata"
    ))
}

fn parse_session_log_metadata(text: &str) -> Result<SessionLogMetadata, RuntimeError> {
    let mut metadata = SessionLogMetadata::default();
    for (line_number, line) in text.lines().enumerate() {
        let Some((key, value)) = line.split_once('=') else {
            return Err(RuntimeError::Protocol(format!(
                "session metadata line {} is not key=value",
                line_number + 1
            )));
        };
        match key {
            "flow_definition_id" => metadata.flow_definition_id = Some(value.to_owned()),
            "registry_hash" => metadata.registry_hash = Some(value.to_owned()),
            "flow_definition_hash" => metadata.flow_definition_hash = Some(value.to_owned()),
            _ => {}
        }
    }
    Ok(metadata)
}
