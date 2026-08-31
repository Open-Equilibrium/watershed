use super::super::contract::protocol;
use crate::runtime::{
    digest::{is_lowercase_sha256_hex, sha256_hex},
    fs_guards::{
        AnchoredDir, AnchoredDirectoryIdentity, AnchoredFile, DirectoryErrorMode,
        create_anchored_file_for_update, open_anchored_file_for_read,
        open_anchored_file_for_update, path_io_error, read_anchored_file_with_limit,
        verify_owned_anchored_file,
    },
    session_authority::conversation_history_validation_dir,
    stage_results::reconcile_operation_and_cleanup,
    types::RuntimeError,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};
use std::{
    fs::{self, File},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const INDEX_SCHEMA: &str = "flow-conversation-history-validation-v1";
const INDEX_MARKER_LIMIT: u64 = 4096;
const LEASE_LEAF: &str = "lease";
const MARKER_LEAF: &str = "marker.json";
const INDEX_RUN_PREFIX: &str = "g";
const EVENT_POINTER_RUN_PREFIX: &str = "pointer-";
const EVENT_IDENTIFIER_RUN_PREFIX: &str = "event-ident-";
const SCRATCH_RUN_SUFFIX: &str = ".bin";
const SCRATCH_SUFFIX: &str = ".scratch";
const GENERATION_WIDTH: usize = 3;
const RUN_WIDTH: usize = 16;
pub(super) const INDEX_WORK_RESERVE: u64 = 16 * 1024 * 1024;
static INDEX_NONCE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryScratchStage {
    ActiveScratchSkipped,
    DirectoryCreated,
    RootLeaseContended,
    StaleSweep,
    UnlockedForRemoval,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryScratchFault {
    InitializationAfterDirectory,
    InitializationAfterMarker,
    CleanupAfterLeaseRemoval,
    CleanupAfterMarkerRemoval,
}

#[cfg(test)]
type HistoryScratchStageObserver = Box<dyn FnMut(HistoryScratchStage)>;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryScratchMemberStage {
    AfterRemoval,
    BeforeInspection,
    BeforeRemoval,
}

#[cfg(test)]
type HistoryScratchMemberObserver = Box<dyn FnMut(HistoryScratchMemberStage, &str)>;

#[cfg(test)]
thread_local! {
    static AVAILABLE_SPACE_OVERRIDE: Cell<Option<u64>> = const { Cell::new(None) };
    static HISTORY_SCRATCH_FAULT: Cell<Option<HistoryScratchFault>> = const { Cell::new(None) };
    static HISTORY_SCRATCH_STAGE_OBSERVER: RefCell<Option<HistoryScratchStageObserver>> =
        RefCell::new(None);
    static HISTORY_SCRATCH_MEMBER_OBSERVER: RefCell<Option<HistoryScratchMemberObserver>> =
        RefCell::new(None);
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScratchMarker {
    schema: String,
    leaf: String,
    device: u64,
    inode: u64,
}

pub(super) struct HistoryScratch {
    pub(super) root: AnchoredDir,
    pub(super) leaf: String,
    pub(super) dir: AnchoredDir,
    pub(super) identity: AnchoredDirectoryIdentity,
    pub(super) lease: File,
    pub(super) current: u64,
    #[cfg(test)]
    pub(super) peak: u64,
    pub(super) limit: u64,
}

struct HistoryScratchRootLease {
    _file: File,
}

impl HistoryScratch {
    pub(super) fn create(
        workspace: &Path,
        conversation_id: &str,
        limit: u64,
    ) -> Result<Self, RuntimeError> {
        let root = conversation_history_validation_dir(workspace, true)?
            .expect("created history validation root exists");
        let _root_lease = HistoryScratchRootLease::acquire(&root)?;
        cleanup_stale_scratch(&root)?;
        Self::create_in_root(root, conversation_id, limit)
    }

    pub(super) fn create_in_root(
        root: AnchoredDir,
        conversation_id: &str,
        limit: u64,
    ) -> Result<Self, RuntimeError> {
        if available_space(&root.path)? < limit {
            return Err(protocol(
                "insufficient space for conversation history validation scratch",
            ));
        }
        let leaf = scratch_leaf(conversation_id);
        let dir = root
            .private_child(&leaf, true, DirectoryErrorMode::Protocol)?
            .expect("created history validation scratch exists");
        #[cfg(test)]
        observe_history_scratch_stage(HistoryScratchStage::DirectoryCreated);
        let initialization = (|| {
            #[cfg(test)]
            history_scratch_fault(HistoryScratchFault::InitializationAfterDirectory, &dir.path)?;
            let identity = dir.identity()?;
            let marker = ScratchMarker {
                schema: INDEX_SCHEMA.to_owned(),
                leaf: leaf.clone(),
                device: identity.device,
                inode: identity.inode,
            };
            let marker_text = format!(
                "{}\n",
                serde_json::to_string(&marker)
                    .map_err(|error| protocol(format!("history scratch marker failed: {error}")))?
            );
            let marker_path = dir.file(MARKER_LEAF);
            let mut marker_file = create_scratch_file(&dir, MARKER_LEAF)?;
            marker_file
                .write_all(marker_text.as_bytes())
                .and_then(|()| marker_file.sync_all())
                .map_err(|source| path_io_error(marker_path.diagnostic_path(), source))?;
            #[cfg(test)]
            history_scratch_fault(HistoryScratchFault::InitializationAfterMarker, &dir.path)?;
            let lease = create_scratch_file(&dir, LEASE_LEAF)?;
            lease.try_lock().map_err(|error| match error {
                fs::TryLockError::WouldBlock => protocol("history validation scratch is active"),
                fs::TryLockError::Error(source) => {
                    path_io_error(&dir.file(LEASE_LEAF).path, source)
                }
            })?;
            let marker_bytes = marker_text.len() as u64;
            if marker_bytes > limit {
                return Err(protocol("conversation history scratch exceeds its budget"));
            }
            Ok((identity, lease, marker_bytes))
        })();
        match initialization {
            Ok((identity, lease, marker_bytes)) => Ok(Self {
                root,
                leaf,
                dir,
                identity,
                lease,
                current: marker_bytes,
                #[cfg(test)]
                peak: marker_bytes,
                limit,
            }),
            Err(operation) => reconcile_operation_and_cleanup(
                Err(operation),
                rollback_scratch_initialization(&root, &leaf, dir),
            ),
        }
    }

    pub(super) fn write(
        &mut self,
        file: &mut File,
        path: &AnchoredFile,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let next = self
            .current
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| protocol("conversation history scratch count overflow"))?;
        if next > self.limit {
            return Err(protocol("conversation history scratch exceeds its budget"));
        }
        file.write_all(bytes)
            .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
        self.current = next;
        #[cfg(test)]
        {
            self.peak = self.peak.max(next);
        }
        Ok(())
    }

    pub(super) fn remove_file(&mut self, leaf: &str) -> Result<(), RuntimeError> {
        let path = self.dir.file(leaf);
        #[cfg(test)]
        observe_history_scratch_member_stage(HistoryScratchMemberStage::BeforeRemoval, leaf);
        let length = path.metadata()?.len();
        path.remove()?;
        #[cfg(test)]
        observe_history_scratch_member_stage(HistoryScratchMemberStage::AfterRemoval, leaf);
        self.current = self
            .current
            .checked_sub(length)
            .ok_or_else(|| protocol("conversation history scratch accounting underflow"))?;
        Ok(())
    }

    pub(super) fn cleanup(self) -> Result<(), RuntimeError> {
        let Self {
            root,
            leaf,
            dir,
            identity,
            lease,
            ..
        } = self;
        let _root_lease = HistoryScratchRootLease::acquire(&root)?;
        lease
            .unlock()
            .map_err(|source| path_io_error(&dir.file(LEASE_LEAF).path, source))?;
        #[cfg(test)]
        observe_history_scratch_stage(HistoryScratchStage::UnlockedForRemoval);
        drop(lease);
        drop(dir);
        remove_scratch_dir(&root, &leaf, identity)
    }
}

impl HistoryScratchRootLease {
    pub(super) fn acquire(root: &AnchoredDir) -> Result<Self, RuntimeError> {
        let path = root.file(LEASE_LEAF);
        let file = match create_scratch_file(root, LEASE_LEAF) {
            Ok(file) => file,
            Err(RuntimeError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                open_scratch_file_for_update(&path)?.0
            }
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                #[cfg(test)]
                observe_history_scratch_stage(HistoryScratchStage::RootLeaseContended);
                file.lock()
                    .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            }
            Err(fs::TryLockError::Error(source)) => {
                return Err(path_io_error(path.diagnostic_path(), source));
            }
        }
        verify_owned_anchored_file(&path, &file, "history validation root lease")?;
        Ok(Self { _file: file })
    }
}

pub(super) fn create_scratch_file(dir: &AnchoredDir, leaf: &str) -> Result<File, RuntimeError> {
    create_anchored_file_for_update(&dir.file(leaf))
}

pub(super) fn write_sorted_scratch_run<T: AsRef<[u8]>>(
    scratch: &mut HistoryScratch,
    chunk: &mut Vec<T>,
    leaf: &str,
) -> Result<(), RuntimeError> {
    let path = scratch.dir.file(leaf);
    let mut file = create_scratch_file(&scratch.dir, leaf)?;
    for record in chunk.iter() {
        scratch.write(&mut file, &path, record.as_ref())?;
    }
    file.sync_all()
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    chunk.clear();
    Ok(())
}

pub(super) fn cleanup_stale_scratch(root: &AnchoredDir) -> Result<(), RuntimeError> {
    for_each_scratch_leaf(root, |leaf| {
        let _ = validate_inactive_scratch(root, leaf)?;
        Ok(())
    })?;
    for_each_scratch_leaf(root, |leaf| {
        #[cfg(test)]
        observe_history_scratch_stage(HistoryScratchStage::StaleSweep);
        if let Some(identity) = validate_inactive_scratch(root, leaf)? {
            remove_scratch_dir(root, leaf, identity)?;
        }
        Ok(())
    })
}

fn for_each_scratch_leaf(
    root: &AnchoredDir,
    mut operation: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    for entry in root
        .dir
        .entries()
        .map_err(|source| path_io_error(&root.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&root.path, source))?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("conversation history validation scratch name is not UTF-8"))?;
        if leaf == LEASE_LEAF {
            continue;
        }
        if !valid_scratch_leaf(&leaf) {
            return Err(protocol(format!(
                "{} contains unsafe history validation scratch",
                root.path.display()
            )));
        }
        operation(&leaf)?;
    }
    Ok(())
}

fn validate_inactive_scratch(
    root: &AnchoredDir,
    leaf: &str,
) -> Result<Option<AnchoredDirectoryIdentity>, RuntimeError> {
    let dir = root
        .private_child(leaf, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("history validation scratch disappeared"))?;
    let identity = dir.identity()?;
    let members = scratch_members(&dir)?;
    if members.count == 0 {
        return Ok(Some(identity));
    }
    if !members.marker {
        return Err(protocol("history validation scratch is missing its marker"));
    }
    let marker = read_marker(&dir)?;
    if marker.leaf != leaf || marker.device != identity.device || marker.inode != identity.inode {
        return Err(protocol("history validation scratch identity is invalid"));
    }
    if !members.lease {
        if members.count != 1 {
            return Err(protocol("history validation scratch is missing its lease"));
        }
        return Ok(Some(identity));
    }
    let lease_path = dir.file(LEASE_LEAF);
    let (lease, _) = open_scratch_file_for_update(&lease_path)?;
    match lease.try_lock() {
        Ok(()) => {
            let validation = validate_scratch_members(&dir);
            let release = lease
                .unlock()
                .map_err(|source| path_io_error(lease_path.diagnostic_path(), source));
            reconcile_operation_and_cleanup(validation, release)?;
            Ok(Some(identity))
        }
        Err(fs::TryLockError::WouldBlock) => {
            #[cfg(test)]
            observe_history_scratch_stage(HistoryScratchStage::ActiveScratchSkipped);
            Ok(None)
        }
        Err(fs::TryLockError::Error(source)) => {
            Err(path_io_error(lease_path.diagnostic_path(), source))
        }
    }
}

#[derive(Default)]
struct ScratchMembers {
    count: usize,
    lease: bool,
    marker: bool,
}

fn scratch_members(dir: &AnchoredDir) -> Result<ScratchMembers, RuntimeError> {
    let mut members = ScratchMembers::default();
    for_each_scratch_member(dir, |member, _file| {
        members.count += 1;
        members.lease |= member == LEASE_LEAF;
        members.marker |= member == MARKER_LEAF;
        Ok(())
    })?;
    Ok(members)
}

fn validate_scratch_members(dir: &AnchoredDir) -> Result<(), RuntimeError> {
    for_each_scratch_member(dir, |_member, file| {
        #[cfg(test)]
        observe_history_scratch_member_stage(HistoryScratchMemberStage::BeforeInspection, _member);
        let _ = open_anchored_file_for_read(file)?;
        Ok(())
    })
}

fn for_each_scratch_member(
    dir: &AnchoredDir,
    mut operation: impl FnMut(&str, &AnchoredFile) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    for entry in dir
        .dir
        .entries()
        .map_err(|source| path_io_error(&dir.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&dir.path, source))?;
        let member = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("history validation scratch member is not UTF-8"))?;
        if !valid_scratch_member(&member) {
            return Err(protocol(
                "history validation scratch contains foreign bytes",
            ));
        }
        operation(&member, &dir.file(&member))?;
    }
    Ok(())
}

fn read_marker(dir: &AnchoredDir) -> Result<ScratchMarker, RuntimeError> {
    let bytes = read_anchored_file_with_limit(&dir.file(MARKER_LEAF), INDEX_MARKER_LIMIT)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| protocol("history validation scratch marker is not UTF-8"))?;
    let line = text
        .strip_suffix('\n')
        .ok_or_else(|| protocol("history validation scratch marker framing is invalid"))?;
    let marker: ScratchMarker = serde_json::from_str(line)
        .map_err(|_| protocol("history validation scratch marker is invalid"))?;
    if marker.schema != INDEX_SCHEMA {
        return Err(protocol(
            "history validation scratch marker schema is invalid",
        ));
    }
    Ok(marker)
}

fn remove_scratch_marker(dir: &AnchoredDir) -> Result<(), RuntimeError> {
    let marker = dir.file(MARKER_LEAF);
    marker.remove()?;
    #[cfg(test)]
    history_scratch_fault(
        HistoryScratchFault::CleanupAfterMarkerRemoval,
        marker.diagnostic_path(),
    )?;
    Ok(())
}

fn rollback_scratch_initialization(
    root: &AnchoredDir,
    leaf: &str,
    dir: AnchoredDir,
) -> Result<(), RuntimeError> {
    let expected = dir.identity()?;
    if dir.identity()? != expected {
        return Err(protocol(
            "history validation scratch identity changed during initialization",
        ));
    }
    for_each_scratch_member(&dir, |member, file| {
        if !matches!(member, MARKER_LEAF | LEASE_LEAF) {
            return Err(protocol(
                "history validation scratch changed during initialization",
            ));
        }
        let _ = open_anchored_file_for_read(file)?;
        file.remove()
    })?;
    remove_empty_scratch_dir(root, leaf, dir, expected)
}

fn remove_scratch_dir(
    root: &AnchoredDir,
    leaf: &str,
    expected: AnchoredDirectoryIdentity,
) -> Result<(), RuntimeError> {
    let dir = root
        .private_child(leaf, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("history validation scratch disappeared before cleanup"))?;
    if dir.identity()? != expected {
        return Err(protocol(
            "history validation scratch identity changed before cleanup",
        ));
    }
    let members = scratch_members(&dir)?;
    if members.count == 0 {
        return remove_empty_scratch_dir(root, leaf, dir, expected);
    }
    if !members.marker {
        return Err(protocol(
            "history validation scratch is missing its marker during cleanup",
        ));
    }
    let marker = read_marker(&dir)?;
    if marker.leaf != leaf || marker.device != expected.device || marker.inode != expected.inode {
        return Err(protocol(
            "history validation scratch marker identity changed",
        ));
    }
    if !members.lease {
        if members.count != 1 {
            return Err(protocol(
                "history validation scratch is missing its lease during cleanup",
            ));
        }
        remove_scratch_marker(&dir)?;
        return remove_empty_scratch_dir(root, leaf, dir, expected);
    }
    let lease_path = dir.file(LEASE_LEAF);
    let (lease, _) = open_scratch_file_for_update(&lease_path)?;
    match lease.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            #[cfg(test)]
            observe_history_scratch_stage(HistoryScratchStage::ActiveScratchSkipped);
            return Ok(());
        }
        Err(fs::TryLockError::Error(source)) => {
            return Err(path_io_error(lease_path.diagnostic_path(), source));
        }
    }
    let removal = (|| {
        for_each_scratch_member(&dir, |member, file| {
            if matches!(member, MARKER_LEAF | LEASE_LEAF) {
                return Ok(());
            }
            let _ = open_anchored_file_for_read(file)?;
            file.remove()?;
            Ok(())
        })?;
        if dir.identity()? != expected {
            return Err(protocol(
                "history validation scratch identity changed during cleanup",
            ));
        }
        Ok(())
    })();
    let release = lease
        .unlock()
        .map_err(|source| path_io_error(lease_path.diagnostic_path(), source));
    reconcile_operation_and_cleanup(removal, release)?;
    drop(lease);
    lease_path.remove()?;
    #[cfg(test)]
    history_scratch_fault(
        HistoryScratchFault::CleanupAfterLeaseRemoval,
        lease_path.diagnostic_path(),
    )?;
    if dir.identity()? != expected {
        return Err(protocol(
            "history validation scratch identity changed after cleanup",
        ));
    }
    let remaining = scratch_members(&dir)?;
    if remaining.count != 1 || !remaining.marker || remaining.lease {
        return Err(protocol(
            "history validation scratch changed before removal",
        ));
    }
    let marker = read_marker(&dir)?;
    if marker.leaf != leaf || marker.device != expected.device || marker.inode != expected.inode {
        return Err(protocol(
            "history validation scratch marker identity changed before removal",
        ));
    }
    remove_scratch_marker(&dir)?;
    drop(lease_path);
    remove_empty_scratch_dir(root, leaf, dir, expected)
}

fn remove_empty_scratch_dir(
    root: &AnchoredDir,
    leaf: &str,
    dir: AnchoredDir,
    expected: AnchoredDirectoryIdentity,
) -> Result<(), RuntimeError> {
    if dir.identity()? != expected {
        return Err(protocol(
            "history validation scratch identity changed after cleanup",
        ));
    }
    if dir
        .dir
        .entries()
        .map_err(|source| path_io_error(&dir.path, source))?
        .next()
        .transpose()
        .map_err(|source| path_io_error(&dir.path, source))?
        .is_some()
    {
        return Err(protocol(
            "history validation scratch changed before removal",
        ));
    }
    drop(dir);
    let rebound = root
        .private_child(leaf, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("history validation scratch disappeared before removal"))?;
    if rebound.identity()? != expected {
        return Err(protocol(
            "history validation scratch was replaced before removal",
        ));
    }
    drop(rebound);
    root.dir
        .remove_dir(leaf)
        .map_err(|source| path_io_error(&root.path.join(leaf), source))
}

fn valid_scratch_leaf(leaf: &str) -> bool {
    leaf.strip_suffix(SCRATCH_SUFFIX)
        .is_some_and(is_lowercase_sha256_hex)
}

fn valid_scratch_member(member: &str) -> bool {
    member == MARKER_LEAF
        || member == LEASE_LEAF
        || valid_scratch_run_member(member)
        || member
            .strip_prefix(EVENT_POINTER_RUN_PREFIX)
            .is_some_and(valid_scratch_run_member)
        || member
            .strip_prefix(EVENT_IDENTIFIER_RUN_PREFIX)
            .and_then(|pass| pass.strip_suffix(SCRATCH_RUN_SUFFIX))
            .is_some_and(|pass| matches!(pass.as_bytes(), [b'0'..=b'2']))
}

fn valid_scratch_run_member(member: &str) -> bool {
    member
        .strip_prefix(INDEX_RUN_PREFIX)
        .and_then(|body| body.strip_suffix(SCRATCH_RUN_SUFFIX))
        .and_then(|body| body.split_once("-r"))
        .is_some_and(|(generation, run)| {
            generation.len() == GENERATION_WIDTH
                && generation.bytes().all(|byte| byte.is_ascii_digit())
                && run.len() == RUN_WIDTH
                && run.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub(super) fn index_run_leaf(generation: u32, run: u64) -> String {
    format!(
        "{INDEX_RUN_PREFIX}{generation:0generation_width$}-r{run:0run_width$}{SCRATCH_RUN_SUFFIX}",
        generation_width = GENERATION_WIDTH,
        run_width = RUN_WIDTH,
    )
}

pub(super) fn event_pointer_run_leaf(generation: u32, run: u64) -> String {
    format!(
        "{EVENT_POINTER_RUN_PREFIX}{}",
        index_run_leaf(generation, run)
    )
}

pub(super) fn event_identifier_run_leaf(pass: u8) -> String {
    format!("{EVENT_IDENTIFIER_RUN_PREFIX}{pass}{SCRATCH_RUN_SUFFIX}")
}

fn scratch_leaf(conversation_id: &str) -> String {
    let nonce = INDEX_NONCE.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let key = format!(
        "{INDEX_SCHEMA}\0{}\0{conversation_id}\0{now}\0{nonce}",
        std::process::id()
    );
    format!("{}{SCRATCH_SUFFIX}", sha256_hex(key.as_bytes()))
}

#[cfg(test)]
pub(crate) fn set_history_index_available_space_for_test(bytes: Option<u64>) {
    AVAILABLE_SPACE_OVERRIDE.with(|slot| slot.set(bytes));
}

#[cfg(test)]
pub(crate) fn set_history_scratch_fault_for_test(fault: Option<HistoryScratchFault>) {
    HISTORY_SCRATCH_FAULT.with(|slot| slot.set(fault));
}

#[cfg(test)]
pub(crate) fn history_validation_dir_path_for_test(
    workspace: &Path,
) -> Result<PathBuf, RuntimeError> {
    conversation_history_validation_dir(workspace, true)?
        .map(|dir| dir.path)
        .ok_or_else(|| protocol("history validation directory is unavailable"))
}

#[cfg(test)]
pub(crate) fn abandon_history_index_scratch_for_test(
    workspace: &Path,
    conversation_id: &str,
) -> Result<PathBuf, RuntimeError> {
    abandon_history_index_scratches_for_test(workspace, conversation_id, 1).map(|mut paths| {
        paths
            .pop()
            .expect("one abandoned history validation scratch is created")
    })
}

#[cfg(test)]
pub(crate) fn abandon_history_index_scratches_for_test(
    workspace: &Path,
    conversation_id: &str,
    count: usize,
) -> Result<Vec<PathBuf>, RuntimeError> {
    let root = conversation_history_validation_dir(workspace, true)?
        .expect("created history validation root exists");
    let _root_lease = HistoryScratchRootLease::acquire(&root)?;
    cleanup_stale_scratch(&root)?;
    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        let scratch = HistoryScratch::create_in_root(
            root.clone(),
            &format!("{conversation_id}-{index}"),
            INDEX_WORK_RESERVE,
        )?;
        paths.push(scratch.dir.path.clone());
        scratch
            .lease
            .unlock()
            .map_err(|source| path_io_error(&scratch.dir.file(LEASE_LEAF).path, source))?;
    }
    Ok(paths)
}

#[cfg(test)]
pub(crate) fn complete_history_index_scratch_for_test(
    workspace: &Path,
    conversation_id: &str,
) -> Result<(), RuntimeError> {
    HistoryScratch::create(workspace, conversation_id, INDEX_WORK_RESERVE)?.cleanup()
}

#[cfg(test)]
pub(crate) fn with_history_scratch_stage_observer_for_test<T>(
    observer: impl FnMut(HistoryScratchStage) + 'static,
    operation: impl FnOnce() -> T,
) -> T {
    HISTORY_SCRATCH_STAGE_OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(observer));
    });
    let result = operation();
    HISTORY_SCRATCH_STAGE_OBSERVER.with(|slot| {
        *slot.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
pub(crate) fn with_history_scratch_member_observer_for_test<T>(
    observer: impl FnMut(HistoryScratchMemberStage, &str) + 'static,
    operation: impl FnOnce() -> T,
) -> T {
    HISTORY_SCRATCH_MEMBER_OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(observer));
    });
    let result = operation();
    HISTORY_SCRATCH_MEMBER_OBSERVER.with(|slot| {
        *slot.borrow_mut() = None;
    });
    result
}

#[cfg(test)]
fn observe_history_scratch_stage(stage: HistoryScratchStage) {
    HISTORY_SCRATCH_STAGE_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().as_mut() {
            observer(stage);
        }
    });
}

#[cfg(test)]
fn history_scratch_fault(stage: HistoryScratchFault, path: &Path) -> Result<(), RuntimeError> {
    HISTORY_SCRATCH_FAULT.with(|slot| {
        if slot.get() == Some(stage) {
            slot.set(None);
            return Err(path_io_error(
                path,
                std::io::Error::other("injected history scratch lifecycle failure"),
            ));
        }
        Ok(())
    })
}

#[cfg(test)]
fn observe_history_scratch_member_stage(stage: HistoryScratchMemberStage, member: &str) {
    HISTORY_SCRATCH_MEMBER_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().as_mut() {
            observer(stage, member);
        }
    });
}

fn open_scratch_file_for_update(path: &AnchoredFile) -> Result<(File, fs::Metadata), RuntimeError> {
    open_anchored_file_for_update(path)
}

#[cfg(unix)]
fn available_space(path: &Path) -> Result<u64, RuntimeError> {
    #[cfg(test)]
    if let Some(bytes) = AVAILABLE_SPACE_OVERRIDE.with(Cell::get) {
        return Ok(bytes);
    }
    let stat = rustix::fs::statvfs(path).map_err(|source| {
        path_io_error(
            path,
            std::io::Error::from_raw_os_error(source.raw_os_error()),
        )
    })?;
    stat.f_bavail
        .checked_mul(stat.f_frsize)
        .ok_or_else(|| protocol("available scratch space overflow"))
}

#[cfg(windows)]
fn available_space(path: &Path) -> Result<u64, RuntimeError> {
    #[cfg(test)]
    if let Some(bytes) = AVAILABLE_SPACE_OVERRIDE.with(Cell::get) {
        return Ok(bytes);
    }
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        Err(path_io_error(path, std::io::Error::last_os_error()))
    } else {
        Ok(available)
    }
}

#[cfg(not(any(unix, windows)))]
fn available_space(_path: &Path) -> Result<u64, RuntimeError> {
    Err(protocol(
        "history validation scratch space cannot be admitted on this platform",
    ))
}
