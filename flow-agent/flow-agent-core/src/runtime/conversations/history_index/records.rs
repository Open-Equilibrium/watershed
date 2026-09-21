use super::super::contract::protocol;
use super::model::{
    ConversationEntry, EVENT_POINTER_RECORD_BYTES, EVENT_POINTER_SEQUENCE_OFFSET,
    EventPointerRecord, INDEX_ENTRY_ID_OFFSET, INDEX_EVENT_SEQUENCE_OFFSET, INDEX_ID_FIELD_BYTES,
    INDEX_IO_BUFFER_BYTES, INDEX_ORDINAL_OFFSET, INDEX_PARENT_ID_OFFSET, INDEX_RECORD_BYTES,
    INDEX_RUN_SESSION_ID_OFFSET, IndexRecord, MAX_HISTORY_INDEX_ID_BYTES, WorkBudget,
};
use crate::runtime::{
    fs_guards::{AnchoredFile, open_anchored_file_for_read, path_io_error},
    types::RuntimeError,
};
use std::{
    cmp::Ordering as CmpOrdering,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
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
    let mut sequential =
        BufReader::with_capacity(INDEX_IO_BUFFER_BYTES, open_anchored_file_for_read(path)?.0);
    let mut lookup = open_anchored_file_for_read(path)?.0;
    let mut prior: Option<[u8; INDEX_ID_FIELD_BYTES]> = None;
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
    let mut file =
        BufReader::with_capacity(INDEX_IO_BUFFER_BYTES, open_anchored_file_for_read(path)?.0);
    for _ in 0..entries {
        records.push(
            read_index_record(&mut file)?
                .ok_or_else(|| protocol("conversation history index ended early"))?,
        );
    }
    if read_index_record(&mut file)?.is_some() {
        return Err(protocol("conversation history index has trailing records"));
    }
    let mut prior: Option<[u8; INDEX_ID_FIELD_BYTES]> = None;
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

fn encode_id_bytes(id: &[u8]) -> Result<[u8; INDEX_ID_FIELD_BYTES], RuntimeError> {
    let mut encoded = [0u8; INDEX_ID_FIELD_BYTES];
    if id.len() > MAX_HISTORY_INDEX_ID_BYTES {
        return Err(protocol("conversation history index id is oversized"));
    }
    encoded[0] = u8::try_from(id.len())
        .map_err(|_| protocol("conversation history index id is oversized"))?;
    encoded[1..1 + id.len()].copy_from_slice(id);
    Ok(encoded)
}

pub(super) fn read_index_record(file: &mut impl Read) -> Result<Option<IndexRecord>, RuntimeError> {
    read_fixed_record(
        file,
        "conversation history index record is truncated",
        "conversation history validation index",
    )
}

pub(super) fn read_event_pointer_record(
    file: &mut impl Read,
) -> Result<Option<EventPointerRecord>, RuntimeError> {
    read_fixed_record(
        file,
        "conversation history pointer record is truncated",
        "conversation history pointer index",
    )
}

pub(super) fn read_fixed_record<const N: usize>(
    file: &mut impl Read,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, BufReader, Cursor};

    struct ShortReader<R>(R);

    impl<R: Read> Read for ShortReader<R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let length = bytes.len().min(2);
            self.0.read(&mut bytes[..length])
        }
    }

    #[test]
    fn fixed_records_preserve_framing_across_short_reads_and_buffer_boundaries() {
        for tail in [b"".as_slice(), b"x", b"xy"] {
            let bytes = [b"abcdef".as_slice(), tail].concat();
            let mut reader = BufReader::with_capacity(5, ShortReader(Cursor::new(bytes)));
            for expected in [*b"abc", *b"def"] {
                assert_eq!(
                    read_fixed_record::<3>(&mut reader, "truncated", "test index")
                        .expect("complete record reads"),
                    Some(expected)
                );
            }
            let end = read_fixed_record::<3>(&mut reader, "truncated", "test index");
            if tail.is_empty() {
                assert_eq!(end.expect("record-aligned EOF succeeds"), None);
            } else {
                assert!(
                    end.expect_err("partial record fails")
                        .to_string()
                        .contains("truncated")
                );
            }
        }
    }

    #[test]
    fn fixed_record_read_error_is_not_eof_or_truncation() {
        struct FailedRead;
        impl Read for FailedRead {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("synthetic read failure"))
            }
        }
        let mut reader = BufReader::with_capacity(5, Cursor::new(b"a").chain(FailedRead));
        let error = read_fixed_record::<3>(&mut reader, "truncated", "test index")
            .expect_err("I/O failure after a partial record propagates");
        match error {
            RuntimeError::Io { path, source } => {
                assert_eq!(path, PathBuf::from("test index"));
                assert_eq!(source.kind(), io::ErrorKind::Other);
                assert_eq!(source.to_string(), "synthetic read failure");
            }
            other => panic!("expected the original I/O error, got {other}"),
        }
    }
}
