#[cfg(any(test, feature = "m11-budget-evidence"))]
use super::super::contract::{MAX_CONVERSATION_SEGMENT_BYTES, protocol};
use crate::runtime::fs_guards::segmented_jsonl_leaf;
#[cfg(test)]
use crate::runtime::types::SessionStreamLimits;
#[cfg(any(test, feature = "m11-budget-evidence"))]
use crate::runtime::{
    fs_guards::{
        SegmentedJsonlLeaf, parse_segmented_jsonl_leaf, path_io_error, segmented_jsonl_leaf_stem,
    },
    types::RuntimeError,
};
#[cfg(any(test, feature = "m11-budget-evidence"))]
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(in crate::runtime::conversations) fn conversation_segment_inventory(
    base: &Path,
) -> Result<usize, RuntimeError> {
    let parent = base
        .parent()
        .ok_or_else(|| protocol("conversation stream has no parent directory"))?;
    let base_name = base
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| protocol("conversation stream name is not valid UTF-8"))?;
    let stem = segmented_jsonl_leaf_stem(base_name)
        .ok_or_else(|| protocol("conversation stream name must end in .jsonl"))?;
    let mut segment_count = 0usize;
    let mut last_ordinal = 0usize;
    for entry in fs::read_dir(parent).map_err(|source| path_io_error(parent, source))? {
        let entry = entry.map_err(|source| path_io_error(parent, source))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let ordinal = match parse_segmented_jsonl_leaf(&name, stem) {
            SegmentedJsonlLeaf::Ordinal(ordinal) if ordinal >= 2 || name == base_name => {
                usize::try_from(ordinal).ok()
            }
            SegmentedJsonlLeaf::Invalid => {
                return Err(protocol("conversation stream segment ordinal is invalid"));
            }
            SegmentedJsonlLeaf::Ordinal(_) | SegmentedJsonlLeaf::Unrelated => None,
        };
        let Some(ordinal) = ordinal else { continue };
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| path_io_error(&path, source))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(protocol("conversation stream segment must be a real file"));
        }
        if metadata.len() > MAX_CONVERSATION_SEGMENT_BYTES {
            return Err(protocol(
                "conversation stream segment exceeds its byte limit",
            ));
        }
        segment_count = segment_count.saturating_add(1);
        last_ordinal = last_ordinal.max(ordinal);
    }
    if last_ordinal == 0 {
        return Err(RuntimeError::Io {
            path: base.to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
    }
    if segment_count != last_ordinal {
        return Err(protocol(
            "conversation stream segment ordinals are not contiguous",
        ));
    }
    Ok(last_ordinal)
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(in crate::runtime::conversations) fn conversation_segment_path_for_ordinal(
    base: &Path,
    ordinal: usize,
) -> Result<PathBuf, RuntimeError> {
    if ordinal == 1 {
        Ok(base.to_owned())
    } else {
        conversation_segment_path(base, ordinal)
    }
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(super) fn conversation_segment_path(
    base: &Path,
    ordinal: usize,
) -> Result<PathBuf, RuntimeError> {
    if ordinal < 2 {
        return Err(protocol("conversation stream segment ordinal is exhausted"));
    }
    let stem = base
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(segmented_jsonl_leaf_stem)
        .ok_or_else(|| protocol("conversation stream name must end in .jsonl"))?;
    let leaf = segmented_jsonl_leaf(stem, u64::try_from(ordinal).unwrap_or(u64::MAX))
        .ok_or_else(|| protocol("conversation stream segment ordinal is exhausted"))?;
    Ok(base.with_file_name(leaf))
}

#[cfg(test)]
fn run_segment_count(run: &Path, stem: &str, max_segments: u64) -> Result<usize, RuntimeError> {
    let base = segmented_jsonl_leaf(stem, 1).expect("first segment ordinal is valid");
    let mut base_present = false;
    let mut segment_count = 0usize;
    let mut max_ordinal = 0usize;
    for entry in fs::read_dir(run).map_err(|source| path_io_error(run, source))? {
        let entry = entry.map_err(|source| path_io_error(run, source))?;
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| path_io_error(&path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(protocol("run stream inventory must not contain symlinks"));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("run stream name must be UTF-8"))?;
        let ordinal = if name == base {
            if !metadata.is_file() {
                return Err(protocol("run base stream must be a real file"));
            }
            base_present = true;
            1
        } else {
            let SegmentedJsonlLeaf::Ordinal(ordinal) = parse_segmented_jsonl_leaf(&name, stem)
            else {
                continue;
            };
            let Some(ordinal) = usize::try_from(ordinal).ok() else {
                continue;
            };
            if !metadata.is_file() {
                return Err(protocol("run stream segment must be a real file"));
            }
            ordinal
        };
        segment_count = segment_count.saturating_add(1);
        max_ordinal = max_ordinal.max(ordinal);
        if u64::try_from(segment_count).unwrap_or(u64::MAX) > max_segments {
            return Err(protocol(format!(
                "productive recovery {stem} prefix has too many segments"
            )));
        }
    }
    if !base_present {
        return Err(protocol(format!("run {stem} stream is missing")));
    }
    if segment_count != max_ordinal {
        return Err(protocol("run stream segment ordinals are not contiguous"));
    }
    Ok(segment_count)
}

pub(in crate::runtime::conversations) fn run_segment_leaf(stem: &str, index: usize) -> String {
    segmented_jsonl_leaf(
        stem,
        u64::try_from(index.saturating_add(1)).unwrap_or(u64::MAX),
    )
    .expect("run segment count stays within its stream limit")
}

#[cfg(test)]
pub(crate) fn target_segment_count_for_test(
    run: &Path,
    stem: &str,
    limits: SessionStreamLimits,
) -> Result<usize, RuntimeError> {
    run_segment_count(run, stem, limits.max_segments)
}
