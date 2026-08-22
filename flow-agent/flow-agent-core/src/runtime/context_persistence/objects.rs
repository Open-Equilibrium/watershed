use crate::runtime::{
    context::ContextObject,
    digest::sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, ensure_anchored_new_leaf_available,
        open_anchored_file_for_update, open_anchored_session_log_append_file, path_io_error,
        read_anchored_file_with_limit, sync_directory, with_anchored_replacement_temp,
    },
    session_bundle::{
        SessionBundlePaths, ensure_session_object_count, ensure_session_object_total,
        session_objects,
    },
    types::{MAX_SESSION_OBJECT_BYTES, RuntimeError},
};
use proto::decode_lowercase_sha256_hex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, io,
    io::{Read, Write},
};

pub struct SessionObjectWriter {
    pub(crate) accounted_bytes: u64,
    pub(crate) object_count: usize,
    pub(crate) object_parent: AnchoredDir,
    pub(crate) preflight_accounted_bytes: u64,
    pub(crate) preflight_object_count: usize,
    pub(crate) session_id: String,
    objects: BTreeMap<[u8; 32], SessionObjectState>,
}

#[derive(Default, Eq, PartialEq)]
struct SessionObjectState {
    preflight: bool,
    published: bool,
}

impl SessionObjectState {
    const PUBLISHED: Self = Self {
        preflight: true,
        published: true,
    };

    fn snapshot(&self) -> Self {
        Self {
            preflight: self.preflight,
            published: self.published,
        }
    }
}

#[derive(Clone, Copy)]
enum InventoryView {
    Preflight,
    Published,
}

#[derive(Clone, Copy)]
enum ObjectDisposition {
    Absent,
    ExistingPublished,
    ExistingUnpublished,
}

struct ValidatedObject<'objects> {
    digest: [u8; 32],
    disposition: ObjectDisposition,
    object: &'objects ContextObject,
    object_bytes: u64,
}

struct ValidatedObjectBatch<'objects> {
    accounted_bytes: u64,
    object_count: usize,
    objects: Vec<ValidatedObject<'objects>>,
}

impl SessionObjectWriter {
    pub(crate) fn open(object_parent: AnchoredDir, session_id: &str) -> Result<Self, RuntimeError> {
        let (objects, accounted_bytes) = session_objects(&object_parent, session_id)?;
        let objects = objects
            .into_keys()
            .map(|digest| {
                Ok((
                    parse_session_object_digest(&digest)?,
                    SessionObjectState::PUBLISHED,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, RuntimeError>>()?;
        let object_count = objects.len();
        ensure_session_object_count(&object_parent, object_count)?;
        Ok(Self {
            accounted_bytes,
            object_count,
            object_parent,
            preflight_accounted_bytes: accounted_bytes,
            preflight_object_count: object_count,
            session_id: session_id.to_owned(),
            objects,
        })
    }

    #[cfg(test)]
    pub(crate) fn persist_all(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        let validated = self.validate_all(objects)?;
        self.persist_validated_batch_with(validated.objects, write_session_object)
    }

    pub(crate) fn persist_manifest_objects(
        &mut self,
        referenced: &BTreeSet<String>,
        objects: &[ContextObject],
    ) -> Result<(), RuntimeError> {
        let validated = self.validate_all(objects)?;
        let supplied = validated
            .objects
            .iter()
            .map(|object| object.object.digest.as_str())
            .collect::<BTreeSet<_>>();
        if supplied.len() != referenced.len()
            || supplied.iter().any(|digest| !referenced.contains(*digest))
        {
            return Err(RuntimeError::Protocol(
                "context manifest object references do not match supplied objects".to_owned(),
            ));
        }
        self.persist_validated_batch_with(validated.objects, write_session_object)
    }

    pub(crate) fn preflight_all(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        let validated = self.validate_all_from(
            objects,
            self.preflight_accounted_bytes.max(self.accounted_bytes),
            self.preflight_object_count.max(self.object_count),
            InventoryView::Preflight,
        )?;
        self.preflight_accounted_bytes = validated.accounted_bytes;
        self.preflight_object_count = validated.object_count;
        for object in validated.objects {
            let state = self.objects.entry(object.digest).or_default();
            state.preflight = true;
        }
        Ok(())
    }

    fn validate_all<'objects>(
        &self,
        objects: &'objects [ContextObject],
    ) -> Result<ValidatedObjectBatch<'objects>, RuntimeError> {
        self.validate_all_from(
            objects,
            self.accounted_bytes,
            self.object_count,
            InventoryView::Published,
        )
    }

    fn validate_all_from<'objects>(
        &self,
        objects: &'objects [ContextObject],
        mut accounted_bytes: u64,
        mut object_count: usize,
        view: InventoryView,
    ) -> Result<ValidatedObjectBatch<'objects>, RuntimeError> {
        let mut unique = BTreeMap::new();
        for object in objects {
            let object_bytes = Self::validate_object(object)?;
            let digest = parse_session_object_digest(&object.digest)?;
            unique.entry(digest).or_insert((object, object_bytes));
        }

        for (digest, (_, object_bytes)) in &unique {
            let known = self.objects.get(digest).is_some_and(|state| match view {
                InventoryView::Preflight => state.preflight,
                InventoryView::Published => state.published,
            });
            if !known {
                object_count = object_count.saturating_add(1);
                ensure_session_object_count(&self.object_parent, object_count)?;
                accounted_bytes = accounted_bytes.saturating_add(*object_bytes);
                ensure_session_object_total(accounted_bytes)?;
            }
        }

        let mut validated = Vec::with_capacity(unique.len());
        for (digest, (object, object_bytes)) in unique {
            let state = self
                .objects
                .get(&digest)
                .map_or_else(SessionObjectState::default, SessionObjectState::snapshot);
            let path = SessionBundlePaths::object_in(
                &self.object_parent,
                &self.session_id,
                &object.digest,
            );
            let disposition = match path.metadata() {
                Ok(_) => {
                    Self::verify_existing(&path, object)?;
                    if state.published {
                        ObjectDisposition::ExistingPublished
                    } else {
                        ObjectDisposition::ExistingUnpublished
                    }
                }
                Err(RuntimeError::Io { source, .. })
                    if source.kind() == io::ErrorKind::NotFound && !state.published =>
                {
                    ObjectDisposition::Absent
                }
                Err(RuntimeError::Io { source, .. })
                    if source.kind() == io::ErrorKind::NotFound =>
                {
                    return Err(RuntimeError::Protocol(format!(
                        "{} known session object is unavailable",
                        path.diagnostic_path().display()
                    )));
                }
                Err(error) => return Err(error),
            };
            validated.push(ValidatedObject {
                digest,
                disposition,
                object,
                object_bytes,
            });
        }
        Ok(ValidatedObjectBatch {
            accounted_bytes,
            object_count,
            objects: validated,
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

    fn verify_and_sync_existing(
        path: &AnchoredFile,
        object: &ContextObject,
    ) -> Result<(), RuntimeError> {
        let (mut file, _) = open_anchored_file_for_update(path)?;
        let mut existing = Vec::with_capacity(object.bytes.len());
        let limit = u64::try_from(object.bytes.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        (&mut file)
            .take(limit)
            .read_to_end(&mut existing)
            .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
        if existing != object.bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} does not match referenced session object bytes",
                path.diagnostic_path().display()
            )));
        }
        file.sync_all()
            .map_err(|source| path_io_error(path.diagnostic_path(), source))
    }

    #[cfg(test)]
    pub(crate) fn persist(&mut self, object: &ContextObject) -> Result<(), RuntimeError> {
        self.persist_with(object, write_session_object)
    }

    #[cfg(test)]
    pub(crate) fn persist_with(
        &mut self,
        object: &ContextObject,
        write_new: impl FnOnce(&AnchoredFile, &[u8]) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let validated = self.validate_all(std::slice::from_ref(object))?.objects;
        let mut write_new = Some(write_new);
        self.persist_validated_batch_with(validated, |path, bytes| {
            write_new
                .take()
                .expect("one object batch writes at most one new candidate")(path, bytes)
        })
    }

    #[cfg(test)]
    pub(crate) fn persist_all_with(
        &mut self,
        objects: &[ContextObject],
        write_new: impl FnMut(&AnchoredFile, &[u8]) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let validated = self.validate_all(objects)?.objects;
        self.persist_validated_batch_with(validated, write_new)
    }

    fn persist_validated_batch_with(
        &mut self,
        validated: Vec<ValidatedObject<'_>>,
        mut write_new: impl FnMut(&AnchoredFile, &[u8]) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let mut sync_parent = false;
        for object in &validated {
            if matches!(
                object.disposition,
                ObjectDisposition::ExistingPublished | ObjectDisposition::ExistingUnpublished
            ) {
                let path = SessionBundlePaths::object_in(
                    &self.object_parent,
                    &self.session_id,
                    &object.object.digest,
                );
                Self::verify_and_sync_existing(&path, object.object)?;
                if matches!(object.disposition, ObjectDisposition::ExistingUnpublished) {
                    self.record_publication(object.digest, object.object_bytes);
                }
                sync_parent = true;
            }
        }

        for object in &validated {
            if !matches!(object.disposition, ObjectDisposition::Absent) {
                continue;
            }
            let path = SessionBundlePaths::object_in(
                &self.object_parent,
                &self.session_id,
                &object.object.digest,
            );
            match path.metadata() {
                Ok(_) => {
                    Self::verify_and_sync_existing(&path, object.object)?;
                    self.record_publication(object.digest, object.object_bytes);
                    sync_parent = true;
                }
                Err(RuntimeError::Io { source, .. })
                    if source.kind() == io::ErrorKind::NotFound =>
                {
                    with_anchored_replacement_temp(&path, None, |temp_path, temp_file| {
                        drop(temp_file);
                        write_new(temp_path, &object.object.bytes)?;
                        ensure_anchored_new_leaf_available(&path)?;
                        temp_path.rename_to(&path)
                    })?;
                    self.record_publication(object.digest, object.object_bytes);
                    sync_parent = true;
                }
                Err(error) => return Err(error),
            }
        }

        if sync_parent {
            sync_directory(&self.object_parent.path)?;
        }
        Ok(())
    }

    fn record_publication(&mut self, digest: [u8; 32], object_bytes: u64) {
        let state = self.objects.entry(digest).or_default();
        if !state.published {
            state.published = true;
            self.object_count = self.object_count.saturating_add(1);
            self.accounted_bytes = self.accounted_bytes.saturating_add(object_bytes);
        }
        if !state.preflight {
            state.preflight = true;
            self.preflight_object_count = self.preflight_object_count.saturating_add(1);
            self.preflight_accounted_bytes =
                self.preflight_accounted_bytes.saturating_add(object_bytes);
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_published_inventory_for_test(
        &mut self,
        count: usize,
        required_digest: Option<&str>,
    ) {
        self.objects.clear();
        for index in 0..count {
            let mut digest = [0_u8; 32];
            digest[24..].copy_from_slice(&u64::try_from(index).unwrap_or(u64::MAX).to_be_bytes());
            self.objects.insert(digest, SessionObjectState::PUBLISHED);
        }
        if let Some(required_digest) = required_digest {
            let digest = parse_session_object_digest(required_digest)
                .expect("test session object digest is valid");
            if !self.objects.contains_key(&digest) {
                self.objects.pop_first();
                self.objects.insert(digest, SessionObjectState::PUBLISHED);
            }
        }
        self.object_count = self.objects.len();
        self.preflight_object_count = self.object_count;
    }

    #[cfg(test)]
    pub(crate) fn seed_published_inventory_for_memory_test(&mut self) {
        self.seed_published_inventory_for_test(crate::runtime::types::MAX_SESSION_OBJECTS, None);
    }
}

fn parse_session_object_digest(value: &str) -> Result<[u8; 32], RuntimeError> {
    decode_lowercase_sha256_hex(value).ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "session object digest {value} is not lowercase SHA-256 hex"
        ))
    })
}

fn write_session_object(path: &AnchoredFile, bytes: &[u8]) -> Result<(), RuntimeError> {
    let mut file = open_anchored_session_log_append_file(path)?;
    file.write_all(bytes)
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    file.sync_all()
        .map_err(|source| path_io_error(path.diagnostic_path(), source))
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
