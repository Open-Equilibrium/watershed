use std::{fs, path::Path};

const REPLAY_TEST_EVENT_COUNT: u64 = 256;
const REPLAY_TEST_RECORD_BYTES: usize = 256 * 1024;
const REPLAY_TEST_SEGMENT_BYTES: usize = 16 * 1024 * 1024;

#[allow(dead_code)]
pub(crate) fn write_sized_conversation_replay(
    sessions: &Path,
    conversation_id: &str,
    run_session_id: &str,
    total_bytes: usize,
    mut observe: impl FnMut(&str),
) {
    let fixed_prefix_bytes = usize::try_from(REPLAY_TEST_EVENT_COUNT - 1)
        .expect("test event count fits usize")
        * REPLAY_TEST_RECORD_BYTES;
    let final_record_bytes = total_bytes
        .checked_sub(fixed_prefix_bytes)
        .expect("test replay is large enough for its fixed records");
    assert!(final_record_bytes <= 320 * 1024);

    let run = sessions
        .join(conversation_id)
        .join("runs")
        .join(run_session_id);
    fs::create_dir_all(&run).expect("sized replay run directory is created");
    remove_replay_segments(&run);
    let mut segment_ordinal = 1u64;
    let mut segment = String::with_capacity(REPLAY_TEST_SEGMENT_BYTES);
    for sequence in 1..=REPLAY_TEST_EVENT_COUNT {
        let target_bytes = if sequence == REPLAY_TEST_EVENT_COUNT {
            final_record_bytes
        } else {
            REPLAY_TEST_RECORD_BYTES
        };
        let line = sized_replay_event_line(run_session_id, sequence, target_bytes);
        if !segment.is_empty()
            && segment.len().saturating_add(line.len()) > REPLAY_TEST_SEGMENT_BYTES
        {
            write_replay_segment(&run, segment_ordinal, &segment);
            segment_ordinal += 1;
            segment.clear();
        }
        observe(&line);
        segment.push_str(&line);
    }
    write_replay_segment(&run, segment_ordinal, &segment);
}

pub(crate) fn remove_replay_segments(run: &Path) {
    for entry in fs::read_dir(run).expect("sized replay run directory is readable") {
        let entry = entry.expect("sized replay run entry is readable");
        if !entry
            .file_type()
            .expect("sized replay run entry type is readable")
            .is_file()
        {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let numbered_ordinal = name
            .strip_prefix("events.")
            .and_then(|name| name.strip_suffix(".jsonl"));
        if name == "events.jsonl"
            || numbered_ordinal.is_some_and(|ordinal| {
                ordinal.len() == 6 && ordinal.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            fs::remove_file(entry.path()).expect("stale sized replay segment is removed");
        }
    }
}

fn sized_replay_event_line(session_id: &str, sequence: u64, target_bytes: usize) -> String {
    let event_type = if sequence == 1 {
        proto::EventType::SessionStarted
    } else if sequence == REPLAY_TEST_EVENT_COUNT {
        proto::EventType::SessionCompleted
    } else {
        proto::EventType::MetricSample
    };
    let payload = if event_type == proto::EventType::MetricSample {
        serde_json::json!({
            "metric_name": "d094.synthetic",
            "padding": "",
            "value": sequence,
        })
    } else {
        serde_json::json!({"padding": ""})
    };
    let mut event = proto::EventEnvelope::new(
        format!("evt-{sequence:06}"),
        event_type,
        session_id,
        sequence,
        "2026-08-03T00:00:00Z",
        "flow-agent-replay-test",
        payload,
    );
    let base = event
        .canonical_jsonl()
        .expect("sized replay event serializes");
    assert!(base.len() <= target_bytes);
    event.payload["padding"] = serde_json::Value::String("x".repeat(target_bytes - base.len()));
    let line = event
        .canonical_jsonl()
        .expect("sized replay event serializes after padding");
    assert_eq!(line.len(), target_bytes);
    line
}

fn write_replay_segment(run: &Path, ordinal: u64, contents: &str) {
    let leaf = if ordinal == 1 {
        "events.jsonl".to_owned()
    } else {
        format!("events.{ordinal:06}.jsonl")
    };
    fs::write(run.join(leaf), contents).expect("sized replay segment is written");
}
