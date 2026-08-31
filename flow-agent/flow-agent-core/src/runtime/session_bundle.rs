use crate::runtime::{
    digest::is_lowercase_sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, RuntimeDirs, open_anchored_file_for_read, path_io_error,
        segmented_jsonl_leaf, segmented_jsonl_leaf_stem,
    },
    types::{
        MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECT_TOTAL_BYTES, MAX_SESSION_OBJECTS, RuntimeError,
    },
};
use std::{collections::BTreeMap, ffi::OsStr};

const CONTEXT_STREAM_STEM_SUFFIX: &str = ".contexts";
const LOCK_SUFFIX: &str = ".lock";
const METADATA_SUFFIX: &str = ".log";
const OBJECT_DIGEST_SEPARATOR: &str = ".object.sha256-";

#[derive(Clone, Debug)]
pub struct SessionBundlePaths {
    pub(crate) contexts: AnchoredFile,
    pub(crate) events: AnchoredFile,
    pub(crate) lock: AnchoredFile,
    pub(crate) metadata: AnchoredFile,
}

impl SessionBundlePaths {
    pub(crate) fn new(sessions: AnchoredDir, logs: AnchoredDir, session_id: &str) -> Self {
        Self {
            contexts: Self::contexts_in(&logs, session_id),
            events: Self::events_in(&sessions, session_id),
            lock: Self::lock_in(&sessions, session_id),
            metadata: Self::metadata_in(&logs, session_id),
        }
    }

    pub(crate) fn from_dirs(dirs: &RuntimeDirs, session_id: &str) -> Self {
        Self::new(dirs.sessions.clone(), dirs.logs.clone(), session_id)
    }

    pub(crate) fn contexts_in(logs: &AnchoredDir, session_id: &str) -> AnchoredFile {
        logs.file(Self::contexts_leaf(session_id))
    }

    pub(crate) fn events_in(sessions: &AnchoredDir, session_id: &str) -> AnchoredFile {
        sessions.file(Self::events_leaf(session_id))
    }

    pub(crate) fn lock_in(sessions: &AnchoredDir, session_id: &str) -> AnchoredFile {
        sessions.file(Self::lock_leaf(session_id))
    }

    pub(crate) fn metadata_in(logs: &AnchoredDir, session_id: &str) -> AnchoredFile {
        logs.file(Self::metadata_leaf(session_id))
    }

    pub(crate) fn contexts_leaf(session_id: &str) -> String {
        segmented_jsonl_leaf(&format!("{session_id}{CONTEXT_STREAM_STEM_SUFFIX}"), 1)
            .expect("first context segment ordinal is valid")
    }

    pub(crate) fn events_leaf(session_id: &str) -> String {
        segmented_jsonl_leaf(session_id, 1).expect("first event segment ordinal is valid")
    }

    pub(crate) fn lock_leaf(session_id: &str) -> String {
        format!("{session_id}{LOCK_SUFFIX}")
    }

    pub(crate) fn metadata_leaf(session_id: &str) -> String {
        format!("{session_id}{METADATA_SUFFIX}")
    }

    pub(crate) fn object_prefix(session_id: &str) -> String {
        format!("{session_id}{OBJECT_DIGEST_SEPARATOR}")
    }

    pub(crate) fn object_leaf(session_id: &str, digest: &str) -> String {
        format!("{}{digest}", Self::object_prefix(session_id))
    }

    pub(crate) fn object_namespace_owner(name: &OsStr) -> Option<String> {
        let bytes = name.as_encoded_bytes();
        let separator = OBJECT_DIGEST_SEPARATOR.as_bytes();
        let boundary = bytes
            .windows(separator.len())
            .position(|window| window.eq_ignore_ascii_case(separator))?;
        let owner = std::str::from_utf8(&bytes[..boundary])
            .ok()?
            .to_ascii_lowercase();
        proto::is_valid_session_id(&owner).then_some(owner)
    }

    pub(crate) fn object_in(
        sessions: &AnchoredDir,
        session_id: &str,
        digest: &str,
    ) -> AnchoredFile {
        sessions.file(Self::object_leaf(session_id, digest))
    }

    pub(crate) fn split_contexts_leaf(name: &str) -> Option<&str> {
        Self::split_contexts_stem(segmented_jsonl_leaf_stem(name)?)
    }

    pub(crate) fn split_contexts_stem(name: &str) -> Option<&str> {
        name.strip_suffix(CONTEXT_STREAM_STEM_SUFFIX)
    }

    pub(crate) fn split_events_leaf(name: &str) -> Option<&str> {
        segmented_jsonl_leaf_stem(name)
    }

    pub(crate) fn split_lock_leaf(name: &str) -> Option<&str> {
        name.strip_suffix(LOCK_SUFFIX)
    }

    pub(crate) fn split_metadata_leaf(name: &str) -> Option<&str> {
        name.strip_suffix(METADATA_SUFFIX)
    }

    pub(crate) fn split_object_leaf(name: &str) -> Option<(&str, &str)> {
        name.split_once(OBJECT_DIGEST_SEPARATOR)
    }
}

fn anchored_file_bytes(file: &AnchoredFile, maximum: u64) -> Result<u64, RuntimeError> {
    let (_, metadata) = open_anchored_file_for_read(file)?;
    let bytes = metadata.len();
    if bytes > maximum {
        return Err(RuntimeError::Protocol(format!(
            "{} size {bytes} bytes exceeds max {maximum}",
            file.diagnostic_path().display()
        )));
    }
    Ok(bytes)
}

pub(crate) fn session_objects(
    sessions: &AnchoredDir,
    session_id: &str,
) -> Result<(BTreeMap<String, AnchoredFile>, u64), RuntimeError> {
    let names = sessions
        .dir
        .entries()
        .map_err(|source| path_io_error(&sessions.path, source))?
        .map(|entry| {
            let entry = entry.map_err(|source| path_io_error(&sessions.path, source))?;
            let name = entry.file_name();
            match name.to_str() {
                Some(name) => Ok(Some(name.to_owned())),
                None if SessionBundlePaths::object_namespace_owner(&name).as_deref()
                    == Some(session_id) =>
                {
                    Err(RuntimeError::Protocol(format!(
                        "{} contains non-canonical session object name",
                        sessions.path.display()
                    )))
                }
                None => Ok(None),
            }
        });
    collect_session_objects(sessions, session_id, names, |path| {
        anchored_file_bytes(path, MAX_SESSION_OBJECT_BYTES)
    })
}

fn collect_session_objects(
    sessions: &AnchoredDir,
    session_id: &str,
    names: impl Iterator<Item = Result<Option<String>, RuntimeError>>,
    mut file_bytes: impl FnMut(&AnchoredFile) -> Result<u64, RuntimeError>,
) -> Result<(BTreeMap<String, AnchoredFile>, u64), RuntimeError> {
    let mut objects = BTreeMap::new();
    let mut total = 0u64;
    for name in names {
        let Some(name) = name? else {
            continue;
        };
        let candidate = name.to_ascii_lowercase();
        let Some((candidate_session_id, digest)) =
            SessionBundlePaths::split_object_leaf(&candidate)
        else {
            continue;
        };
        if candidate_session_id != session_id {
            continue;
        }
        if candidate != name || !is_lowercase_sha256_hex(digest) {
            return Err(RuntimeError::Protocol(format!(
                "{} contains non-canonical session object name {name}",
                sessions.path.display()
            )));
        }
        ensure_session_object_count(sessions, objects.len().saturating_add(1))?;
        let path = SessionBundlePaths::object_in(sessions, session_id, digest);
        let bytes = file_bytes(&path)?;
        total = total.saturating_add(bytes);
        ensure_session_object_total(total)?;
        objects.insert(digest.to_owned(), path);
    }
    Ok((objects, total))
}

pub(crate) fn ensure_session_object_count(
    sessions: &AnchoredDir,
    count: usize,
) -> Result<(), RuntimeError> {
    if count > MAX_SESSION_OBJECTS {
        return Err(RuntimeError::Protocol(format!(
            "{} session object count exceeds max {MAX_SESSION_OBJECTS}",
            sessions.path.display()
        )));
    }
    Ok(())
}

pub(crate) fn ensure_session_object_total(bytes: u64) -> Result<(), RuntimeError> {
    if bytes > MAX_SESSION_OBJECT_TOTAL_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "Run bundle object data size {bytes} bytes exceeds max {MAX_SESSION_OBJECT_TOTAL_BYTES}"
        )));
    }
    Ok(())
}
