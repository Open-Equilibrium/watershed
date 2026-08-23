use super::manifest::validate_context_manifest_line;
use crate::runtime::{
    context::ContextManifestSourceRecord,
    digest::sha256_hex,
    fs_guards::{
        AnchoredDir, ensure_anchored_real_file, for_each_segmented_jsonl_line,
        read_anchored_file_with_limit,
    },
    session_bundle::{
        SessionBundlePaths, ensure_session_object_count, ensure_session_object_total,
    },
    stream_signature::{
        CONTEXT_PLAN_DOMAIN, RuntimeStreamSignature, RuntimeStreamSignatureBuilder,
    },
    types::{CONTEXT_MANIFEST_STREAM_LIMITS, MAX_SESSION_OBJECT_BYTES, RuntimeError},
};
use std::{collections::BTreeSet, io};

pub(crate) fn read_anchored_context_manifest_signature(
    logs: &AnchoredDir,
    sessions: &AnchoredDir,
    session_id: &str,
    completed_turns: usize,
) -> Result<RuntimeStreamSignature, RuntimeError> {
    let path = SessionBundlePaths::contexts_in(logs, session_id);
    match ensure_anchored_real_file(&path) {
        Ok(()) => {}
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest stream is missing",
                path.diagnostic_path().display()
            )));
        }
        Err(error) => return Err(error),
    }
    let mut recorded = RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN);
    let mut line_number = 0usize;
    let mut verified_objects = BTreeSet::new();
    let mut verified_object_bytes = 0u64;
    for_each_segmented_jsonl_line(&path, CONTEXT_MANIFEST_STREAM_LIMITS, |line| {
        line_number = line_number.saturating_add(1);
        if !line.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest stream must end with LF",
                path.diagnostic_path().display()
            )));
        }
        let manifest =
            validate_context_manifest_line(path.diagnostic_path(), line).map_err(|error| {
                let detail = match error {
                    RuntimeError::Protocol(message) => message
                        .strip_prefix(&format!("{} ", path.diagnostic_path().display()))
                        .unwrap_or(&message)
                        .to_owned(),
                    other => other.to_string(),
                };
                RuntimeError::Protocol(format!(
                    "{} line {line_number}: {detail}",
                    path.diagnostic_path().display()
                ))
            })?;
        verify_context_manifest_sources(
            sessions,
            session_id,
            &manifest.ordered_sources,
            &mut verified_objects,
            &mut verified_object_bytes,
        )?;
        recorded.push(line.as_bytes());
        Ok(())
    })?;
    let recoverable_manifest_count = completed_turns.saturating_add(1);
    let recorded = recorded.signature();
    if recorded.record_count < completed_turns || recorded.record_count > recoverable_manifest_count
    {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifests do not match deterministic replay",
            path.diagnostic_path().display()
        )));
    }
    Ok(recorded)
}

fn verify_context_manifest_sources(
    sessions: &AnchoredDir,
    session_id: &str,
    sources: &[ContextManifestSourceRecord],
    verified: &mut BTreeSet<String>,
    verified_bytes: &mut u64,
) -> Result<(), RuntimeError> {
    for source in sources {
        let digest = source.projection_hash.as_str();
        if verified.contains(digest) {
            continue;
        }
        ensure_session_object_count(sessions, verified.len().saturating_add(1))?;
        let path = SessionBundlePaths::object_in(sessions, session_id, digest);
        let bytes =
            read_anchored_file_with_limit(&path, MAX_SESSION_OBJECT_BYTES).map_err(|err| {
                RuntimeError::Protocol(format!(
                    "{} referenced context object is unavailable: {err}",
                    path.diagnostic_path().display()
                ))
            })?;
        let total = verified_bytes
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .unwrap_or(u64::MAX);
        ensure_session_object_total(total)?;
        *verified_bytes = total;
        if sha256_hex(&bytes) != digest {
            return Err(RuntimeError::Protocol(format!(
                "{} referenced context object hash does not match",
                path.diagnostic_path().display()
            )));
        }
        verified.insert(digest.to_owned());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn verify_context_manifest_objects(
    sessions: &AnchoredDir,
    session_id: &str,
    sources: &[ContextManifestSourceRecord],
    verified: &mut BTreeSet<String>,
    verified_bytes: &mut u64,
) -> Result<(), RuntimeError> {
    verify_context_manifest_sources(sessions, session_id, sources, verified, verified_bytes)
}
