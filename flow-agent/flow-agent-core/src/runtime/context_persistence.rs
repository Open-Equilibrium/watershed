use crate::runtime::{
    config_io::path_io_error,
    context::{ContextManifestCheckpoint, ContextObject, sha256_hex},
    event_writer::{BatchAppendFailure, EventLogAppender, SessionLogAppender},
    fixture_tools::with_anchored_replacement_temp,
    fs_guards::{
        AnchoredDir, AnchoredFile, ensure_anchored_new_leaf_available,
        for_each_segmented_jsonl_line, open_anchored_session_log_append_file,
        read_anchored_file_with_limit,
    },
    session_bundle::{ensure_session_object_count, session_objects},
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, MAX_SESSION_CONTEXT_MANIFEST_BYTES,
        MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECT_TOTAL_BYTES, RuntimeError,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, io,
    io::Write,
    path::Path,
};

pub struct ContextManifestWriter {
    pub(crate) appender: SessionLogAppender,
    pub(crate) byte_count: u64,
    pub(crate) last_manifest: Option<String>,
    pub(crate) manifest_count: usize,
    pub(crate) object_writer: Option<SessionObjectWriter>,
}

pub(crate) fn context_manifest_inventory(
    path: &AnchoredFile,
) -> Result<(Option<String>, usize, u64), RuntimeError> {
    let mut last_manifest = None;
    let mut manifest_count = 0usize;
    let byte_count = for_each_segmented_jsonl_line(path, CONTEXT_MANIFEST_STREAM_LIMITS, |line| {
        if !line.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest stream must end with LF",
                path.diagnostic_path().display()
            )));
        }
        last_manifest = Some(line.to_owned());
        manifest_count = manifest_count.saturating_add(1);
        Ok(())
    })?;
    Ok((last_manifest, manifest_count, byte_count))
}

pub(crate) fn validate_context_manifest_checkpoint(
    path: &Path,
    manifest_count: usize,
    last_manifest: Option<&str>,
    checkpoint: &ContextManifestCheckpoint,
) -> Result<bool, RuntimeError> {
    let replay = checkpoint.ordinal == manifest_count;
    if replay && last_manifest != Some(&checkpoint.manifest.line) {
        return Err(RuntimeError::Protocol(format!(
            "{} in-flight context manifest does not match deterministic replay",
            path.display()
        )));
    }
    if !replay && checkpoint.ordinal != manifest_count.saturating_add(1) {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest ordinal {} does not follow persisted ordinal {}",
            path.display(),
            checkpoint.ordinal,
            manifest_count
        )));
    }
    if checkpoint.manifest.line.is_empty() || !checkpoint.manifest.line.ends_with('\n') {
        return Err(RuntimeError::Protocol(
            "context manifest must be one LF-terminated JSONL record".to_owned(),
        ));
    }
    Ok(replay)
}

impl ContextManifestWriter {
    #[cfg(test)]
    pub(crate) fn open(path: &AnchoredFile) -> Result<Self, RuntimeError> {
        Self::open_with_object_writer(path, None)
    }

    pub(crate) fn open_for_session(
        path: &AnchoredFile,
        object_parent: AnchoredDir,
        session_id: &str,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_object_writer(
            path,
            Some(SessionObjectWriter::open(object_parent, session_id)?),
        )
    }

    pub(crate) fn open_with_object_writer(
        path: &AnchoredFile,
        object_writer: Option<SessionObjectWriter>,
    ) -> Result<Self, RuntimeError> {
        let (last_manifest, manifest_count, byte_count) = context_manifest_inventory(path)?;
        Ok(Self {
            appender: SessionLogAppender::open_with_limits(path, CONTEXT_MANIFEST_STREAM_LIMITS)?,
            byte_count,
            last_manifest,
            manifest_count,
            object_writer,
        })
    }

    pub(crate) fn persist(
        &mut self,
        path: &AnchoredFile,
        checkpoint: &ContextManifestCheckpoint,
    ) -> Result<(), RuntimeError> {
        let path = path.diagnostic_path();
        let replay = validate_context_manifest_checkpoint(
            path,
            self.manifest_count,
            self.last_manifest.as_deref(),
            checkpoint,
        )?;
        let total = if replay {
            self.byte_count
        } else {
            ensure_context_manifest_growth_within_limit(
                path,
                self.byte_count,
                checkpoint.manifest.line.len(),
            )?
        };
        let actual = self.appender.len(path)?;
        if actual != self.byte_count {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside context manifest append semantics",
                path.display()
            )));
        }
        if let Some(object_writer) = self.object_writer.as_mut() {
            object_writer.persist_all(&checkpoint.objects)?;
        }
        if replay {
            return self.appender.sync(path);
        }
        self.appender
            .append(path, checkpoint.manifest.line.as_bytes())?;
        self.appender.sync(path)?;
        self.byte_count = total;
        self.last_manifest = Some(checkpoint.manifest.line.clone());
        self.manifest_count = checkpoint.ordinal;
        Ok(())
    }
}

pub struct SessionObjectWriter {
    pub(crate) accounted_bytes: u64,
    pub(crate) known: BTreeSet<String>,
    pub(crate) object_count: usize,
    pub(crate) object_parent: AnchoredDir,
    pub(crate) preflight_accounted_bytes: u64,
    pub(crate) preflight_known: BTreeSet<String>,
    pub(crate) preflight_object_count: usize,
    pub(crate) session_id: String,
    pub(crate) verified: BTreeSet<String>,
}

struct ValidatedObjectBatch {
    accounted_bytes: u64,
    known: BTreeSet<String>,
    object_count: usize,
    verified: BTreeSet<String>,
}

impl SessionObjectWriter {
    pub(crate) fn open(object_parent: AnchoredDir, session_id: &str) -> Result<Self, RuntimeError> {
        let (objects, accounted_bytes) = session_objects(&object_parent, session_id)?;
        let known = objects.into_keys().collect::<BTreeSet<_>>();
        let object_count = known.len();
        ensure_session_object_count(&object_parent, object_count)?;
        Ok(Self {
            accounted_bytes,
            known: known.clone(),
            object_count,
            object_parent,
            preflight_accounted_bytes: accounted_bytes,
            preflight_known: known,
            preflight_object_count: object_count,
            session_id: session_id.to_owned(),
            verified: BTreeSet::new(),
        })
    }

    pub(crate) fn persist_all(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        self.validate_all(objects)?;
        for object in objects {
            self.persist(object)?;
        }
        Ok(())
    }

    pub(crate) fn preflight_all(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        let validated = self.validate_all_from(
            objects,
            self.preflight_accounted_bytes.max(self.accounted_bytes),
            self.preflight_object_count.max(self.object_count),
            &self.preflight_known,
        )?;
        self.preflight_accounted_bytes = validated.accounted_bytes;
        self.preflight_known = validated.known;
        self.preflight_object_count = validated.object_count;
        self.verified = validated.verified;
        Ok(())
    }

    fn validate_all(
        &self,
        objects: &[ContextObject],
    ) -> Result<ValidatedObjectBatch, RuntimeError> {
        self.validate_all_from(
            objects,
            self.accounted_bytes,
            self.object_count,
            &self.known,
        )
    }

    fn validate_all_from(
        &self,
        objects: &[ContextObject],
        mut accounted_bytes: u64,
        mut object_count: usize,
        known: &BTreeSet<String>,
    ) -> Result<ValidatedObjectBatch, RuntimeError> {
        let mut prospective_known = known.clone();
        let mut verified = self.verified.clone();
        let mut unique = BTreeMap::new();
        for object in objects {
            let object_bytes = Self::validate_object(object)?;
            if unique.contains_key(&object.digest) {
                continue;
            }
            if prospective_known.insert(object.digest.clone()) {
                object_count = object_count.saturating_add(1);
                ensure_session_object_count(&self.object_parent, object_count)?;
                accounted_bytes = accounted_bytes.saturating_add(object_bytes);
                ensure_session_object_total(accounted_bytes)?;
            }
            unique.insert(object.digest.clone(), object);
        }

        for object in unique.into_values() {
            if verified.contains(&object.digest) {
                continue;
            }
            let path = self.object_parent.file(format!(
                "{}.object.sha256-{}",
                self.session_id, object.digest
            ));
            if self.known.contains(&object.digest) {
                Self::verify_existing(&path, object)?;
                verified.insert(object.digest.clone());
                continue;
            }
            match path.metadata() {
                Ok(_) => {
                    Self::verify_existing(&path, object)?;
                    verified.insert(object.digest.clone());
                }
                Err(RuntimeError::Io { source, .. })
                    if source.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(ValidatedObjectBatch {
            accounted_bytes,
            known: prospective_known,
            object_count,
            verified,
        })
    }

    fn validate_object(object: &ContextObject) -> Result<u64, RuntimeError> {
        let object_bytes = u64::try_from(object.bytes.len()).unwrap_or(u64::MAX);
        ensure_session_object_size(&object.digest, object_bytes)?;
        if sha256_hex(&object.bytes) != object.digest {
            return Err(RuntimeError::Protocol(format!(
                "session object {} does not match its content hash",
                object.digest
            )));
        }
        Ok(object_bytes)
    }

    fn verify_existing(path: &AnchoredFile, object: &ContextObject) -> Result<(), RuntimeError> {
        let existing = read_anchored_file_with_limit(path, MAX_SESSION_OBJECT_BYTES)?;
        if existing != object.bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} does not match referenced session object bytes",
                path.diagnostic_path().display()
            )));
        }
        Ok(())
    }

    pub(crate) fn persist(&mut self, object: &ContextObject) -> Result<(), RuntimeError> {
        self.persist_with(object, |path, bytes| {
            let mut file = open_anchored_session_log_append_file(path)?;
            file.write_all(bytes)
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            file.sync_all()
                .map_err(|source| path_io_error(path.diagnostic_path(), source))
        })
    }

    pub(crate) fn persist_with(
        &mut self,
        object: &ContextObject,
        write_new: impl FnOnce(&AnchoredFile, &[u8]) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let validated = self.validate_all(std::slice::from_ref(object))?;
        if self.verified.contains(&object.digest) {
            return Ok(());
        }
        let object_bytes = Self::validate_object(object)?;
        let was_known = self.known.contains(&object.digest);
        let path = self.object_parent.file(format!(
            "{}.object.sha256-{}",
            self.session_id, object.digest
        ));
        match path.metadata() {
            Ok(_) => {
                Self::verify_existing(&path, object)?;
                if !was_known {
                    self.record_publication(&object.digest, object_bytes);
                }
            }
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                if was_known {
                    return Err(RuntimeError::Protocol(format!(
                        "{} known session object is unavailable",
                        path.diagnostic_path().display()
                    )));
                }
                with_anchored_replacement_temp(&path, None, |temp_path, temp_file| {
                    drop(temp_file);
                    write_new(temp_path, &object.bytes)?;
                    ensure_anchored_new_leaf_available(&path)?;
                    temp_path.rename_to(&path)
                })?;
                self.record_publication(&object.digest, object_bytes);
            }
            Err(error) => return Err(error),
        }
        self.verified.insert(object.digest.clone());
        debug_assert_eq!(self.accounted_bytes, validated.accounted_bytes);
        debug_assert_eq!(self.object_count, validated.object_count);
        debug_assert_eq!(self.known, validated.known);
        Ok(())
    }

    fn record_publication(&mut self, digest: &str, object_bytes: u64) {
        if self.known.insert(digest.to_owned()) {
            self.object_count = self.object_count.saturating_add(1);
            self.accounted_bytes = self.accounted_bytes.saturating_add(object_bytes);
        }
        if self.preflight_known.insert(digest.to_owned()) {
            self.preflight_object_count = self.preflight_object_count.saturating_add(1);
            self.preflight_accounted_bytes =
                self.preflight_accounted_bytes.saturating_add(object_bytes);
        }
    }
}

pub fn ensure_session_object_size(
    label: impl fmt::Display,
    bytes: u64,
) -> Result<(), RuntimeError> {
    if bytes > MAX_SESSION_OBJECT_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{label} session object is {bytes} bytes; max {MAX_SESSION_OBJECT_BYTES}"
        )));
    }
    Ok(())
}

pub fn ensure_session_object_total(bytes: u64) -> Result<(), RuntimeError> {
    if bytes > MAX_SESSION_OBJECT_TOTAL_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "session bundle object data size {bytes} bytes exceeds max {MAX_SESSION_OBJECT_TOTAL_BYTES}"
        )));
    }
    Ok(())
}

pub fn ensure_context_manifest_growth_within_limit(
    path: &Path,
    current_bytes: impl TryInto<u64>,
    appended_bytes: usize,
) -> Result<u64, RuntimeError> {
    let current_bytes = current_bytes.try_into().unwrap_or(u64::MAX);
    let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
    let total = current_bytes.saturating_add(appended_bytes);
    if total > MAX_SESSION_CONTEXT_MANIFEST_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest size {total} bytes exceeds max {MAX_SESSION_CONTEXT_MANIFEST_BYTES}",
            path.display()
        )));
    }
    Ok(total)
}

impl BatchAppendFailure {
    pub(crate) fn none_committed(error: RuntimeError) -> Self {
        Self {
            committed_events: Some(0),
            error,
        }
    }
}
