use super::*;

#[derive(Clone, Debug)]
pub struct SessionBundlePaths {
    pub(crate) contexts: AnchoredFile,
    pub(crate) events: AnchoredFile,
    pub(crate) lock: AnchoredFile,
    pub(crate) metadata: AnchoredFile,
    pub(crate) session_id: String,
    pub(crate) sessions: AnchoredDir,
}

impl SessionBundlePaths {
    pub(crate) fn new(sessions: AnchoredDir, logs: AnchoredDir, session_id: &str) -> Self {
        Self {
            contexts: Self::contexts_in(&logs, session_id),
            events: Self::events_in(&sessions, session_id),
            lock: Self::lock_in(&sessions, session_id),
            metadata: Self::metadata_in(&logs, session_id),
            session_id: session_id.to_owned(),
            sessions,
        }
    }

    pub(crate) fn from_dirs(dirs: &RuntimeDirs, session_id: &str) -> Self {
        Self::new(dirs.sessions.clone(), dirs.logs.clone(), session_id)
    }

    #[cfg(test)]
    pub(crate) fn from_reservation(reservation: &SessionReservation) -> Self {
        Self::new(
            reservation.session_path.parent.clone(),
            reservation.log_path.parent.clone(),
            &reservation.session_id,
        )
    }

    pub(crate) fn contexts_in(logs: &AnchoredDir, session_id: &str) -> AnchoredFile {
        logs.file(format!("{session_id}.contexts.jsonl"))
    }

    pub(crate) fn events_in(sessions: &AnchoredDir, session_id: &str) -> AnchoredFile {
        sessions.file(format!("{session_id}.jsonl"))
    }

    pub(crate) fn lock_in(sessions: &AnchoredDir, session_id: &str) -> AnchoredFile {
        sessions.file(format!("{session_id}.lock"))
    }

    pub(crate) fn metadata_in(logs: &AnchoredDir, session_id: &str) -> AnchoredFile {
        logs.file(format!("{session_id}.log"))
    }
}

#[derive(Debug)]
pub struct SessionBundleInventory {
    pub(crate) context_bytes: u64,
    pub(crate) context_segments: Vec<AnchoredFile>,
    pub(crate) event_bytes: u64,
    pub(crate) event_segments: Vec<AnchoredFile>,
    #[cfg(test)]
    pub(crate) lock_present: bool,
    pub(crate) metadata_bytes: u64,
    pub(crate) object_bytes: u64,
    pub(crate) objects: BTreeMap<String, AnchoredFile>,
    paths: SessionBundlePaths,
}

impl SessionBundleInventory {
    pub(crate) fn inspect(paths: SessionBundlePaths) -> Result<Self, RuntimeError> {
        let event_segments =
            required_segmented_jsonl_files(&paths.events, EVENT_STREAM_LIMITS, "event stream")?;
        let event_bytes = segment_bytes(&event_segments, MAX_SESSION_EVENT_BYTES)?;
        let context_segments = required_segmented_jsonl_files(
            &paths.contexts,
            CONTEXT_MANIFEST_STREAM_LIMITS,
            "context manifest stream",
        )?;
        let context_bytes = segment_bytes(&context_segments, MAX_SESSION_CONTEXT_MANIFEST_BYTES)?;
        let metadata_bytes = anchored_file_bytes(&paths.metadata, MAX_SESSION_METADATA_BYTES)?;
        let (objects, object_bytes) = session_objects(&paths.sessions, &paths.session_id)?;
        #[cfg(test)]
        let lock_present = anchored_leaf_present(&paths.lock)?;
        let inventory = Self {
            context_bytes,
            context_segments,
            event_bytes,
            event_segments,
            #[cfg(test)]
            lock_present,
            metadata_bytes,
            object_bytes,
            objects,
            paths,
        };
        if inventory.total_bytes() > MAX_SESSION_BUNDLE_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "session bundle size {} bytes exceeds max {MAX_SESSION_BUNDLE_BYTES}",
                inventory.total_bytes()
            )));
        }
        Ok(inventory)
    }

    pub(crate) fn total_bytes(&self) -> u64 {
        self.event_bytes
            .saturating_add(self.context_bytes)
            .saturating_add(self.metadata_bytes)
            .saturating_add(self.object_bytes)
    }

    pub(crate) fn validate_resumable_bundle(&self) -> Result<(), RuntimeError> {
        if self.event_segments.is_empty() {
            return Err(RuntimeError::Protocol(format!(
                "{} event stream is missing",
                self.paths.events.diagnostic_path().display()
            )));
        }
        if self.context_segments.is_empty() {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest stream is missing",
                self.paths.contexts.diagnostic_path().display()
            )));
        }
        for (digest, path) in &self.objects {
            let expected = format!("{}.object.sha256-{digest}", self.paths_session_id());
            if path.leaf != Path::new(&expected) {
                return Err(RuntimeError::Protocol(format!(
                    "{} does not match its session object inventory key",
                    path.diagnostic_path().display()
                )));
            }
        }
        Ok(())
    }

    fn paths_session_id(&self) -> &str {
        &self.paths.session_id
    }
}

fn required_segmented_jsonl_files(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
    stream_name: &str,
) -> Result<Vec<AnchoredFile>, RuntimeError> {
    match segmented_jsonl_files(base, limits) {
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Err(RuntimeError::Protocol(format!(
                "{} {stream_name} is missing",
                base.diagnostic_path().display()
            )))
        }
        result => result,
    }
}

fn segment_bytes(files: &[AnchoredFile], maximum: u64) -> Result<u64, RuntimeError> {
    let mut total = 0u64;
    for file in files {
        total = total.saturating_add(anchored_file_bytes(file, MAX_SESSION_SEGMENT_BYTES)?);
        if total > maximum {
            return Err(RuntimeError::Protocol(format!(
                "{} segmented JSONL size {total} bytes exceeds max {maximum}",
                files[0].diagnostic_path().display()
            )));
        }
    }
    Ok(total)
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

#[cfg(test)]
fn anchored_leaf_present(file: &AnchoredFile) -> Result<bool, RuntimeError> {
    match file.metadata() {
        Ok(_) => {
            ensure_anchored_non_hardlinked_file(file)?;
            Ok(true)
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
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
            entry
                .map(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .map(std::borrow::ToOwned::to_owned)
                })
                .map_err(|source| path_io_error(&sessions.path, source))
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
    let prefix = format!("{session_id}.object.sha256-");
    let mut objects = BTreeMap::new();
    let mut total = 0u64;
    for name in names {
        let Some(name) = name? else {
            continue;
        };
        let candidate = name.to_ascii_lowercase();
        let Some(digest) = candidate.strip_prefix(&prefix) else {
            continue;
        };
        if candidate != name || !is_lowercase_sha256_hex(digest) {
            return Err(RuntimeError::Protocol(format!(
                "{} contains non-canonical session object name {name}",
                sessions.path.display()
            )));
        }
        ensure_session_object_count(sessions, objects.len().saturating_add(1))?;
        let path = sessions.file(&name);
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

#[cfg(test)]
pub(crate) fn generated_zero_byte_session_objects_for_test(
    sessions: &AnchoredDir,
    session_id: &str,
    count: usize,
    opened: &Cell<usize>,
) -> Result<(BTreeMap<String, AnchoredFile>, u64), RuntimeError> {
    let names =
        (0..count).map(|index| Ok(Some(format!("{session_id}.object.sha256-{index:064x}"))));
    collect_session_objects(sessions, session_id, names, |_| {
        opened.set(opened.get() + 1);
        Ok(0)
    })
}

pub(crate) fn session_ids(sessions: &AnchoredDir) -> Result<Vec<String>, RuntimeError> {
    let mut ids = Vec::new();
    for entry in sessions
        .dir
        .entries()
        .map_err(|source| path_io_error(&sessions.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&sessions.path, source))?;
        let name = entry.file_name();
        let path = Path::new(&name);
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("jsonl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if !proto::is_valid_session_id(id)
            || open_anchored_file_for_read(&sessions.file(name.clone())).is_err()
        {
            continue;
        }
        ids.push(id.to_owned());
    }
    ids.sort();
    Ok(ids)
}

pub const MAX_UNIQUE_SESSION_CANDIDATES: u32 = 10_000;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum SessionCandidateHint {
    Free,
    Occupied,
    Probe,
}

pub fn session_candidate_hints(
    dirs: &RuntimeDirs,
    base_session_id: &str,
) -> Result<Vec<SessionCandidateHint>, RuntimeError> {
    let mut hints = vec![SessionCandidateHint::Free; MAX_UNIQUE_SESSION_CANDIDATES as usize];
    for (dir, logs) in [(&dirs.sessions, false), (&dirs.logs, true)] {
        for entry in dir
            .dir
            .entries()
            .map_err(|source| path_io_error(&dir.path, source))?
        {
            let entry = entry.map_err(|source| path_io_error(&dir.path, source))?;
            let name = entry.file_name();
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
    }
    Ok(hints)
}

fn classify_log_candidate_leaf(name: &str) -> Option<(&str, SessionCandidateHint)> {
    if let Some(id) = name.strip_suffix(".log") {
        return Some((id, SessionCandidateHint::Occupied));
    }
    segmented_candidate_id(name, true)
        .or_else(|| name.strip_suffix(".contexts.jsonl"))
        .map(|id| (id, SessionCandidateHint::Occupied))
}

fn classify_session_candidate_leaf<'a>(
    dir: &AnchoredDir,
    name: &'a str,
    canonical: bool,
) -> Option<(&'a str, SessionCandidateHint)> {
    if let Some((id, _)) = name.split_once(".object.sha256-") {
        return Some((id, SessionCandidateHint::Occupied));
    }
    if let Some(id) = name.strip_suffix(".lock") {
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
    name.strip_suffix(".jsonl").map(|id| {
        let hint = if canonical {
            SessionCandidateHint::Probe
        } else {
            SessionCandidateHint::Occupied
        };
        (id, hint)
    })
}

fn segmented_candidate_id(name: &str, contexts: bool) -> Option<&str> {
    let (stem, ordinal) = name.strip_suffix(".jsonl")?.rsplit_once('.')?;
    if ordinal.len() != 6 || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if contexts {
        stem.strip_suffix(".contexts")
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
