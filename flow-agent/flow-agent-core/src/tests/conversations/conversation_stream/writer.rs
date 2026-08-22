use super::super::super::helpers::empty_workspace;
use super::padded_json_value;
use crate::runtime::conversations::{
    MAX_CONVERSATION_RECORD_BYTES, MAX_CONVERSATION_SEGMENT_BYTES, append_jsonl, read_jsonl,
};
use std::fs;

#[test]
fn conversation_rotation_budget() {
    let workspace = empty_workspace("conversation-rotation-budget");
    let stream = workspace.join("history.jsonl");
    fs::write(&stream, "").expect("stream is created");
    let maximum_stored_record = MAX_CONVERSATION_RECORD_BYTES + 1;
    let full_records =
        usize::try_from(MAX_CONVERSATION_SEGMENT_BYTES).unwrap() / maximum_stored_record;
    let remainder = usize::try_from(MAX_CONVERSATION_SEGMENT_BYTES).unwrap()
        - full_records * maximum_stored_record;
    let padded_value = |canonical_bytes: usize| {
        let empty = proto::canonical_json(&serde_json::json!({"padding": ""})).unwrap();
        assert!(canonical_bytes >= empty.len());
        serde_json::json!({"padding": "x".repeat(canonical_bytes - empty.len())})
    };
    for _ in 0..full_records {
        append_jsonl(&stream, &padded_value(MAX_CONVERSATION_RECORD_BYTES))
            .expect("maximum record appends");
    }
    append_jsonl(&stream, &padded_value(remainder - 1)).expect("segment fills exactly");
    assert_eq!(
        fs::metadata(&stream).unwrap().len(),
        MAX_CONVERSATION_SEGMENT_BYTES
    );

    append_jsonl(&stream, &serde_json::json!({"next": true}))
        .expect("next complete record rotates");
    let rotated = workspace.join("history.000002.jsonl");
    assert!(rotated.is_file());
    assert_eq!(
        read_jsonl::<serde_json::Value>(&stream)
            .expect("rotated stream replays")
            .len(),
        full_records + 2
    );
}

#[test]
fn conversation_record_budget() {
    let workspace = empty_workspace("conversation-record-budget");
    let stream = workspace.join("records.jsonl");
    fs::write(&stream, "").expect("stream is created");

    append_jsonl(&stream, &padded_json_value(MAX_CONVERSATION_RECORD_BYTES))
        .expect("256 KiB canonical record is accepted");
    let error = append_jsonl(
        &stream,
        &padded_json_value(MAX_CONVERSATION_RECORD_BYTES + 1),
    )
    .expect_err("256 KiB plus one byte is rejected");
    assert!(error.to_string().contains("byte limit"), "{error}");
}
