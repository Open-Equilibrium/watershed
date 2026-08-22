use super::super::contract::protocol;
use super::model::{
    ConversationEntry, EVENT_POINTER_RECORD_BYTES, EVENT_POINTER_SEQUENCE_OFFSET,
    EventPointerRecord, INDEX_ANCESTRY_RECORD_BYTES, INDEX_ENTRY_ID_OFFSET,
    INDEX_EVENT_SEQUENCE_OFFSET, INDEX_ORDINAL_OFFSET, INDEX_PARENT_ID_OFFSET, INDEX_RECORD_BYTES,
    INDEX_RUN_SESSION_ID_OFFSET, IndexRecord, IndexedConversationEntry, MAX_HISTORY_INDEX_ID_BYTES,
    WorkBudget,
};
use crate::runtime::{
    fs_guards::{AnchoredFile, open_anchored_file_for_read, path_io_error},
    types::RuntimeError,
};
use std::{
    cmp::Ordering as CmpOrdering,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

pub(super) fn validate_sorted_index(
    path: &AnchoredFile,
    entries: u64,
    chunk: &mut Vec<IndexRecord>,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    if usize::try_from(entries).is_ok_and(|entries| entries <= chunk.capacity()) {
        return validate_sorted_index_in_memory(path, entries, chunk, work);
    }
    let (mut sequential, _) = open_anchored_file_for_read(path)?;
    let mut lookup = open_anchored_file_for_read(path)?.0;
    let mut prior: Option<[u8; INDEX_ANCESTRY_RECORD_BYTES]> = None;
    for _ in 0..entries {
        let record = read_index_record(&mut sequential)?
            .ok_or_else(|| protocol("conversation history index ended early"))?;
        work.add(1)?;
        let current = encode_id_bytes(record_id(&record))?;
        if prior.as_ref().is_some_and(|id| id == &current) {
            return Err(protocol("conversation entry id is duplicated"));
        }
        let child_ordinal = record_ordinal(&record);
        if let Some(parent) = record_parent(&record) {
            let parent = find_record_in(&mut lookup, path, entries, parent, work)?
                .ok_or_else(|| protocol("conversation parent entry does not precede its child"))?;
            if record_ordinal(&parent) >= child_ordinal {
                return Err(protocol(
                    "conversation parent entry does not precede its child",
                ));
            }
        }
        prior = Some(current);
    }
    if read_index_record(&mut sequential)?.is_some() {
        return Err(protocol("conversation history index has trailing records"));
    }
    Ok(())
}

fn validate_sorted_index_in_memory(
    path: &AnchoredFile,
    entries: u64,
    records: &mut Vec<IndexRecord>,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    let (mut file, _) = open_anchored_file_for_read(path)?;
    for _ in 0..entries {
        records.push(
            read_index_record(&mut file)?
                .ok_or_else(|| protocol("conversation history index ended early"))?,
        );
    }
    if read_index_record(&mut file)?.is_some() {
        return Err(protocol("conversation history index has trailing records"));
    }
    let mut prior: Option<[u8; INDEX_ANCESTRY_RECORD_BYTES]> = None;
    for record in records.iter() {
        work.add(1)?;
        let current = encode_id_bytes(record_id(record))?;
        if prior.as_ref().is_some_and(|id| id == &current) {
            return Err(protocol("conversation entry id is duplicated"));
        }
        let child_ordinal = record_ordinal(record);
        if let Some(parent) = record_parent(record) {
            let parent = find_record_in_memory(records, parent, work)?
                .ok_or_else(|| protocol("conversation parent entry does not precede its child"))?;
            if record_ordinal(parent) >= child_ordinal {
                return Err(protocol(
                    "conversation parent entry does not precede its child",
                ));
            }
        }
        prior = Some(current);
    }
    Ok(())
}

fn find_record_in_memory<'a>(
    records: &'a [IndexRecord],
    id: &[u8],
    work: &mut WorkBudget,
) -> Result<Option<&'a IndexRecord>, RuntimeError> {
    let mut low = 0usize;
    let mut high = records.len();
    while low < high {
        work.add(1)?;
        let middle = low + (high - low) / 2;
        match record_id(&records[middle]).cmp(id) {
            CmpOrdering::Less => low = middle + 1,
            CmpOrdering::Greater => high = middle,
            CmpOrdering::Equal => return Ok(Some(&records[middle])),
        }
    }
    Ok(None)
}

pub(super) fn find_record(
    path: &AnchoredFile,
    entries: u64,
    id: &[u8],
    work: &mut WorkBudget,
) -> Result<Option<IndexRecord>, RuntimeError> {
    let mut file = open_anchored_file_for_read(path)?.0;
    find_record_in(&mut file, path, entries, id, work)
}

fn find_record_in(
    file: &mut File,
    path: &AnchoredFile,
    entries: u64,
    id: &[u8],
    work: &mut WorkBudget,
) -> Result<Option<IndexRecord>, RuntimeError> {
    let mut low = 0u64;
    let mut high = entries;
    while low < high {
        work.add(1)?;
        let middle = low + (high - low) / 2;
        file.seek(SeekFrom::Start(middle * INDEX_RECORD_BYTES as u64))
            .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
        let record = read_index_record(file)?
            .ok_or_else(|| protocol("conversation history index lookup ended early"))?;
        match record_id(&record).cmp(id) {
            CmpOrdering::Less => low = middle + 1,
            CmpOrdering::Greater => high = middle,
            CmpOrdering::Equal => return Ok(Some(record)),
        }
    }
    Ok(None)
}

pub(super) fn encode_record(
    entry: &ConversationEntry,
    ordinal: u64,
) -> Result<IndexRecord, RuntimeError> {
    let mut record = [0u8; INDEX_RECORD_BYTES];
    encode_field(
        &mut record[INDEX_ENTRY_ID_OFFSET..INDEX_PARENT_ID_OFFSET],
        &entry.entry_id,
    )?;
    if let Some(parent) = &entry.parent_entry_id {
        encode_field(
            &mut record[INDEX_PARENT_ID_OFFSET..INDEX_RUN_SESSION_ID_OFFSET],
            parent,
        )?;
    } else {
        record[INDEX_PARENT_ID_OFFSET] = u8::MAX;
    }
    encode_field(
        &mut record[INDEX_RUN_SESSION_ID_OFFSET..INDEX_ORDINAL_OFFSET],
        &entry.run_session_id,
    )?;
    record[INDEX_ORDINAL_OFFSET..INDEX_EVENT_SEQUENCE_OFFSET]
        .copy_from_slice(&ordinal.to_le_bytes());
    record[INDEX_EVENT_SEQUENCE_OFFSET..INDEX_RECORD_BYTES]
        .copy_from_slice(&entry.event_sequence.to_le_bytes());
    Ok(record)
}

pub(super) fn encode_event_pointer_record(entry: &IndexRecord) -> EventPointerRecord {
    let mut record = [0u8; EVENT_POINTER_RECORD_BYTES];
    record[..EVENT_POINTER_SEQUENCE_OFFSET]
        .copy_from_slice(&entry[INDEX_RUN_SESSION_ID_OFFSET..INDEX_ORDINAL_OFFSET]);
    record[EVENT_POINTER_SEQUENCE_OFFSET..]
        .copy_from_slice(&entry[INDEX_EVENT_SEQUENCE_OFFSET..INDEX_RECORD_BYTES]);
    record
}

fn encode_field(target: &mut [u8], value: &str) -> Result<(), RuntimeError> {
    let length = u8::try_from(value.len())
        .map_err(|_| protocol("conversation history index id is oversized"))?;
    if value.len() > MAX_HISTORY_INDEX_ID_BYTES {
        return Err(protocol("conversation history index id is oversized"));
    }
    target[0] = length;
    target[1..1 + value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}

pub(super) fn record_id(record: &IndexRecord) -> &[u8] {
    &record[INDEX_ENTRY_ID_OFFSET + 1
        ..INDEX_ENTRY_ID_OFFSET + 1 + record[INDEX_ENTRY_ID_OFFSET] as usize]
}

fn record_parent(record: &IndexRecord) -> Option<&[u8]> {
    let length = record[INDEX_PARENT_ID_OFFSET] as usize;
    (record[INDEX_PARENT_ID_OFFSET] != u8::MAX)
        .then(|| &record[INDEX_PARENT_ID_OFFSET + 1..INDEX_PARENT_ID_OFFSET + 1 + length])
}

fn record_ordinal(record: &IndexRecord) -> u64 {
    u64::from_le_bytes(
        record[INDEX_ORDINAL_OFFSET..INDEX_EVENT_SEQUENCE_OFFSET]
            .try_into()
            .unwrap(),
    )
}

pub(super) fn event_pointer_id(record: &EventPointerRecord) -> &[u8] {
    &record[1..1 + record[0] as usize]
}

pub(super) fn event_pointer_sequence(record: &EventPointerRecord) -> u64 {
    u64::from_le_bytes(
        record[EVENT_POINTER_SEQUENCE_OFFSET..EVENT_POINTER_RECORD_BYTES]
            .try_into()
            .unwrap(),
    )
}

pub(super) fn decode_index_id(id: Vec<u8>) -> Result<String, RuntimeError> {
    String::from_utf8(id).map_err(|_| protocol("conversation history index id is not UTF-8"))
}

pub(super) fn decode_record(record: IndexRecord) -> IndexedConversationEntry {
    IndexedConversationEntry {
        entry_id: String::from_utf8(record_id(&record).to_vec()).unwrap(),
        parent_entry_id: record_parent(&record)
            .map(|value| String::from_utf8(value.to_vec()).unwrap()),
        run_session_id: String::from_utf8(
            record[INDEX_RUN_SESSION_ID_OFFSET + 1
                ..INDEX_RUN_SESSION_ID_OFFSET + 1 + record[INDEX_RUN_SESSION_ID_OFFSET] as usize]
                .to_vec(),
        )
        .unwrap(),
        event_sequence: u64::from_le_bytes(
            record[INDEX_EVENT_SEQUENCE_OFFSET..INDEX_RECORD_BYTES]
                .try_into()
                .unwrap(),
        ),
    }
}

pub(super) fn encode_id(id: &str) -> Result<[u8; INDEX_ANCESTRY_RECORD_BYTES], RuntimeError> {
    encode_id_bytes(id.as_bytes())
}

fn encode_id_bytes(id: &[u8]) -> Result<[u8; INDEX_ANCESTRY_RECORD_BYTES], RuntimeError> {
    let mut encoded = [0u8; INDEX_ANCESTRY_RECORD_BYTES];
    if id.len() > MAX_HISTORY_INDEX_ID_BYTES {
        return Err(protocol("conversation ancestry id is oversized"));
    }
    encoded[0] =
        u8::try_from(id.len()).map_err(|_| protocol("conversation ancestry id is oversized"))?;
    encoded[1..1 + id.len()].copy_from_slice(id);
    Ok(encoded)
}

pub(super) fn decode_id(
    encoded: &[u8; INDEX_ANCESTRY_RECORD_BYTES],
) -> Result<String, RuntimeError> {
    String::from_utf8(encoded[1..1 + encoded[0] as usize].to_vec())
        .map_err(|_| protocol("conversation ancestry id is not UTF-8"))
}

pub(super) fn read_index_record(file: &mut File) -> Result<Option<IndexRecord>, RuntimeError> {
    read_fixed_record(
        file,
        "conversation history index record is truncated",
        "conversation history validation index",
    )
}

pub(super) fn read_event_pointer_record(
    file: &mut File,
) -> Result<Option<EventPointerRecord>, RuntimeError> {
    read_fixed_record(
        file,
        "conversation history pointer record is truncated",
        "conversation history pointer index",
    )
}

pub(super) fn read_fixed_record<const N: usize>(
    file: &mut File,
    truncated: &'static str,
    diagnostic_path: &'static str,
) -> Result<Option<[u8; N]>, RuntimeError> {
    let mut record = [0u8; N];
    let mut read = 0usize;
    while read < record.len() {
        match file.read(&mut record[read..]) {
            Ok(0) if read == 0 => return Ok(None),
            Ok(0) => return Err(protocol(truncated)),
            Ok(bytes) => read += bytes,
            Err(source) => {
                return Err(RuntimeError::Io {
                    path: PathBuf::from(diagnostic_path),
                    source,
                });
            }
        }
    }
    Ok(Some(record))
}
