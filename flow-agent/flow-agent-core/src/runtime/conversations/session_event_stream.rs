use super::{
    contract::{CONVERSATION_RUNS_DIR, RUN_EVENTS_LEAF},
    conversation_stream::open_anchored_jsonl_segment,
    storage::ConversationScanQuantum,
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredDirectoryIdentity, AnchoredFile, AnchoredWorkspace,
        DirectoryErrorMode, ensure_anchored_real_file, open_anchored_file_for_read,
        open_anchored_runtime_dir_read_only, path_io_error, retry_event_segment_discovery,
        segmented_jsonl_files, segmented_jsonl_path,
    },
    session_authority::{SessionOwnershipObserver, run_ownership_key},
    session_bundle::SessionBundlePaths,
    session_store::workspace_store_path,
    stream_signature::{EVENT_PLAN_DOMAIN, RuntimeStreamSignature, RuntimeStreamSignatureBuilder},
    types::{
        EVENT_STREAM_LIMITS, MAX_CANONICAL_EVENT_BYTES, MAX_SESSION_EVENT_BYTES,
        MAX_SESSION_SEGMENT_BYTES, RuntimeError, SESSION_STORAGE_DIR,
    },
    validate::SessionAppendValidationState,
};
use proto::EventEnvelope;
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_IN_MEMORY_REPLAY_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn ensure_in_memory_replay_output_limit(
    output_bytes: usize,
) -> Result<(), RuntimeError> {
    if output_bytes > MAX_IN_MEMORY_REPLAY_OUTPUT_BYTES {
        return Err(RuntimeError::ReplayOutputLimitExceeded {
            limit_bytes: MAX_IN_MEMORY_REPLAY_OUTPUT_BYTES,
        });
    }
    Ok(())
}

/// Validated, append-only reader for one authoritative session log.
///
/// Each read is bounded by the per-segment and per-session event-data limits. The reader
/// tolerates an incomplete final JSONL line while session ownership is active, rejects
/// mutation of an already observed event, and leaves cursor advancement to the caller.
pub struct SessionEventReader {
    observed_current_segment_bytes: u64,
    observed_segment_count: usize,
    observed_signature: RuntimeStreamSignatureBuilder,
    ownership: SessionOwnershipObserver,
    path: AnchoredFile,
    validation: SessionAppendValidationState,
    workspace_identity: AnchoredDirectoryIdentity,
    workspace_path: PathBuf,
}

impl SessionEventReader {
    pub(crate) fn diagnostic_path(&self) -> &Path {
        self.path.diagnostic_path()
    }

    pub(crate) fn open_flat(workspace: &Path, session_id: &str) -> Result<Self, RuntimeError> {
        if !proto::is_valid_session_id(session_id) {
            return Err(RuntimeError::Usage(format!(
                "invalid session_id {session_id:?}"
            )));
        }
        let workspace_path =
            fs::canonicalize(workspace).map_err(|source| path_io_error(workspace, source))?;
        let workspace = AnchoredWorkspace::open_read_only(&workspace_path)?;
        let session_dir_path = workspace_store_path(&workspace)?.join(SESSION_STORAGE_DIR);
        let sessions = open_anchored_runtime_dir_read_only(&workspace, SESSION_STORAGE_DIR)?
            .ok_or_else(|| RuntimeError::Io {
                path: session_dir_path,
                source: io::Error::from(io::ErrorKind::NotFound),
            })?;
        Self::open_flat_anchored(&workspace, &sessions, session_id)
    }

    pub(super) fn open_flat_anchored(
        workspace: &AnchoredWorkspace,
        sessions: &AnchoredDir,
        session_id: &str,
    ) -> Result<Self, RuntimeError> {
        if !proto::is_valid_session_id(session_id) {
            return Err(RuntimeError::Usage(format!(
                "invalid session_id {session_id:?}"
            )));
        }
        let path = SessionBundlePaths::events_in(sessions, session_id);
        ensure_anchored_real_file(&path)?;
        Ok(Self {
            observed_current_segment_bytes: 0,
            observed_segment_count: 0,
            observed_signature: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            ownership: SessionOwnershipObserver::open_anchored(workspace, session_id)?,
            path,
            validation: SessionAppendValidationState::empty(session_id),
            workspace_identity: workspace.identity(),
            workspace_path: workspace.canonical_path().to_owned(),
        })
    }

    pub(super) fn open_conversation_run_raw(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
    ) -> Result<Self, RuntimeError> {
        let workspace_path =
            fs::canonicalize(workspace).map_err(|source| path_io_error(workspace, source))?;
        let workspace = AnchoredWorkspace::open_read_only(&workspace_path)?;
        let ownership_key = run_ownership_key(conversation_id, run_session_id);
        let ownership = SessionOwnershipObserver::open_anchored(&workspace, &ownership_key)?;
        let session_dir_path = workspace_store_path(&workspace)?.join(SESSION_STORAGE_DIR);
        let sessions = open_anchored_runtime_dir_read_only(&workspace, SESSION_STORAGE_DIR)?
            .ok_or_else(|| RuntimeError::Io {
                path: session_dir_path,
                source: io::Error::from(io::ErrorKind::NotFound),
            })?;
        let Some(conversation) =
            sessions.child(conversation_id, false, DirectoryErrorMode::Protocol)?
        else {
            if conversation_id == run_session_id {
                return Self::open_flat_anchored(&workspace, &sessions, run_session_id);
            }
            return Err(RuntimeError::Io {
                path: sessions.path.join(conversation_id),
                source: io::Error::from(io::ErrorKind::NotFound),
            });
        };
        let runs = conversation
            .child(CONVERSATION_RUNS_DIR, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| RuntimeError::Io {
                path: conversation.path.join(CONVERSATION_RUNS_DIR),
                source: io::Error::from(io::ErrorKind::NotFound),
            })?;
        let run = runs
            .child(run_session_id, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| RuntimeError::Io {
                path: runs.path.join(run_session_id),
                source: io::Error::from(io::ErrorKind::NotFound),
            })?;
        let path = run.file(RUN_EVENTS_LEAF);
        ensure_anchored_real_file(&path)?;
        Ok(Self {
            observed_current_segment_bytes: 0,
            observed_segment_count: 0,
            observed_signature: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            ownership,
            path,
            validation: SessionAppendValidationState::empty(run_session_id),
            workspace_identity: workspace.identity(),
            workspace_path: workspace.canonical_path().to_owned(),
        })
    }

    /// Reads every complete committed event whose sequence is greater than `cursor`.
    ///
    /// The caller must advance `cursor` only after successfully processing each returned
    /// event. Repeating this call is safe after a processing failure. The returned canonical
    /// records are bounded in memory; use [`Self::visit_verified_after`] for larger
    /// callback-streamed reads.
    pub fn read_after(&mut self, cursor: u64) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let mut output_bytes = 0usize;
        let mut events = Vec::new();
        self.visit_verified_after_with(cursor, u64::MAX, true, |event, line| {
            output_bytes = output_bytes.saturating_add(line.len());
            ensure_in_memory_replay_output_limit(output_bytes)?;
            events.push(event.clone());
            Ok(())
        })?;
        Ok(events)
    }

    /// Verifies the complete authoritative log while visiting a bounded committed range.
    ///
    /// Segments are validated one at a time. `visit` receives each event and its canonical
    /// JSONL record only when its sequence is in `(cursor, through_sequence]`; the reader's
    /// append-only state advances only after the complete authoritative scan succeeds.
    pub fn visit_verified_after(
        &mut self,
        cursor: u64,
        through_sequence: u64,
        visit: impl FnMut(&EventEnvelope, &str) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        self.visit_verified_after_with(cursor, through_sequence, false, visit)
    }

    fn visit_verified_after_with(
        &mut self,
        cursor: u64,
        through_sequence: u64,
        defer_inactive_error_until_validated: bool,
        mut visit: impl FnMut(&EventEnvelope, &str) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let expected_prefix = self.observed_signature.signature();
        let mut retried_inactive_final = false;
        loop {
            let segments = retry_event_segment_discovery(|| {
                segmented_jsonl_files(&self.path, EVENT_STREAM_LIMITS)
            })?;
            let final_segment = segments
                .last()
                .expect("segmented JSONL inventory always includes its base file");
            let mut final_segment = Some(open_anchored_jsonl_segment(
                final_segment,
                MAX_SESSION_SEGMENT_BYTES,
            )?);
            let has_partial_line = final_segment
                .as_ref()
                .expect("final segment snapshot is present")
                .has_partial_line;
            let is_empty = segments.len() == 1
                && final_segment
                    .as_ref()
                    .expect("final segment snapshot is present")
                    .is_empty;
            let ownership_inactive =
                (has_partial_line || is_empty) && !self.session_ownership_active()?;
            let inactive_partial = has_partial_line && ownership_inactive;
            let inactive_empty = is_empty && ownership_inactive;
            if (inactive_partial || inactive_empty) && !retried_inactive_final {
                retried_inactive_final = true;
                continue;
            }
            if inactive_partial && !defer_inactive_error_until_validated {
                return Err(self.inactive_partial());
            }
            if inactive_empty && !defer_inactive_error_until_validated {
                return Err(self.inactive_empty());
            }

            let session_id = self
                .validation
                .expected_session_id
                .as_deref()
                .expect("session readers always validate one session");
            let mut validation = SessionAppendValidationState::empty(session_id);
            let mut signature = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
            let mut total_bytes = 0u64;
            let mut final_complete_bytes = 0u64;
            let mut quantum = ConversationScanQuantum::new();
            for (index, segment) in segments.iter().enumerate() {
                let segment = if index + 1 == segments.len() {
                    final_segment
                        .take()
                        .expect("final segment snapshot is consumed once")
                } else {
                    open_anchored_jsonl_segment(segment, MAX_SESSION_SEGMENT_BYTES)?
                };
                total_bytes = total_bytes.saturating_add(segment.stored_bytes);
                if total_bytes > MAX_SESSION_EVENT_BYTES {
                    return Err(RuntimeError::Protocol(format!(
                        "{} session event data exceeds max {MAX_SESSION_EVENT_BYTES}",
                        self.path.diagnostic_path().display()
                    )));
                }
                let complete_bytes = segment.scan(
                    MAX_CANONICAL_EVENT_BYTES,
                    index + 1 != segments.len(),
                    &mut quantum,
                    |text| {
                        self.visit_verified_record(
                            &mut validation,
                            &mut signature,
                            &expected_prefix,
                            text,
                            cursor,
                            through_sequence,
                            &mut visit,
                        )
                    },
                )?;
                if index + 1 == segments.len() {
                    final_complete_bytes = complete_bytes;
                }
            }
            quantum.finish();
            if signature.record_count < expected_prefix.record_count {
                return Err(self.changed_outside_append_only());
            }
            if has_partial_line && validation.terminal_line.is_some() {
                return Err(RuntimeError::Protocol(format!(
                    "{} contains a partial line after a terminal event",
                    self.path.diagnostic_path().display()
                )));
            }
            if inactive_partial {
                return Err(self.inactive_partial());
            }
            if inactive_empty {
                return Err(self.inactive_empty());
            }
            self.ensure_cursor(cursor, validation.previous_sequence)?;
            self.observed_current_segment_bytes = final_complete_bytes;
            self.observed_segment_count = segments.len();
            self.observed_signature = signature;
            self.validation = validation;
            return Ok(());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_verified_record(
        &self,
        validation: &mut SessionAppendValidationState,
        signature: &mut RuntimeStreamSignatureBuilder,
        expected_prefix: &RuntimeStreamSignature,
        text: &str,
        cursor: u64,
        through_sequence: u64,
        visit: &mut impl FnMut(&EventEnvelope, &str) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        validation.validate_appended_with(self.path.diagnostic_path(), text, |event| {
            signature.push(text.as_bytes());
            if signature.record_count == expected_prefix.record_count
                && signature.signature() != *expected_prefix
            {
                return Err(self.changed_outside_append_only());
            }
            if event.sequence > cursor && event.sequence <= through_sequence {
                visit(event, text)?;
            }
            Ok(())
        })
    }

    /// Reads only the newly appended complete suffix after an initial verified read.
    ///
    /// This materializing compatibility path is bounded to complete canonical bytes. Streaming
    /// receivers should use [`Self::visit_incremental_after`].
    pub fn read_incremental_after(
        &mut self,
        cursor: u64,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        self.read_incremental_after_with(cursor, &mut || {})
    }

    /// Visits only the newly appended complete suffix after an initial verified read.
    ///
    /// The callback receives committed events in `(cursor, through_sequence]` with their
    /// canonical JSONL records. Call [`Self::visit_verified_after`] after the producer closes
    /// to verify the complete authoritative log before treating delivery as final.
    pub fn visit_incremental_after(
        &mut self,
        cursor: u64,
        through_sequence: u64,
        visit: impl FnMut(&EventEnvelope, &str) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        self.visit_incremental_after_with(cursor, through_sequence, &mut || {}, visit)
    }

    pub(crate) fn read_incremental_after_with(
        &mut self,
        cursor: u64,
        after_read: &mut impl FnMut(),
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let mut output_bytes = 0usize;
        let mut events = Vec::new();
        self.visit_incremental_after_with(cursor, u64::MAX, after_read, |event, line| {
            output_bytes = output_bytes.saturating_add(line.len());
            ensure_in_memory_replay_output_limit(output_bytes)?;
            events.push(event.clone());
            Ok(())
        })?;
        Ok(events)
    }

    fn visit_incremental_after_with(
        &mut self,
        cursor: u64,
        through_sequence: u64,
        after_read: &mut impl FnMut(),
        mut visit: impl FnMut(&EventEnvelope, &str) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        if self.validation.line_count == 0 || cursor < self.validation.previous_sequence {
            return self.visit_verified_after(cursor, through_sequence, visit);
        }
        let target_sequence = cursor.max(through_sequence);
        if self.validation.previous_sequence >= target_sequence {
            return self.ensure_cursor(cursor, self.validation.previous_sequence);
        }
        let mut retried_inactive_partial = false;
        loop {
            let segments = self.incremental_segments()?;
            if segments.len() < self.observed_segment_count || self.observed_segment_count == 0 {
                return Err(self.changed_outside_append_only());
            }
            let prior_final_index = self.observed_segment_count - 1;
            let final_index = segments.len() - 1;
            let final_offset = if final_index == prior_final_index {
                self.observed_current_segment_bytes
            } else {
                0
            };
            let (final_suffix, _) =
                self.read_incremental_segment(&segments[final_index], final_offset, 0)?;
            let final_complete_len = complete_jsonl_prefix_len(&final_suffix);
            let has_partial_line = final_complete_len != final_suffix.len();
            let inactive_partial = has_partial_line && !self.session_ownership_active()?;
            if inactive_partial && !retried_inactive_partial {
                retried_inactive_partial = true;
                continue;
            }
            after_read();
            let mut validation = self.validation.clone();
            let mut signature = self.observed_signature.clone();
            let mut discovered_bytes = 0u64;
            let mut current_segment_count = self.observed_segment_count;
            let mut current_segment_bytes = self.observed_current_segment_bytes;
            for (index, segment) in segments.iter().enumerate().skip(prior_final_index) {
                let offset = if index == prior_final_index {
                    self.observed_current_segment_bytes
                } else {
                    0
                };
                let (segment_suffix, metadata_len) = if index == final_index {
                    (
                        final_suffix.as_slice(),
                        final_offset
                            .saturating_add(u64::try_from(final_suffix.len()).unwrap_or(u64::MAX)),
                    )
                } else {
                    let (bytes, metadata_len) =
                        self.read_incremental_segment(segment, offset, discovered_bytes)?;
                    discovered_bytes = discovered_bytes
                        .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                    if metadata_len == 0 || complete_jsonl_prefix_len(&bytes) != bytes.len() {
                        return Err(RuntimeError::Protocol(format!(
                            "{} non-final segment must end with LF",
                            segment.diagnostic_path().display()
                        )));
                    }
                    let consumed = self.visit_incremental_records(
                        &mut validation,
                        &mut signature,
                        &bytes,
                        cursor,
                        through_sequence,
                        target_sequence,
                        &mut visit,
                    )?;
                    current_segment_count = index + 1;
                    current_segment_bytes =
                        offset.saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
                    if validation.previous_sequence >= target_sequence {
                        break;
                    }
                    continue;
                };
                discovered_bytes = discovered_bytes
                    .saturating_add(u64::try_from(segment_suffix.len()).unwrap_or(u64::MAX));
                let observed_event_bytes =
                    u64::try_from(self.observed_signature.byte_count).unwrap_or(u64::MAX);
                if observed_event_bytes.saturating_add(discovered_bytes) > MAX_SESSION_EVENT_BYTES {
                    return Err(RuntimeError::Protocol(format!(
                        "{} session event data exceeds max {MAX_SESSION_EVENT_BYTES}",
                        self.path.diagnostic_path().display()
                    )));
                }
                let complete_len = complete_jsonl_prefix_len(segment_suffix);
                if index != final_index
                    && (metadata_len == 0 || complete_len != segment_suffix.len())
                {
                    return Err(RuntimeError::Protocol(format!(
                        "{} non-final segment must end with LF",
                        segment.diagnostic_path().display()
                    )));
                }
                let consumed = self.visit_incremental_records(
                    &mut validation,
                    &mut signature,
                    &segment_suffix[..complete_len],
                    cursor,
                    through_sequence,
                    target_sequence,
                    &mut visit,
                )?;
                current_segment_count = index + 1;
                current_segment_bytes =
                    offset.saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
                if validation.previous_sequence >= target_sequence {
                    break;
                }
            }
            if has_partial_line && validation.terminal_line.is_some() {
                return Err(RuntimeError::Protocol(format!(
                    "{} contains a partial line after a terminal event",
                    self.path.diagnostic_path().display()
                )));
            }
            if inactive_partial {
                return Err(self.inactive_partial());
            }
            self.ensure_cursor(cursor, validation.previous_sequence)?;
            self.observed_current_segment_bytes = current_segment_bytes;
            self.observed_segment_count = current_segment_count;
            self.observed_signature = signature;
            self.validation = validation;
            return Ok(());
        }
    }

    fn read_incremental_segment(
        &self,
        segment: &AnchoredFile,
        offset: u64,
        discovered_bytes: u64,
    ) -> Result<(Vec<u8>, u64), RuntimeError> {
        let (mut file, metadata) = open_anchored_file_for_read(segment)?;
        if metadata.len() < offset {
            return Err(self.changed_outside_append_only());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
        let remaining_segment_bytes = MAX_SESSION_SEGMENT_BYTES.saturating_sub(offset);
        let observed_event_bytes =
            u64::try_from(self.observed_signature.byte_count).unwrap_or(u64::MAX);
        let remaining_event_bytes = MAX_SESSION_EVENT_BYTES
            .saturating_sub(observed_event_bytes)
            .saturating_sub(discovered_bytes);
        let mut bytes = Vec::new();
        file.take(
            remaining_segment_bytes
                .min(remaining_event_bytes)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
        let read_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if read_bytes > remaining_segment_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} read size exceeds max {MAX_SESSION_SEGMENT_BYTES}",
                segment.diagnostic_path().display()
            )));
        }
        if observed_event_bytes
            .saturating_add(discovered_bytes)
            .saturating_add(read_bytes)
            > MAX_SESSION_EVENT_BYTES
        {
            return Err(RuntimeError::Protocol(format!(
                "{} session event data exceeds max {MAX_SESSION_EVENT_BYTES}",
                self.path.diagnostic_path().display()
            )));
        }
        Ok((bytes, metadata.len()))
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_incremental_records(
        &self,
        validation: &mut SessionAppendValidationState,
        signature: &mut RuntimeStreamSignatureBuilder,
        bytes: &[u8],
        cursor: u64,
        through_sequence: u64,
        target_sequence: u64,
        visit: &mut impl FnMut(&EventEnvelope, &str) -> Result<(), RuntimeError>,
    ) -> Result<usize, RuntimeError> {
        let mut consumed = 0usize;
        for record in bytes.split_inclusive(|byte| *byte == b'\n') {
            if validation.previous_sequence >= target_sequence {
                break;
            }
            let text = std::str::from_utf8(record).map_err(|source| {
                RuntimeError::Protocol(format!(
                    "{} is not valid UTF-8: {source}",
                    self.path.diagnostic_path().display()
                ))
            })?;
            validation.validate_appended_with(self.path.diagnostic_path(), text, |event| {
                signature.push(record);
                if event.sequence > cursor && event.sequence <= through_sequence {
                    visit(event, text)?;
                }
                Ok(())
            })?;
            consumed = consumed.saturating_add(record.len());
        }
        Ok(consumed)
    }

    pub(crate) fn incremental_segments(&self) -> Result<Vec<AnchoredFile>, RuntimeError> {
        let observed = u64::try_from(self.observed_segment_count).unwrap_or(u64::MAX);
        let mut segments = Vec::new();
        for ordinal in 1..=observed {
            let segment = segmented_jsonl_path(&self.path, ordinal)?;
            ensure_anchored_real_file(&segment)?;
            segments.push(segment);
        }
        for ordinal in observed.saturating_add(1)..=EVENT_STREAM_LIMITS.max_segments {
            let segment = segmented_jsonl_path(&self.path, ordinal)?;
            match segment.metadata() {
                Ok(_) => ensure_anchored_real_file(&segment)?,
                Err(RuntimeError::Io { source, .. })
                    if source.kind() == io::ErrorKind::NotFound =>
                {
                    break;
                }
                Err(error) => return Err(error),
            }
            segments.push(segment);
        }
        Ok(segments)
    }

    pub(crate) fn session_ownership_active(&self) -> Result<bool, RuntimeError> {
        if !self.workspace_identity_is_current()? || !self.ownership.is_active()? {
            return Ok(false);
        }
        self.workspace_identity_is_current()
    }

    fn workspace_identity_is_current(&self) -> Result<bool, RuntimeError> {
        let current = match AnchoredWorkspace::open_read_only(&self.workspace_path) {
            Ok(current) => current,
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        Ok(current.identity() == self.workspace_identity)
    }

    pub(crate) fn inactive_partial(&self) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "{} contains an incomplete final JSONL line without active session ownership",
            self.path.diagnostic_path().display()
        ))
    }

    pub(crate) fn inactive_empty(&self) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "{} is empty without active session ownership",
            self.path.diagnostic_path().display()
        ))
    }

    pub(crate) fn ensure_cursor(
        &self,
        cursor: u64,
        latest_sequence: u64,
    ) -> Result<(), RuntimeError> {
        if cursor <= latest_sequence {
            return Ok(());
        }
        Err(RuntimeError::Protocol(format!(
            "{} no longer contains processed sequence {cursor}",
            self.path.diagnostic_path().display()
        )))
    }

    pub(crate) fn changed_outside_append_only(&self) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "{} changed outside append-only session semantics",
            self.path.diagnostic_path().display()
        ))
    }
}

pub fn complete_jsonl_prefix_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline_index| newline_index + 1)
}
