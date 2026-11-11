//! Test-only trace of state-changing filesystem steps for crash injection.
//!
//! A traced operation reports every create, rename, removal, directory creation, link and write
//! under its store root in order. Syncs are not steps: a dying process keeps everything already
//! written, so a sync never changes the image, and timer-driven flushes would make replays differ. Capturing a step copies the whole store before that step runs.
//! The copy is the image a process dying at that boundary leaves behind; the original operation
//! may keep running, and nothing it does afterwards reaches the image.

use super::test_path_key;
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

static TRACES: Mutex<Vec<ActiveTrace>> = Mutex::new(Vec::new());

struct ActiveTrace {
    capture_at: Option<usize>,
    effects: Option<Arc<AtomicUsize>>,
    image: Option<StoreImage>,
    root: PathBuf,
    steps: Vec<String>,
}

/// The ordered steps of one traced operation.
pub(crate) struct DurableTrace {
    pub(crate) image: Option<StoreImage>,
    pub(crate) steps: Vec<String>,
}

/// A complete copy of one store root with the external effect count observed at capture time.
#[derive(Clone, Debug)]
pub(crate) struct StoreImage {
    effects: usize,
    entries: Vec<ImageEntry>,
    root_mode: Option<u32>,
}

#[derive(Clone, Debug)]
enum ImageEntry {
    Directory {
        mode: u32,
        relative: PathBuf,
    },
    File {
        bytes: Vec<u8>,
        mode: u32,
        relative: PathBuf,
    },
}

impl StoreImage {
    pub(crate) fn effects(&self) -> usize {
        self.effects
    }
}

pub(crate) fn start_durable_trace(
    root: &Path,
    capture_at: Option<usize>,
    effects: Option<Arc<AtomicUsize>>,
) {
    TRACES
        .lock()
        .expect("durable trace seam is not poisoned")
        .push(ActiveTrace {
            capture_at,
            effects,
            image: None,
            root: test_path_key(root),
            steps: Vec::new(),
        });
}

pub(crate) fn finish_durable_trace(root: &Path) -> DurableTrace {
    let root = test_path_key(root);
    let mut traces = TRACES.lock().expect("durable trace seam is not poisoned");
    let index = traces
        .iter()
        .position(|trace| trace.root == root)
        .expect("durable trace is active for its root");
    let trace = traces.swap_remove(index);
    DurableTrace {
        image: trace.image,
        steps: trace.steps,
    }
}

/// Records one state-changing step under an active trace root.
pub(crate) fn durable_step(path: &Path, operation: &str) {
    let mut traces = TRACES.lock().expect("durable trace seam is not poisoned");
    if traces.is_empty() {
        return;
    }
    let key = test_path_key(path);
    let Some(trace) = traces.iter_mut().find(|trace| key.starts_with(&trace.root)) else {
        return;
    };
    // WHY: the whole-history validation scratch is a private per-command index, never a durable
    // Conversation format (PROTOCOL.md), so its steps do not change recoverable state.
    if key
        .strip_prefix(&trace.root)
        .is_ok_and(|relative| relative.starts_with("scratch"))
    {
        return;
    }
    let index = trace.steps.len();
    if trace.capture_at == Some(index) {
        trace.image = Some(capture(&trace.root, trace.effects.as_deref()));
    }
    let relative = key
        .strip_prefix(&trace.root)
        .expect("traced step is under its root");
    trace
        .steps
        .push(format!("{operation} {}", relative.display()));
}

/// Captures the store as it is now, with the external effect count at this moment.
pub(crate) fn capture_store_image(root: &Path, effects: &AtomicUsize) -> StoreImage {
    capture(root, Some(effects))
}

/// Replaces the store root with the captured image.
pub(crate) fn restore_store_image(root: &Path, image: &StoreImage) {
    match fs::remove_dir_all(root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("store image restore clears {}: {error}", root.display()),
    }
    let Some(root_mode) = image.root_mode else {
        return;
    };
    fs::create_dir_all(root).expect("store image restore recreates its root");
    set_mode(root, root_mode);
    for entry in &image.entries {
        match entry {
            ImageEntry::Directory { mode, relative } => {
                let path = root.join(relative);
                fs::create_dir(&path).expect("store image restore recreates a directory");
                set_mode(&path, *mode);
            }
            ImageEntry::File {
                bytes,
                mode,
                relative,
            } => {
                let path = root.join(relative);
                fs::write(&path, bytes).expect("store image restore recreates a file");
                set_mode(&path, *mode);
            }
        }
    }
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .expect("store image restore sets a mode");
}

fn capture(root: &Path, effects: Option<&AtomicUsize>) -> StoreImage {
    let mut entries = Vec::new();
    let root_mode = match fs::symlink_metadata(root) {
        Ok(metadata) => {
            collect(root, Path::new(""), &mut entries);
            Some(metadata.permissions().mode() & 0o7777)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("store image reads {}: {error}", root.display()),
    };
    StoreImage {
        effects: effects.map_or(0, |effects| effects.load(Ordering::SeqCst)),
        entries,
        root_mode,
    }
}

fn collect(root: &Path, relative: &Path, entries: &mut Vec<ImageEntry>) {
    let directory = root.join(relative);
    let mut children = fs::read_dir(&directory)
        .expect("store image lists a directory")
        .map(|entry| entry.expect("store image reads an entry"))
        .collect::<Vec<_>>();
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        let relative = relative.join(child.file_name());
        let path = child.path();
        let metadata = fs::symlink_metadata(&path).expect("store image stats an entry");
        let mode = metadata.permissions().mode() & 0o7777;
        if metadata.is_dir() {
            entries.push(ImageEntry::Directory {
                mode,
                relative: relative.clone(),
            });
            collect(root, &relative, entries);
        } else if metadata.is_file() {
            entries.push(ImageEntry::File {
                bytes: fs::read(&path).expect("store image reads a file"),
                mode,
                relative,
            });
        } else {
            panic!("store image supports only regular files and directories");
        }
    }
}
