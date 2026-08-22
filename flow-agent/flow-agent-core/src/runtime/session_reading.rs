pub use crate::runtime::conversations::SessionEventReader;
use crate::runtime::{
    conversations::ensure_in_memory_replay_output_limit,
    types::{EmitMode, HumanFailureStatus, RunOutput, RuntimeError, human_run_status_from_failure},
};
use proto::{EventEnvelope, EventType};
use std::path::Path;

/// Replays one run owned by the addressed conversation.
pub fn replay_conversation_run(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    let mut reader = SessionEventReader::open_conversation_run(
        workspace.as_ref(),
        conversation_id,
        run_session_id,
    )?;
    let path = reader.diagnostic_path().to_owned();
    let mut output = InMemoryReplayOutput::new(emit);
    let summary = replay_conversation_run_with_sink(&mut reader, true, |line| output.push(line))?;
    Ok(RunOutput {
        event_count: summary.event_count,
        failed: summary.failed,
        session_id: run_session_id.to_owned(),
        session_path: path,
        stdout: match emit {
            EmitMode::Jsonl => output.finish(),
            EmitMode::Human => {
                human_run_status_from_failure(run_session_id, "replayed", summary.failure.status())
            }
        },
    })
}

struct InMemoryReplayOutput {
    jsonl: Option<String>,
    output_bytes: usize,
}

impl InMemoryReplayOutput {
    fn new(emit: EmitMode) -> Self {
        Self {
            jsonl: (emit == EmitMode::Jsonl).then(String::new),
            output_bytes: 0,
        }
    }

    fn push(&mut self, line: &str) -> Result<(), RuntimeError> {
        if let Some(jsonl) = &mut self.jsonl {
            self.output_bytes = self.output_bytes.saturating_add(line.len());
            ensure_in_memory_replay_output_limit(self.output_bytes)?;
            jsonl.push_str(line);
        }
        Ok(())
    }

    fn finish(self) -> String {
        self.jsonl.unwrap_or_default()
    }
}

/// Streams one conversation Run as validated canonical JSONL records.
///
/// The callback is invoked once per complete record. The returned [`RunOutput`] contains the
/// validated summary with an empty `stdout`; callers that need in-memory output may use
/// [`replay_conversation_run`], which is bounded in memory.
pub fn replay_conversation_run_streaming(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    mut write_jsonl: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<RunOutput, RuntimeError> {
    let mut reader = SessionEventReader::open_conversation_run(
        workspace.as_ref(),
        conversation_id,
        run_session_id,
    )?;
    let path = reader.diagnostic_path().to_owned();
    let summary = replay_conversation_run_with_sink(&mut reader, false, |line| write_jsonl(line))?;
    Ok(RunOutput {
        event_count: summary.event_count,
        failed: summary.failed,
        session_id: run_session_id.to_owned(),
        session_path: path,
        stdout: String::new(),
    })
}

#[derive(Default)]
struct ReplaySummary {
    event_count: usize,
    failed: bool,
    failure: HumanFailureStatus,
}

impl ReplaySummary {
    fn observe(&mut self, event: &EventEnvelope, retain_failure: bool) {
        self.event_count = self.event_count.saturating_add(1);
        self.failed = event.event_type == EventType::SessionFailed;
        if !retain_failure {
            return;
        }
        self.failure.observe(event);
    }
}

fn replay_conversation_run_with_sink(
    reader: &mut SessionEventReader,
    retain_failure: bool,
    mut write_jsonl: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<ReplaySummary, RuntimeError> {
    let mut summary = ReplaySummary::default();
    reader.visit_verified_after(0, u64::MAX, |event, text| {
        summary.observe(event, retain_failure);
        write_jsonl(text)
    })?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_replay_does_not_limit_unretained_jsonl_bytes() {
        let mut output = InMemoryReplayOutput::new(EmitMode::Human);
        let line = "x".repeat(1024 * 1024);

        for _ in 0..65 {
            output
                .push(&line)
                .expect("human replay does not retain or limit canonical JSONL bytes");
        }
        assert_eq!(output.finish(), "");
    }
}
