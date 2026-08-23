use super::super::contract::protocol;
use super::model::{
    EVENT_POINTER_RECORD_BYTES, EventPointerRecord, INDEX_MERGE_FAN_IN, INDEX_RECORD_BYTES,
    INDEX_SORT_BYTES, IndexRecord, WorkBudget,
};
use super::records::{event_pointer_id, read_event_pointer_record, read_index_record, record_id};
use super::scratch::{
    HistoryScratch, create_scratch_file, event_pointer_run_leaf, index_run_leaf,
    write_sorted_scratch_run,
};
use crate::runtime::{
    fs_guards::{open_anchored_file_for_read, path_io_error},
    types::RuntimeError,
};
use std::{cell::Cell, fs::File};

#[cfg(test)]
thread_local! {
    static INDEX_SORT_RECORD_LIMIT_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
    static EVENT_POINTER_SORT_RECORD_LIMIT_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(super) fn write_sorted_run(
    scratch: &mut HistoryScratch,
    chunk: &mut Vec<IndexRecord>,
    generation: u32,
    run: u64,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    write_sorted_record_run(scratch, chunk, generation, run, work)
}

pub(super) fn merge_all_runs(
    scratch: &mut HistoryScratch,
    count: u64,
    work: &mut WorkBudget,
) -> Result<(u32, u64), RuntimeError> {
    merge_all_record_runs::<IndexRecord>(scratch, count, work)
}

pub(super) fn write_sorted_event_pointer_run(
    scratch: &mut HistoryScratch,
    chunk: &mut Vec<EventPointerRecord>,
    generation: u32,
    run: u64,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    write_sorted_record_run(scratch, chunk, generation, run, work)
}

pub(super) fn merge_all_event_pointer_runs(
    scratch: &mut HistoryScratch,
    count: u64,
    work: &mut WorkBudget,
) -> Result<(u32, u64), RuntimeError> {
    merge_all_record_runs::<EventPointerRecord>(scratch, count, work)
}

trait ExternalSortRecord: AsRef<[u8]> + Clone + Sized {
    fn id(&self) -> &[u8];
    fn read(file: &mut File) -> Result<Option<Self>, RuntimeError>;
    fn run_leaf(generation: u32, run: u64) -> String;
    fn merge_generation_overflow() -> &'static str;
}

impl ExternalSortRecord for IndexRecord {
    fn id(&self) -> &[u8] {
        record_id(self)
    }

    fn read(file: &mut File) -> Result<Option<Self>, RuntimeError> {
        read_index_record(file)
    }

    fn run_leaf(generation: u32, run: u64) -> String {
        index_run_leaf(generation, run)
    }

    fn merge_generation_overflow() -> &'static str {
        "conversation history merge generation overflow"
    }
}

impl ExternalSortRecord for EventPointerRecord {
    fn id(&self) -> &[u8] {
        event_pointer_id(self)
    }

    fn read(file: &mut File) -> Result<Option<Self>, RuntimeError> {
        read_event_pointer_record(file)
    }

    fn run_leaf(generation: u32, run: u64) -> String {
        event_pointer_run_leaf(generation, run)
    }

    fn merge_generation_overflow() -> &'static str {
        "conversation history pointer merge generation overflow"
    }
}

fn write_sorted_record_run<R: ExternalSortRecord>(
    scratch: &mut HistoryScratch,
    chunk: &mut Vec<R>,
    generation: u32,
    run: u64,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    let comparisons = Cell::new(0u64);
    chunk.sort_unstable_by(|left, right| {
        comparisons.set(comparisons.get().saturating_add(1));
        left.id().cmp(right.id())
    });
    work.add(comparisons.get())?;
    let leaf = R::run_leaf(generation, run);
    write_sorted_scratch_run(scratch, chunk, &leaf)
}

fn merge_all_record_runs<R: ExternalSortRecord>(
    scratch: &mut HistoryScratch,
    mut count: u64,
    work: &mut WorkBudget,
) -> Result<(u32, u64), RuntimeError> {
    let mut generation = 0u32;
    while count > 1 {
        let next_generation = generation
            .checked_add(1)
            .ok_or_else(|| protocol(R::merge_generation_overflow()))?;
        let mut output = 0u64;
        let mut first = 0u64;
        while first < count {
            let length = (count - first).min(INDEX_MERGE_FAN_IN);
            merge_record_run_group::<R>(
                scratch,
                generation,
                first,
                length,
                next_generation,
                output,
                work,
            )?;
            first += length;
            output += 1;
        }
        generation = next_generation;
        count = output;
    }
    Ok((generation, count))
}

fn merge_record_run_group<R: ExternalSortRecord>(
    scratch: &mut HistoryScratch,
    generation: u32,
    first: u64,
    length: u64,
    next_generation: u32,
    output_run: u64,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    let mut readers = Vec::new();
    for run in first..first + length {
        let path = scratch.dir.file(R::run_leaf(generation, run));
        readers.push(open_anchored_file_for_read(&path)?.0);
    }
    let mut heads = vec![None; readers.len()];
    for (head, reader) in heads.iter_mut().zip(&mut readers) {
        *head = R::read(reader)?;
    }
    let leaf = R::run_leaf(next_generation, output_run);
    let path = scratch.dir.file(&leaf);
    let mut output = create_scratch_file(&scratch.dir, &leaf)?;
    loop {
        let mut least: Option<usize> = None;
        for (index, head) in heads.iter().enumerate() {
            if let Some(record) = head {
                work.add(1)?;
                if least.is_none_or(|prior| {
                    record.id() < heads[prior].as_ref().expect("prior head is present").id()
                }) {
                    least = Some(index);
                }
            }
        }
        let Some(least) = least else { break };
        scratch.write(
            &mut output,
            &path,
            heads[least]
                .as_ref()
                .expect("selected head is present")
                .as_ref(),
        )?;
        heads[least] = R::read(&mut readers[least])?;
    }
    output
        .sync_all()
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    drop(output);
    drop(readers);
    for run in first..first + length {
        scratch.remove_file(&R::run_leaf(generation, run))?;
    }
    Ok(())
}

pub(super) fn index_sort_record_limit() -> usize {
    #[cfg(test)]
    if let Some(limit) = INDEX_SORT_RECORD_LIMIT_OVERRIDE.with(Cell::get) {
        return limit;
    }
    INDEX_SORT_BYTES / INDEX_RECORD_BYTES
}

pub(super) fn event_pointer_sort_record_limit() -> usize {
    #[cfg(test)]
    if let Some(limit) = EVENT_POINTER_SORT_RECORD_LIMIT_OVERRIDE.with(Cell::get) {
        return limit;
    }
    INDEX_SORT_BYTES / EVENT_POINTER_RECORD_BYTES
}

#[cfg(test)]
pub(super) fn set_event_pointer_sort_record_limit_for_test(limit: Option<usize>) {
    EVENT_POINTER_SORT_RECORD_LIMIT_OVERRIDE.with(|slot| slot.set(limit));
}

#[cfg(test)]
pub(super) fn set_history_index_sort_record_limit_for_test(limit: Option<usize>) {
    INDEX_SORT_RECORD_LIMIT_OVERRIDE.with(|slot| slot.set(limit));
}
