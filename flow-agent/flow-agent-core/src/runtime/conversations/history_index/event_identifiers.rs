use super::super::contract::{CONVERSATION_RUNS_DIR, RUN_EVENTS_LEAF, protocol};
use super::external_sort::{
    event_pointer_sort_record_limit, merge_all_event_pointer_runs, write_sorted_event_pointer_run,
};
use super::model::{EventPointerMetrics, EventPointerRecord, WorkBudget};
use super::records::{
    decode_index_id, encode_event_pointer_record, event_pointer_id, event_pointer_sequence,
    read_event_pointer_record, read_fixed_record, read_index_record,
};
use super::scratch::{
    HistoryScratch, INDEX_WORK_RESERVE, event_identifier_run_leaf, event_pointer_run_leaf,
    write_sorted_scratch_run,
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredFile, DirectoryErrorMode, for_each_segmented_jsonl_line,
        open_anchored_file_for_read, path_io_error, segmented_jsonl_files,
    },
    types::{EVENT_STREAM_LIMITS, MAX_CANONICAL_EVENT_BYTES, MAX_FLOW_EVENTS, RuntimeError},
    validate::{SessionAppendValidationState, parse_canonical_event, validate_event_size},
};
use proto::{
    EventEnvelope, EventStateIdentifierKind, MAX_EVENT_PAYLOAD_STATE_IDENTIFIERS_V0,
    MAX_EVENT_STATE_IDENTIFIERS_V0,
};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering as CmpOrdering,
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
};

const EVENT_IDENTIFIER_RECORD_BYTES: usize = 48;

#[cfg(test)]
thread_local! {
    static FORCE_EVENT_IDENTIFIER_DIGEST_COLLISION: Cell<bool> = const { Cell::new(false) };
}
const EVENT_IDENTIFIER_MAX_RECORDS_PER_PASS: u64 =
    MAX_FLOW_EVENTS * MAX_EVENT_PAYLOAD_STATE_IDENTIFIERS_V0;
pub(super) const EVENT_IDENTIFIER_SORT_BYTES: u64 =
    EVENT_IDENTIFIER_MAX_RECORDS_PER_PASS * EVENT_IDENTIFIER_RECORD_BYTES as u64;
const EVENT_IDENTIFIER_MAX_LOCATIONS: u64 = MAX_FLOW_EVENTS * MAX_EVENT_STATE_IDENTIFIERS_V0;
const EVENT_STATE_BYTES_PER_EVENT_BOUND: u64 = 320;
const EVENT_TRANSIENT_MEMORY_RESERVE: u64 = 2 * 1024 * 1024;
const EVENT_IDENTIFIER_TOKEN_BYTES: u64 =
    EVENT_IDENTIFIER_MAX_LOCATIONS * size_of::<IdentifierTokenLocation>() as u64;
const EVENT_IDENTIFIER_BUILD_MEMORY_BOUND: u64 = EVENT_IDENTIFIER_SORT_BYTES
    + EVENT_IDENTIFIER_TOKEN_BYTES
    + MAX_CANONICAL_EVENT_BYTES as u64
    + EVENT_TRANSIENT_MEMORY_RESERVE;
const EVENT_IDENTIFIER_VALIDATION_MEMORY_BOUND: u64 = EVENT_IDENTIFIER_TOKEN_BYTES
    + MAX_FLOW_EVENTS * EVENT_STATE_BYTES_PER_EVENT_BOUND
    + MAX_CANONICAL_EVENT_BYTES as u64
    + EVENT_TRANSIENT_MEMORY_RESERVE;
pub(super) const EVENT_IDENTIFIER_MEMORY_BOUND: u64 =
    if EVENT_IDENTIFIER_BUILD_MEMORY_BOUND > EVENT_IDENTIFIER_VALIDATION_MEMORY_BOUND {
        EVENT_IDENTIFIER_BUILD_MEMORY_BOUND
    } else {
        EVENT_IDENTIFIER_VALIDATION_MEMORY_BOUND
    };
type EventIdentifierRecord = [u8; EVENT_IDENTIFIER_RECORD_BYTES];

#[derive(Clone, Copy)]
struct IdentifierTokenLocation {
    offset: u32,
    token: u32,
    kind: EventStateIdentifierKind,
}

struct EventIdentifierSource {
    segments: Vec<(u64, u64, AnchoredFile)>,
}

impl IdentifierTokenLocation {
    fn new(offset: u64, kind: EventStateIdentifierKind, token: u32) -> Result<Self, RuntimeError> {
        let offset = u32::try_from(offset)
            .map_err(|_| protocol("event identifier mapping offset overflow"))?;
        Ok(Self {
            offset,
            token,
            kind,
        })
    }

    fn key(self) -> (u32, EventStateIdentifierKind) {
        (self.offset, self.kind)
    }

    fn lookup_key(
        offset: u64,
        kind: EventStateIdentifierKind,
    ) -> Result<(u32, EventStateIdentifierKind), RuntimeError> {
        Self::new(offset, kind, 0).map(Self::key)
    }
}

impl EventIdentifierSource {
    fn open(events: &AnchoredFile) -> Result<Self, RuntimeError> {
        let mut segments = Vec::new();
        let mut start = 0u64;
        for segment in segmented_jsonl_files(events, EVENT_STREAM_LIMITS)? {
            let length = segment.metadata()?.len();
            let end = start
                .checked_add(length)
                .ok_or_else(|| protocol("event identifier source offset overflow"))?;
            segments.push((start, end, segment));
            start = end;
        }
        Ok(Self { segments })
    }

    fn identifier(
        &self,
        offset: u64,
        kind: EventStateIdentifierKind,
    ) -> Result<String, RuntimeError> {
        let (start, _, segment) = self
            .segments
            .iter()
            .find(|(start, end, _)| *start <= offset && offset < *end)
            .ok_or_else(|| protocol("event identifier source offset is outside its segments"))?;
        let (mut file, _) = open_anchored_file_for_read(segment)?;
        file.seek(SeekFrom::Start(offset - start))
            .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
        let mut line = Vec::new();
        BufReader::new(file)
            .take((MAX_CANONICAL_EVENT_BYTES as u64).saturating_add(1))
            .read_until(b'\n', &mut line)
            .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
        let line = std::str::from_utf8(&line).map_err(|source| {
            protocol(format!(
                "{} is not valid UTF-8: {source}",
                segment.diagnostic_path().display()
            ))
        })?;
        let canonical = line
            .strip_suffix('\n')
            .ok_or_else(|| protocol("committed event identifier source must end with LF"))?;
        let event = parse_canonical_event(segment.diagnostic_path(), 1, canonical)?;
        let mut identifier = None;
        event.try_for_each_state_identifier::<RuntimeError>(|candidate, value| {
            if candidate == kind {
                identifier = Some(value.to_owned());
            }
            Ok(())
        })?;
        identifier.ok_or_else(|| protocol("event identifier source no longer contains its field"))
    }

    fn compare(
        &self,
        left: &EventIdentifierRecord,
        right: &EventIdentifierRecord,
    ) -> Result<CmpOrdering, RuntimeError> {
        let left_kind = identifier_record_kind(left)?;
        let right_kind = identifier_record_kind(right)?;
        let left = self.identifier(identifier_record_offset(left), left_kind)?;
        let right = self.identifier(identifier_record_offset(right), right_kind)?;
        Ok(left.cmp(&right))
    }
}

pub(super) fn validate_history_event_pointers(
    index: &AnchoredFile,
    conversation: &AnchoredDir,
    entries: u64,
    scratch: &mut HistoryScratch,
    work: &mut WorkBudget,
) -> Result<EventPointerMetrics, RuntimeError> {
    if entries == 0 {
        return Ok(EventPointerMetrics::default());
    }
    let mut chunk = Vec::<EventPointerRecord>::new();
    let sort_record_limit = event_pointer_sort_record_limit();
    chunk
        .try_reserve_exact(sort_record_limit)
        .map_err(|_| protocol("conversation history pointer sort memory admission failed"))?;
    let (mut source, _) = open_anchored_file_for_read(index)?;
    let mut pointer_count = 0u64;
    let mut run_count = 0u64;
    for _ in 0..entries {
        let record = read_index_record(&mut source)?
            .ok_or_else(|| protocol("conversation history index ended early"))?;
        work.add(1)?;
        chunk.push(encode_event_pointer_record(&record));
        pointer_count = pointer_count
            .checked_add(1)
            .ok_or_else(|| protocol("conversation history entry count overflow"))?;
        if chunk.len() == sort_record_limit {
            write_sorted_event_pointer_run(scratch, &mut chunk, 0, run_count, work)?;
            run_count += 1;
        }
    }
    if read_index_record(&mut source)?.is_some() {
        return Err(protocol("conversation history index has trailing records"));
    }
    if pointer_count != entries {
        return Err(protocol(
            "conversation history changed while its event pointers were validated",
        ));
    }
    if !chunk.is_empty() {
        write_sorted_event_pointer_run(scratch, &mut chunk, 0, run_count, work)?;
        run_count += 1;
    }
    drop(chunk);
    let (generation, final_count) = merge_all_event_pointer_runs(scratch, run_count, work)?;
    if final_count != 1 {
        return Err(protocol(
            "conversation history pointer index did not produce one final run",
        ));
    }
    let leaf = event_pointer_run_leaf(generation, 0);
    let pointer_path = scratch.dir.file(&leaf);
    let event_metrics =
        validate_sorted_event_pointers(&pointer_path, conversation, entries, scratch, work)?;
    scratch.remove_file(&leaf)?;
    Ok(event_metrics)
}

fn validate_sorted_event_pointers(
    path: &AnchoredFile,
    conversation: &AnchoredDir,
    entries: u64,
    scratch: &mut HistoryScratch,
    work: &mut WorkBudget,
) -> Result<EventPointerMetrics, RuntimeError> {
    let (mut pointers, _) = open_anchored_file_for_read(path)?;
    let mut run_id: Option<Vec<u8>> = None;
    let mut max_sequence = 0u64;
    let mut metrics: EventPointerMetrics = Default::default();
    for _ in 0..entries {
        let record = read_event_pointer_record(&mut pointers)?
            .ok_or_else(|| protocol("conversation history pointer index ended early"))?;
        work.add(1)?;
        let current = event_pointer_id(&record);
        if run_id.as_deref().is_some_and(|prior| prior != current) {
            metrics.include(validate_committed_event_pointer(
                conversation,
                &decode_index_id(run_id.take().expect("prior run is present"))?,
                max_sequence,
                scratch,
                work,
            )?);
            max_sequence = 0;
        }
        if run_id.is_none() {
            run_id = Some(current.to_vec());
        }
        max_sequence = max_sequence.max(event_pointer_sequence(&record));
    }
    if read_event_pointer_record(&mut pointers)?.is_some() {
        return Err(protocol(
            "conversation history pointer index has trailing records",
        ));
    }
    if let Some(run_id) = run_id {
        metrics.include(validate_committed_event_pointer(
            conversation,
            &decode_index_id(run_id)?,
            max_sequence,
            scratch,
            work,
        )?);
    }
    Ok(metrics)
}

pub(super) fn validate_committed_event_pointer(
    conversation: &AnchoredDir,
    run_session_id: &str,
    event_sequence: u64,
    scratch: &mut HistoryScratch,
    work: &mut WorkBudget,
) -> Result<EventPointerMetrics, RuntimeError> {
    let runs = conversation
        .child(CONVERSATION_RUNS_DIR, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("conversation runs directory disappeared"))?;
    let run = runs
        .child(run_session_id, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("conversation history run disappeared"))?;
    let events = run.file(RUN_EVENTS_LEAF);
    let source = EventIdentifierSource::open(&events)?;
    let (mut tokens, mut event_work) =
        build_event_identifier_tokens(&events, &source, event_sequence, scratch, work)?;
    let comparisons = Cell::new(0u64);
    tokens.sort_unstable_by(|left, right| {
        comparisons.set(comparisons.get().saturating_add(1));
        left.key().cmp(&right.key())
    });
    event_work.add(comparisons.get())?;
    let mut validation = SessionAppendValidationState::empty(run_session_id);
    let mut reached = false;
    #[cfg(test)]
    let mut event_state_payload_peak = 0u64;
    let mut offset = 0u64;
    let mut line_number = 0usize;
    for_each_segmented_jsonl_line(&events, EVENT_STREAM_LIMITS, |line| {
        if reached {
            offset = offset.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
            return Ok(());
        }
        line_number = line_number.saturating_add(1);
        let canonical = line
            .strip_suffix('\n')
            .ok_or_else(|| protocol("committed event stream must end with LF"))?;
        validate_event_size(events.diagnostic_path(), line_number, line.len())?;
        let mut event = parse_canonical_event(events.diagnostic_path(), line_number, canonical)?;
        normalize_event_identifiers(&mut event, offset, &tokens)?;
        validation.validate_constructed_event(events.diagnostic_path(), &event, line.len())?;
        reached = event.sequence == event_sequence;
        #[cfg(test)]
        {
            event_state_payload_peak =
                event_state_payload_peak.max(validation.retained_identifier_payload_bytes());
        }
        offset = offset.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
        Ok(())
    })?;
    if reached {
        Ok(EventPointerMetrics {
            #[cfg(test)]
            state_payload_peak: event_state_payload_peak,
            #[cfg(test)]
            work: event_work.used,
            #[cfg(test)]
            work_limit: event_work.limit,
        })
    } else {
        Err(protocol(format!(
            "conversation history run {run_session_id} has no committed event at sequence {event_sequence}"
        )))
    }
}

fn build_event_identifier_tokens(
    events: &AnchoredFile,
    source: &EventIdentifierSource,
    event_sequence: u64,
    scratch: &mut HistoryScratch,
    _history_work: &mut WorkBudget,
) -> Result<(Vec<IdentifierTokenLocation>, WorkBudget), RuntimeError> {
    let max_locations = MAX_FLOW_EVENTS
        .checked_mul(MAX_EVENT_STATE_IDENTIFIERS_V0)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| protocol("event identifier mapping capacity overflow"))?;
    let mut tokens = Vec::new();
    tokens
        .try_reserve_exact(max_locations)
        .map_err(|_| protocol("event identifier mapping memory admission failed"))?;
    let mut event_work = WorkBudget {
        used: 0,
        limit: event_identifier_work_limit(event_sequence)?,
    };
    let mut next_token = 0u32;
    for pass in 0..3u8 {
        let mut chunk = Vec::<EventIdentifierRecord>::new();
        chunk
            .try_reserve_exact(
                usize::try_from(EVENT_IDENTIFIER_MAX_RECORDS_PER_PASS)
                    .map_err(|_| protocol("event identifier sort capacity overflow"))?,
            )
            .map_err(|_| protocol("event identifier sort memory admission failed"))?;
        let mut offset = 0u64;
        let mut line_number = 0usize;
        let mut reached = false;
        for_each_segmented_jsonl_line(events, EVENT_STREAM_LIMITS, |line| {
            if reached {
                offset = offset.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
                return Ok(());
            }
            line_number = line_number.saturating_add(1);
            let canonical = line
                .strip_suffix('\n')
                .ok_or_else(|| protocol("committed event stream must end with LF"))?;
            validate_event_size(events.diagnostic_path(), line_number, line.len())?;
            let event = parse_canonical_event(events.diagnostic_path(), line_number, canonical)?;
            for_each_identifier_in_pass(&event, pass, |kind, value| {
                if u64::try_from(chunk.len()).unwrap_or(u64::MAX)
                    >= EVENT_IDENTIFIER_MAX_RECORDS_PER_PASS
                {
                    return Err(protocol("event identifier sort record budget exceeded"));
                }
                chunk.push(encode_event_identifier_record(kind, value, offset));
                Ok(())
            })?;
            reached = event.sequence == event_sequence;
            offset = offset.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
            Ok(())
        })?;
        if !chunk.is_empty() {
            write_event_identifier_sorted_run(scratch, &mut chunk, pass, &mut event_work)?;
            drop(chunk);
            let leaf = event_identifier_run_leaf(pass);
            assign_event_identifier_tokens(
                &scratch.dir.file(&leaf),
                source,
                &mut tokens,
                &mut next_token,
                &mut event_work,
            )?;
            scratch.remove_file(&leaf)?;
        }
    }
    Ok((tokens, event_work))
}

fn event_identifier_work_limit(event_sequence: u64) -> Result<u64, RuntimeError> {
    let identifiers = event_sequence
        .min(MAX_FLOW_EVENTS)
        .checked_mul(MAX_EVENT_STATE_IDENTIFIERS_V0)
        .ok_or_else(|| protocol("event identifier work budget overflow"))?;
    let logarithm = if identifiers <= 1 {
        1
    } else {
        u64::from(u64::BITS - (identifiers - 1).leading_zeros())
    };
    identifiers
        .max(1)
        .checked_mul(logarithm + 1)
        .and_then(|work| work.checked_mul(128))
        .ok_or_else(|| protocol("event identifier work budget overflow"))
}

fn for_each_identifier_in_pass(
    event: &EventEnvelope,
    pass: u8,
    mut visit: impl FnMut(EventStateIdentifierKind, &str) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    if pass > 2 {
        return Err(protocol("event identifier sort has an invalid pass"));
    }
    event.try_for_each_state_identifier(|kind, value| {
        if identifier_pass(kind) == pass {
            visit(kind, value)?;
        }
        Ok(())
    })
}

const fn identifier_kind_byte(kind: EventStateIdentifierKind) -> u8 {
    match kind {
        EventStateIdentifierKind::Event => 0,
        EventStateIdentifierKind::Flow => 1,
        EventStateIdentifierKind::ParentFlow => 2,
        EventStateIdentifierKind::FlowDefinition => 3,
        EventStateIdentifierKind::PhaseExecution => 4,
        EventStateIdentifierKind::Phase => 5,
        EventStateIdentifierKind::Tool => 6,
        EventStateIdentifierKind::Message => 7,
        EventStateIdentifierKind::Attempt => 8,
    }
}

fn identifier_kind_from_byte(value: u8) -> Result<EventStateIdentifierKind, RuntimeError> {
    match value {
        0 => Ok(EventStateIdentifierKind::Event),
        1 => Ok(EventStateIdentifierKind::Flow),
        2 => Ok(EventStateIdentifierKind::ParentFlow),
        3 => Ok(EventStateIdentifierKind::FlowDefinition),
        4 => Ok(EventStateIdentifierKind::PhaseExecution),
        5 => Ok(EventStateIdentifierKind::Phase),
        6 => Ok(EventStateIdentifierKind::Tool),
        7 => Ok(EventStateIdentifierKind::Message),
        8 => Ok(EventStateIdentifierKind::Attempt),
        _ => Err(protocol("event identifier index has an invalid kind")),
    }
}

const fn identifier_namespace(kind: EventStateIdentifierKind) -> u8 {
    match kind {
        EventStateIdentifierKind::Event => 0,
        EventStateIdentifierKind::Flow | EventStateIdentifierKind::ParentFlow => 1,
        EventStateIdentifierKind::FlowDefinition => 2,
        EventStateIdentifierKind::PhaseExecution => 3,
        EventStateIdentifierKind::Phase => 4,
        EventStateIdentifierKind::Tool => 5,
        EventStateIdentifierKind::Message => 6,
        EventStateIdentifierKind::Attempt => 7,
    }
}

const fn identifier_pass(kind: EventStateIdentifierKind) -> u8 {
    match kind {
        EventStateIdentifierKind::Event => 0,
        EventStateIdentifierKind::Flow | EventStateIdentifierKind::ParentFlow => 1,
        EventStateIdentifierKind::FlowDefinition
        | EventStateIdentifierKind::PhaseExecution
        | EventStateIdentifierKind::Phase
        | EventStateIdentifierKind::Tool
        | EventStateIdentifierKind::Attempt
        | EventStateIdentifierKind::Message => 2,
    }
}

fn encode_event_identifier_record(
    kind: EventStateIdentifierKind,
    value: &str,
    offset: u64,
) -> EventIdentifierRecord {
    let mut record = [0u8; EVENT_IDENTIFIER_RECORD_BYTES];
    record[0] = identifier_namespace(kind);
    record[1] = identifier_kind_byte(kind);
    record[8..40].copy_from_slice(&event_identifier_digest(value));
    record[40..48].copy_from_slice(&offset.to_le_bytes());
    record
}

fn event_identifier_digest(value: &str) -> [u8; 32] {
    #[cfg(test)]
    if FORCE_EVENT_IDENTIFIER_DIGEST_COLLISION.with(Cell::get) {
        return [0; 32];
    }
    Sha256::digest(value.as_bytes()).into()
}

fn identifier_record_kind(
    record: &EventIdentifierRecord,
) -> Result<EventStateIdentifierKind, RuntimeError> {
    identifier_kind_from_byte(record[1])
}

fn identifier_record_offset(record: &EventIdentifierRecord) -> u64 {
    u64::from_le_bytes(record[40..48].try_into().unwrap())
}

fn compare_event_identifier_records(
    left: &EventIdentifierRecord,
    right: &EventIdentifierRecord,
) -> CmpOrdering {
    left[0]
        .cmp(&right[0])
        .then_with(|| left[8..40].cmp(&right[8..40]))
}

fn same_event_identifier_primary(
    left: &EventIdentifierRecord,
    right: &EventIdentifierRecord,
) -> bool {
    left[0] == right[0] && left[8..40] == right[8..40]
}

fn write_event_identifier_sorted_run(
    scratch: &mut HistoryScratch,
    chunk: &mut Vec<EventIdentifierRecord>,
    pass: u8,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    let comparisons = Cell::new(0u64);
    chunk.sort_unstable_by(|left, right| {
        comparisons.set(comparisons.get().saturating_add(1));
        compare_event_identifier_records(left, right)
    });
    work.add(comparisons.get())?;
    let leaf = event_identifier_run_leaf(pass);
    write_sorted_scratch_run(scratch, chunk, &leaf)
}

fn read_event_identifier_record(
    reader: &mut File,
) -> Result<Option<EventIdentifierRecord>, RuntimeError> {
    read_fixed_record(
        reader,
        "event identifier index record is truncated",
        "event identifier validation index",
    )
}

fn assign_event_identifier_tokens(
    path: &AnchoredFile,
    source: &EventIdentifierSource,
    tokens: &mut Vec<IdentifierTokenLocation>,
    next_token: &mut u32,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    let (mut file, _) = open_anchored_file_for_read(path)?;
    let mut group = Vec::<EventIdentifierRecord>::new();
    while let Some(record) = read_event_identifier_record(&mut file)? {
        if group
            .first()
            .is_some_and(|first| !same_event_identifier_primary(first, &record))
        {
            assign_event_identifier_group(&mut group, source, tokens, next_token, work)?;
            group.clear();
        }
        group.push(record);
    }
    if !group.is_empty() {
        assign_event_identifier_group(&mut group, source, tokens, next_token, work)?;
    }
    Ok(())
}

fn assign_event_identifier_group(
    group: &mut [EventIdentifierRecord],
    source: &EventIdentifierSource,
    tokens: &mut Vec<IdentifierTokenLocation>,
    next_token: &mut u32,
    work: &mut WorkBudget,
) -> Result<(), RuntimeError> {
    let first_kind = identifier_record_kind(&group[0])?;
    let first_value = source.identifier(identifier_record_offset(&group[0]), first_kind)?;
    let mut collision = false;
    for record in &group[1..] {
        work.add(1)?;
        let kind = identifier_record_kind(record)?;
        let value = source.identifier(identifier_record_offset(record), kind)?;
        if value != first_value {
            collision = true;
            break;
        }
    }
    if !collision {
        if group.len() > 1 && group[0][0] == identifier_namespace(EventStateIdentifierKind::Event) {
            return Err(protocol("event stream must use unique event_id values"));
        }
        let token = *next_token;
        *next_token = next_token
            .checked_add(1)
            .ok_or_else(|| protocol("event identifier token overflow"))?;
        for record in group.iter() {
            tokens.push(IdentifierTokenLocation::new(
                identifier_record_offset(record),
                identifier_record_kind(record)?,
                token,
            )?);
        }
        return Ok(());
    }
    let error = RefCell::new(None);
    let comparisons = Cell::new(0u64);
    group.sort_unstable_by(|left, right| {
        comparisons.set(comparisons.get().saturating_add(1));
        match source.compare(left, right) {
            Ok(ordering) => ordering,
            Err(found) => {
                if error.borrow().is_none() {
                    *error.borrow_mut() = Some(found);
                }
                CmpOrdering::Equal
            }
        }
    });
    work.add(comparisons.get())?;
    if let Some(error) = error.into_inner() {
        return Err(error);
    }
    let mut prior: Option<EventIdentifierRecord> = None;
    let mut token = 0u32;
    for record in group.iter() {
        let equal = match prior.as_ref() {
            Some(prior) => {
                work.add(1)?;
                source.compare(prior, record)? == CmpOrdering::Equal
            }
            None => false,
        };
        if !equal {
            token = *next_token;
            *next_token = next_token
                .checked_add(1)
                .ok_or_else(|| protocol("event identifier token overflow"))?;
        } else if record[0] == identifier_namespace(EventStateIdentifierKind::Event) {
            return Err(protocol("event stream must use unique event_id values"));
        }
        tokens.push(IdentifierTokenLocation::new(
            identifier_record_offset(record),
            identifier_record_kind(record)?,
            token,
        )?);
        prior = Some(*record);
    }
    Ok(())
}

fn normalize_event_identifiers(
    event: &mut EventEnvelope,
    offset: u64,
    tokens: &[IdentifierTokenLocation],
) -> Result<(), RuntimeError> {
    event.try_for_each_state_identifier_mut(|kind, value| {
        *value = event_identifier_token(tokens, offset, kind)?;
        Ok(())
    })
}

fn event_identifier_token(
    tokens: &[IdentifierTokenLocation],
    offset: u64,
    kind: EventStateIdentifierKind,
) -> Result<String, RuntimeError> {
    let key = IdentifierTokenLocation::lookup_key(offset, kind)?;
    let index = tokens
        .binary_search_by_key(&key, |location| location.key())
        .map_err(|_| protocol("event identifier token mapping is incomplete"))?;
    Ok(format!("i{:x}", tokens[index].token))
}

#[cfg(test)]
pub(crate) fn with_event_identifier_digest_collision_for_test<T>(
    operation: impl FnOnce() -> T,
) -> T {
    FORCE_EVENT_IDENTIFIER_DIGEST_COLLISION.with(|forced| {
        let previous = forced.replace(true);
        struct Reset<'a>(&'a Cell<bool>, bool);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _reset = Reset(forced, previous);
        operation()
    })
}

const _: () = assert!(EVENT_IDENTIFIER_SORT_BYTES <= INDEX_WORK_RESERVE);
