use crate::runtime::{
    fs_guards::{AnchoredDir, path_io_error, segmented_jsonl_leaf_stem},
    session_bundle::SessionBundlePaths,
    types::RuntimeError,
};
use std::io;

pub const MAX_UNIQUE_SESSION_CANDIDATES: u32 = 10_000;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum SessionCandidateHint {
    Free,
    Occupied,
    Probe,
}

pub(crate) fn session_candidate_hints_from_dirs(
    sessions: Option<&AnchoredDir>,
    logs: Option<&AnchoredDir>,
    base_session_id: &str,
) -> Result<Vec<SessionCandidateHint>, RuntimeError> {
    let mut hints = vec![SessionCandidateHint::Free; MAX_UNIQUE_SESSION_CANDIDATES as usize];
    for (dir, logs) in [(sessions, false), (logs, true)] {
        let Some(dir) = dir else {
            continue;
        };
        for entry in dir
            .dir
            .entries()
            .map_err(|source| path_io_error(&dir.path, source))?
        {
            let entry = entry.map_err(|source| path_io_error(&dir.path, source))?;
            let name = entry.file_name();
            if !logs && let Some(session_id) = SessionBundlePaths::object_namespace_owner(&name) {
                mark_candidate_hint(
                    &mut hints,
                    base_session_id,
                    &session_id,
                    SessionCandidateHint::Occupied,
                );
                continue;
            }
            let Some(name) = name.to_str() else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            let classified = if logs {
                classify_log_candidate_leaf(&lower)
            } else {
                classify_session_candidate_leaf(dir, &lower, name == lower)
            };
            let Some((session_id, hint)) = classified else {
                continue;
            };
            mark_candidate_hint(&mut hints, base_session_id, session_id, hint);
        }
    }
    Ok(hints)
}

fn mark_candidate_hint(
    hints: &mut [SessionCandidateHint],
    base_session_id: &str,
    session_id: &str,
    hint: SessionCandidateHint,
) {
    for index in [
        (session_id == base_session_id).then_some(0),
        generated_suffix_candidate_index(base_session_id, session_id),
    ]
    .into_iter()
    .flatten()
    {
        hints[index] = hints[index].max(hint);
    }
}

fn classify_log_candidate_leaf(name: &str) -> Option<(&str, SessionCandidateHint)> {
    if let Some(id) = SessionBundlePaths::split_metadata_leaf(name) {
        return Some((id, SessionCandidateHint::Occupied));
    }
    segmented_candidate_id(name, true)
        .or_else(|| SessionBundlePaths::split_contexts_leaf(name))
        .map(|id| (id, SessionCandidateHint::Occupied))
}

fn classify_session_candidate_leaf<'a>(
    dir: &AnchoredDir,
    name: &'a str,
    canonical: bool,
) -> Option<(&'a str, SessionCandidateHint)> {
    if let Some(id) = SessionBundlePaths::split_lock_leaf(name) {
        let hint = match dir.file(name).metadata() {
            Ok(metadata) if !metadata.file_type().is_symlink() => SessionCandidateHint::Occupied,
            Err(RuntimeError::Io { source, .. })
                if !canonical && source.kind() == io::ErrorKind::NotFound =>
            {
                SessionCandidateHint::Occupied
            }
            _ => SessionCandidateHint::Probe,
        };
        return Some((id, hint));
    }
    if let Some(id) = segmented_candidate_id(name, false) {
        return Some((id, SessionCandidateHint::Occupied));
    }
    SessionBundlePaths::split_events_leaf(name).map(|id| {
        let hint = if canonical {
            SessionCandidateHint::Probe
        } else {
            SessionCandidateHint::Occupied
        };
        (id, hint)
    })
}

fn segmented_candidate_id(name: &str, contexts: bool) -> Option<&str> {
    let (stem, ordinal) = segmented_jsonl_leaf_stem(name)?.rsplit_once('.')?;
    if ordinal.len() != 6 || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if contexts {
        SessionBundlePaths::split_contexts_stem(stem)
    } else {
        Some(stem)
    }
}

fn generated_suffix_candidate_index(base_session_id: &str, session_id: &str) -> Option<usize> {
    let ordinal = session_id.rsplit_once('-')?.1.parse::<u32>().ok()?;
    if !(2..=MAX_UNIQUE_SESSION_CANDIDATES).contains(&ordinal) {
        return None;
    }
    (suffixed_session_id(base_session_id, ordinal) == session_id).then_some(ordinal as usize - 1)
}

pub fn suffixed_session_id(base_session_id: &str, ordinal: u32) -> String {
    let suffix = format!("-{ordinal}");
    let prefix_len = 128usize.saturating_sub(suffix.len());
    let prefix = if base_session_id.len() > prefix_len {
        &base_session_id[..prefix_len]
    } else {
        base_session_id
    };
    let candidate = format!("{prefix}{suffix}");
    debug_assert!(proto::is_valid_session_id(&candidate));
    candidate
}
