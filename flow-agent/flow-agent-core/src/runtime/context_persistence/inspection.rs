use crate::runtime::{
    context::ContextManifestSourceRecord,
    digest::sha256_hex,
    fs_guards::{AnchoredDir, read_anchored_file_with_limit},
    session_bundle::{
        SessionBundlePaths, ensure_session_object_count, ensure_session_object_total,
    },
    types::{MAX_SESSION_OBJECT_BYTES, RuntimeError},
};
use std::collections::BTreeSet;

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
