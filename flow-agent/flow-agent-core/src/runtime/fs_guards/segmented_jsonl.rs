use super::{
    AnchoredFile, MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits,
    ensure_anchored_real_file, for_each_anchored_file_line_with_limit, path_io_error,
};
#[cfg(test)]
use std::cell::Cell;

const JSONL_SUFFIX: &str = ".jsonl";
const MAX_SEGMENT_ORDINAL: u64 = 999_999;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SegmentedJsonlDiscoveryMetrics {
    pub passes: usize,
    pub retained_path_peak: usize,
}

#[cfg(test)]
thread_local! {
    static DISCOVERY_METRICS: Cell<Option<SegmentedJsonlDiscoveryMetrics>> = const { Cell::new(None) };
}

#[cfg(test)]
fn record_discovery_pass() {
    DISCOVERY_METRICS.with(|slot| {
        if let Some(mut metrics) = slot.get() {
            metrics.passes = metrics.passes.saturating_add(1);
            slot.set(Some(metrics));
        }
    });
}

#[cfg(test)]
fn record_retained_paths(retained: usize) {
    DISCOVERY_METRICS.with(|slot| {
        if let Some(mut metrics) = slot.get() {
            metrics.retained_path_peak = metrics.retained_path_peak.max(retained);
            slot.set(Some(metrics));
        }
    });
}

#[cfg(test)]
pub fn with_segmented_jsonl_discovery_metrics_for_test<T>(
    operation: impl FnOnce() -> T,
) -> (T, SegmentedJsonlDiscoveryMetrics) {
    DISCOVERY_METRICS.with(|slot| {
        let previous = slot.replace(Some(SegmentedJsonlDiscoveryMetrics::default()));
        let result = operation();
        let metrics = slot
            .replace(previous)
            .expect("segmented JSONL discovery metrics are active");
        (result, metrics)
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentedJsonlLeaf {
    Unrelated,
    Invalid,
    Ordinal(u64),
}

pub fn segmented_jsonl_leaf(stem: &str, ordinal: u64) -> Option<String> {
    match ordinal {
        1 => Some(format!("{stem}{JSONL_SUFFIX}")),
        2..=MAX_SEGMENT_ORDINAL => Some(format!("{stem}.{ordinal:06}{JSONL_SUFFIX}")),
        _ => None,
    }
}

pub fn is_segmented_jsonl_ordinal(ordinal: u64) -> bool {
    (1..=MAX_SEGMENT_ORDINAL).contains(&ordinal)
}

pub fn segmented_jsonl_leaf_stem(name: &str) -> Option<&str> {
    name.strip_suffix(JSONL_SUFFIX)
}

pub fn parse_segmented_jsonl_leaf(name: &str, stem: &str) -> SegmentedJsonlLeaf {
    if name == segmented_jsonl_leaf(stem, 1).expect("first segment ordinal is valid") {
        return SegmentedJsonlLeaf::Ordinal(1);
    }
    let Some(ordinal) = name
        .strip_prefix(stem)
        .and_then(|suffix| suffix.strip_prefix('.'))
        .and_then(|suffix| suffix.strip_suffix(JSONL_SUFFIX))
    else {
        return SegmentedJsonlLeaf::Unrelated;
    };
    if ordinal.len() != 6 || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
        return SegmentedJsonlLeaf::Invalid;
    }
    match ordinal
        .parse::<u64>()
        .ok()
        .filter(|ordinal| *ordinal <= MAX_SEGMENT_ORDINAL)
    {
        Some(ordinal) => SegmentedJsonlLeaf::Ordinal(ordinal),
        None => SegmentedJsonlLeaf::Invalid,
    }
}

pub fn segmented_jsonl_path(
    base: &AnchoredFile,
    ordinal: u64,
) -> Result<AnchoredFile, RuntimeError> {
    if ordinal == 1 {
        return Ok(base.clone());
    }
    let leaf = segmented_jsonl_stem(base)?;
    let leaf = segmented_jsonl_leaf(leaf, ordinal)
        .ok_or_else(|| RuntimeError::Protocol("segmented JSONL ordinal is exhausted".to_owned()))?;
    Ok(base.parent.file(leaf))
}

pub fn segmented_jsonl_stem(base: &AnchoredFile) -> Result<&str, RuntimeError> {
    let stem = base
        .leaf
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .and_then(segmented_jsonl_leaf_stem)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} segmented JSONL path must end in .jsonl",
                base.diagnostic_path().display()
            ))
        })?;
    if !stem.is_ascii() {
        return Err(RuntimeError::Protocol(format!(
            "{} segmented JSONL stem must be ASCII",
            base.diagnostic_path().display()
        )));
    }
    Ok(stem)
}

pub enum SegmentedJsonlMember {
    Canonical(u64, AnchoredFile),
    Alias(AnchoredFile),
}

pub fn for_each_segmented_jsonl_member(
    base: &AnchoredFile,
    mut visit: impl FnMut(SegmentedJsonlMember) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    #[cfg(test)]
    record_discovery_pass();
    let leaf = segmented_jsonl_stem(base)?;
    let base_name = segmented_jsonl_leaf(leaf, 1).expect("first segment ordinal is valid");
    let folded_leaf = leaf.to_ascii_lowercase();
    let folded_base_name =
        segmented_jsonl_leaf(&folded_leaf, 1).expect("first segment ordinal is valid");
    for entry in base
        .parent
        .dir
        .entries()
        .map_err(|source| path_io_error(&base.parent.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&base.parent.path, source))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let candidate = name.to_ascii_lowercase();
        if candidate == folded_base_name {
            if name != base_name {
                visit(SegmentedJsonlMember::Alias(base.parent.file(name)))?;
            }
            continue;
        }
        let ordinal = match parse_segmented_jsonl_leaf(&candidate, &folded_leaf) {
            SegmentedJsonlLeaf::Ordinal(ordinal) => ordinal,
            SegmentedJsonlLeaf::Invalid => {
                return Err(RuntimeError::Protocol(format!(
                    "{} contains malformed segmented JSONL name {name}",
                    base.parent.path.display()
                )));
            }
            SegmentedJsonlLeaf::Unrelated => continue,
        };
        let file = base.parent.file(name);
        let canonical = format!("{leaf}.{ordinal:06}{JSONL_SUFFIX}");
        visit(if name == canonical {
            SegmentedJsonlMember::Canonical(ordinal, file)
        } else {
            SegmentedJsonlMember::Alias(file)
        })?;
    }
    Ok(())
}

pub fn canonical_segmented_jsonl_sibling(
    base: &AnchoredFile,
    member: SegmentedJsonlMember,
) -> Result<(u64, AnchoredFile), RuntimeError> {
    match member {
        SegmentedJsonlMember::Canonical(ordinal, file) => Ok((ordinal, file)),
        SegmentedJsonlMember::Alias(file) => Err(RuntimeError::Protocol(format!(
            "{} contains non-canonical segmented JSONL name {}",
            base.parent.path.display(),
            file.leaf.display()
        ))),
    }
}

pub fn retry_event_segment_discovery<T>(
    mut discover: impl FnMut() -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    match discover() {
        Err(RuntimeError::Protocol(_)) => discover(),
        result => result,
    }
}

pub fn segmented_jsonl_files(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<Vec<AnchoredFile>, RuntimeError> {
    ensure_anchored_real_file(base)?;
    let mut files = vec![base.clone()];
    #[cfg(test)]
    record_retained_paths(files.len());
    let mut siblings = Vec::new();
    let mut invalid_ordinal = None;
    let mut exceeds_limit = false;
    for_each_segmented_jsonl_member(base, |member| {
        let (ordinal, candidate) = canonical_segmented_jsonl_sibling(base, member)?;
        if ordinal < 2 {
            invalid_ordinal = Some(invalid_ordinal.map_or(ordinal, |old: u64| old.min(ordinal)));
        } else if ordinal > limits.max_segments {
            exceeds_limit = true;
        } else {
            siblings.push((ordinal, candidate));
        }
        Ok(())
    })?;
    if let Some(ordinal) = invalid_ordinal {
        return Err(RuntimeError::Protocol(format!(
            "{} has invalid segmented JSONL ordinal {ordinal:06}",
            base.diagnostic_path().display()
        )));
    }
    if exceeds_limit {
        return Err(RuntimeError::Protocol(format!(
            "{} segment count exceeds max {}",
            base.diagnostic_path().display(),
            limits.max_segments
        )));
    }
    siblings.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    for (expected, (ordinal, candidate)) in (2..).zip(siblings) {
        if ordinal != expected {
            return Err(RuntimeError::Protocol(format!(
                "{} has non-contiguous segmented JSONL ordinals",
                base.diagnostic_path().display()
            )));
        }
        ensure_anchored_real_file(&candidate)?;
        files.push(candidate);
        #[cfg(test)]
        record_retained_paths(files.len());
    }
    Ok(files)
}

pub fn segmented_jsonl_segment_count(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<usize, RuntimeError> {
    ensure_anchored_real_file(base)?;
    let mut segment_count = 1usize;
    let mut max_ordinal = 1u64;
    let mut invalid_ordinal = None;
    let mut exceeds_limit = false;
    for_each_segmented_jsonl_member(base, |member| {
        let (ordinal, candidate) = canonical_segmented_jsonl_sibling(base, member)?;
        if ordinal < 2 {
            invalid_ordinal = Some(invalid_ordinal.map_or(ordinal, |old: u64| old.min(ordinal)));
        } else if ordinal > limits.max_segments {
            exceeds_limit = true;
        } else {
            ensure_anchored_real_file(&candidate)?;
            segment_count = segment_count.saturating_add(1);
            max_ordinal = max_ordinal.max(ordinal);
            #[cfg(test)]
            record_retained_paths(1);
        }
        Ok(())
    })?;
    if let Some(ordinal) = invalid_ordinal {
        return Err(RuntimeError::Protocol(format!(
            "{} has invalid segmented JSONL ordinal {ordinal:06}",
            base.diagnostic_path().display()
        )));
    }
    if exceeds_limit {
        return Err(RuntimeError::Protocol(format!(
            "{} segment count exceeds max {}",
            base.diagnostic_path().display(),
            limits.max_segments
        )));
    }
    if u64::try_from(segment_count).unwrap_or(u64::MAX) != max_ordinal {
        return Err(RuntimeError::Protocol(format!(
            "{} has non-contiguous segmented JSONL ordinals",
            base.diagnostic_path().display()
        )));
    }
    Ok(segment_count)
}

pub fn for_each_segmented_jsonl_line(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
    mut visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let mut total = 0u64;
    let files = segmented_jsonl_files(base, limits)?;
    let segment_count = files.len();
    for (index, file) in files.into_iter().enumerate() {
        let remaining = limits.max_total_bytes.saturating_sub(total);
        let non_final = index + 1 != segment_count;
        let segment_bytes = for_each_anchored_file_line_with_limit(
            &file,
            MAX_SESSION_SEGMENT_BYTES.min(remaining),
            non_final,
            &mut visit,
        )?;
        total = total.saturating_add(segment_bytes);
    }
    Ok(total)
}
