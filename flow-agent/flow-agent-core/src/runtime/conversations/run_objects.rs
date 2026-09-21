use super::recovery_record::validate_recovery_object_uri;
use super::{
    contract::{RUN_OBJECTS_DIR, parse_run_object_digest, protocol},
    conversation_stream::conversation_file_sync_checkpoint,
    storage::{existing_anchored_run, required_child},
};
use crate::runtime::{
    context::ContextObject,
    digest::sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, create_anchored_file, open_anchored_file_for_read,
        open_anchored_file_for_update, path_io_error, sync_anchored_directory,
    },
    session_bundle::ensure_session_object_total,
    types::{MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECTS, RuntimeError},
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(crate) struct RunObjectStore {
    inner: Arc<Mutex<RunObjectWriter>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RunObjectUsageSnapshot {
    pub(crate) object_bytes: u64,
    pub(crate) object_count: usize,
}

#[derive(Clone, Copy)]
struct RunObjectEntry {
    directory_synced: bool,
    length: u32,
}

struct RunObjectWriter {
    accounted_bytes: u64,
    failed: bool,
    maximum_objects: usize,
    object_dir: AnchoredDir,
    objects: BTreeMap<[u8; 32], RunObjectEntry>,
}

impl RunObjectStore {
    pub(crate) fn open(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_limit(
            workspace,
            conversation_id,
            run_session_id,
            MAX_SESSION_OBJECTS,
        )
    }

    #[cfg(test)]
    pub(crate) fn open_with_object_limit_for_test(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        maximum_objects: usize,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_limit(workspace, conversation_id, run_session_id, maximum_objects)
    }

    fn open_with_limit(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        maximum_objects: usize,
    ) -> Result<Self, RuntimeError> {
        let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
        let object_dir = required_child(&run, RUN_OBJECTS_DIR, "run object directory")?;
        let mut accounted_bytes = 0_u64;
        let mut objects = BTreeMap::new();
        for entry in object_dir
            .dir
            .entries()
            .map_err(|source| path_io_error(&object_dir.path, source))?
        {
            let entry = entry.map_err(|source| path_io_error(&object_dir.path, source))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| protocol("run object name must be UTF-8"))?;
            let path = object_dir.file(&name);
            let metadata = path.metadata()?;
            if metadata.file_type().is_symlink()
                || !metadata.file_type().is_file()
                || metadata.len() > MAX_SESSION_OBJECT_BYTES
            {
                return Err(protocol("run object inventory is invalid"));
            }
            if objects.len() == maximum_objects {
                return Err(protocol("run object count exceeds its limit"));
            }
            let digest = parse_run_object_digest(&name)?;
            let length = u32::try_from(metadata.len())
                .map_err(|_| protocol("run object inventory is invalid"))?;
            accounted_bytes = accounted_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| protocol("run object byte count overflow"))?;
            objects.insert(
                digest,
                RunObjectEntry {
                    directory_synced: false,
                    length,
                },
            );
        }
        ensure_session_object_total(accounted_bytes)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(RunObjectWriter {
                accounted_bytes,
                failed: false,
                maximum_objects,
                object_dir,
                objects,
            })),
        })
    }

    pub(crate) fn persist(&self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        if objects.is_empty() {
            return Ok(());
        }
        let mut writer = self
            .inner
            .lock()
            .map_err(|_| protocol("run object store lock is poisoned"))?;
        writer.persist(objects)
    }

    pub(crate) fn read(&self, uri: &str) -> Result<Vec<u8>, RuntimeError> {
        let writer = self
            .inner
            .lock()
            .map_err(|_| protocol("run object store lock is poisoned"))?;
        if writer.failed {
            return Err(protocol("run object store is closed after a prior failure"));
        }
        read_run_object_uri(&writer.object_dir, uri)
    }

    pub(crate) fn usage_snapshot(&self) -> Result<RunObjectUsageSnapshot, RuntimeError> {
        let writer = self
            .inner
            .lock()
            .map_err(|_| protocol("run object store lock is poisoned"))?;
        if writer.failed {
            return Err(protocol("run object store is closed after a prior failure"));
        }
        Ok(RunObjectUsageSnapshot {
            object_bytes: writer.accounted_bytes,
            object_count: writer.objects.len(),
        })
    }
}

pub(super) fn read_run_object_uri(
    objects: &AnchoredDir,
    uri: &str,
) -> Result<Vec<u8>, RuntimeError> {
    let digest = validate_recovery_object_uri(uri)?;
    let path = objects.file(digest);
    let (file, metadata) = open_anchored_file_for_read(&path)?;
    if metadata.len() > MAX_SESSION_OBJECT_BYTES {
        return Err(protocol(
            "productive recovery object exceeds its byte limit",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(usize::MAX));
    file.take(MAX_SESSION_OBJECT_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    if sha256_hex(&bytes) != digest {
        return Err(protocol(
            "productive recovery object does not match its URI digest",
        ));
    }
    Ok(bytes)
}

impl RunObjectWriter {
    fn persist(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(protocol("run object store is closed after a prior failure"));
        }
        let mut candidates = BTreeMap::new();
        for object in objects {
            let digest = parse_run_object_digest(&object.digest)?;
            if sha256_hex(&object.bytes) != object.digest {
                return Err(protocol("run object digest does not match its bytes"));
            }
            let bytes = u64::try_from(object.bytes.len()).unwrap_or(u64::MAX);
            if bytes > MAX_SESSION_OBJECT_BYTES {
                return Err(protocol("run object exceeds its size limit"));
            }
            if let Some(prior) = candidates.insert(digest, object)
                && prior.bytes != object.bytes
            {
                return Err(protocol(
                    "run object batch maps one digest to different bytes",
                ));
            }
        }
        let new_count = candidates
            .keys()
            .filter(|digest| !self.objects.contains_key(*digest))
            .count();
        let candidate_count = self
            .objects
            .len()
            .checked_add(new_count)
            .ok_or_else(|| protocol("run object count overflow"))?;
        if candidate_count > self.maximum_objects {
            return Err(protocol("run object count exceeds its limit"));
        }
        let candidate_bytes = candidates
            .iter()
            .filter(|(digest, _)| !self.objects.contains_key(*digest))
            .try_fold(self.accounted_bytes, |total, (_, object)| {
                total
                    .checked_add(u64::try_from(object.bytes.len()).unwrap_or(u64::MAX))
                    .ok_or_else(|| protocol("run object byte count overflow"))
            })?;
        ensure_session_object_total(candidate_bytes)?;

        let mut newly_directory_synced = Vec::new();
        for (digest, object) in &candidates {
            let Some(entry) = self.objects.get(digest).copied() else {
                continue;
            };
            let path = self.object_dir.file(&object.digest);
            let metadata = match path.metadata() {
                Ok(metadata) => metadata,
                Err(error) => return self.fail(error),
            };
            if !metadata.file_type().is_file()
                || metadata.len() != u64::from(entry.length)
                || metadata.len() != u64::try_from(object.bytes.len()).unwrap_or(u64::MAX)
            {
                return self.fail(protocol("existing run object does not match its digest"));
            }
            let mut bytes = Vec::with_capacity(object.bytes.len());
            let mut file = match open_anchored_file_for_update(&path) {
                Ok((file, _)) => file,
                Err(error) => return self.fail(error),
            };
            if let Err(source) = file.read_to_end(&mut bytes) {
                return self.fail(path_io_error(path.diagnostic_path(), source));
            }
            if bytes != object.bytes {
                return self.fail(protocol("existing run object does not match its digest"));
            }
            conversation_file_sync_checkpoint(path.diagnostic_path())?;
            file.sync_all()
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            if !entry.directory_synced {
                newly_directory_synced.push(*digest);
            }
        }
        if !newly_directory_synced.is_empty() {
            sync_anchored_directory(&self.object_dir)?;
        }

        for (digest, object) in candidates {
            if self.objects.contains_key(&digest) {
                continue;
            }
            let path = self.object_dir.file(&object.digest);
            let mut file = match create_anchored_file(&path) {
                Ok(file) => file,
                Err(error) => return self.fail(error),
            };
            if let Err(source) = file.write_all(&object.bytes).and_then(|()| file.sync_all()) {
                drop(file);
                return self.fail_created(path_io_error(path.diagnostic_path(), source), &path);
            }
            drop(file);
            if let Err(error) = sync_anchored_directory(&self.object_dir) {
                return self.fail_created(error, &path);
            }
            let object_bytes =
                u32::try_from(object.bytes.len()).expect("session object byte limit fits in u32");
            self.objects.insert(
                digest,
                RunObjectEntry {
                    directory_synced: true,
                    length: object_bytes,
                },
            );
            self.accounted_bytes = self
                .accounted_bytes
                .checked_add(u64::from(object_bytes))
                .expect("candidate byte total was checked before publication");
        }
        for digest in newly_directory_synced {
            self.objects
                .get_mut(&digest)
                .expect("existing candidate remains inventoried")
                .directory_synced = true;
        }
        Ok(())
    }

    fn fail<T>(&mut self, error: RuntimeError) -> Result<T, RuntimeError> {
        self.failed = true;
        Err(error)
    }

    fn fail_created<T>(
        &mut self,
        error: RuntimeError,
        path: &AnchoredFile,
    ) -> Result<T, RuntimeError> {
        let cleanup = path
            .remove()
            .and_then(|()| sync_anchored_directory(&self.object_dir));
        if cleanup.is_err() {
            self.failed = true;
        }
        Err(error)
    }
}
