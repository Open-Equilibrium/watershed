use super::super::super::helpers::empty_workspace;
use super::padded_json_value;
use crate::runtime::conversations::{
    MAX_CONVERSATION_RECORD_BYTES, MAX_CONVERSATION_SCAN_BYTES, MAX_CONVERSATION_SCAN_RECORDS,
    MAX_CONVERSATION_SEGMENT_BYTES, append_jsonl, read_jsonl, read_jsonl_quantum,
};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

#[test]
fn conversation_scan_count_budget() {
    let workspace = empty_workspace("conversation-scan-count-budget");
    let stream = workspace.join("records.jsonl");
    let line = b"{\"value\":0}\n";
    fs::write(&stream, line.repeat(MAX_CONVERSATION_SCAN_RECORDS + 1))
        .expect("count-boundary stream is written");

    let (first, cursor) =
        read_jsonl_quantum::<serde_json::Value>(&stream, None).expect("first scan quantum reads");
    assert_eq!(first.len(), MAX_CONVERSATION_SCAN_RECORDS);
    let (second, cursor) = read_jsonl_quantum::<serde_json::Value>(&stream, cursor)
        .expect("second scan quantum reads");
    assert_eq!(second.len(), 1);
    assert!(cursor.is_none());
}

#[test]
fn conversation_scan_rejects_a_cursor_after_segment_truncation() {
    let workspace = empty_workspace("conversation-scan-truncated-cursor");
    let stream = workspace.join("records.jsonl");
    let line = b"{\"value\":0}\n";
    fs::write(&stream, line.repeat(MAX_CONVERSATION_SCAN_RECORDS + 1))
        .expect("count-boundary stream is written");
    let (_, cursor) =
        read_jsonl_quantum::<serde_json::Value>(&stream, None).expect("first quantum reads");
    fs::write(&stream, "").expect("stream is truncated below the cursor");

    let error = read_jsonl_quantum::<serde_json::Value>(&stream, cursor)
        .expect_err("truncation below an observed cursor is rejected");
    assert!(error.to_string().contains("changed while it was scanned"));
}

fn exact_byte_quantum_stream(workspace: &Path) -> PathBuf {
    let stream = workspace.join("records.jsonl");
    fs::write(&stream, "").expect("stream is created");
    let stored_record_bytes = MAX_CONVERSATION_RECORD_BYTES;
    let record_count = usize::try_from(MAX_CONVERSATION_SCAN_BYTES).unwrap() / stored_record_bytes;
    for _ in 0..record_count {
        append_jsonl(&stream, &padded_json_value(stored_record_bytes - 1))
            .expect("byte-boundary record appends");
    }
    assert_eq!(
        fs::metadata(&stream).unwrap().len(),
        MAX_CONVERSATION_SCAN_BYTES
    );
    append_jsonl(&stream, &serde_json::json!({"next": true})).expect("next record rotates");
    stream
}

#[test]
fn conversation_scan_byte_budget() {
    let workspace = empty_workspace("conversation-scan-byte-budget");
    let stream = exact_byte_quantum_stream(&workspace);

    let (first, cursor) =
        read_jsonl_quantum::<serde_json::Value>(&stream, None).expect("exact 16 MiB quantum reads");
    assert_eq!(first.len(), 64);
    let (second, cursor) = read_jsonl_quantum::<serde_json::Value>(&stream, cursor)
        .expect("record behind the byte cursor reads next");
    assert_eq!(second, [serde_json::json!({"next": true})]);
    assert!(cursor.is_none());
}

#[test]
fn conversation_io_buffer_budget() {
    crate::runtime::m11_budget_evidence::verify_conversation_operation_boundaries_for_test(
        &empty_workspace("conversation-io-buffer-budget"),
    )
    .expect("real migration and replay stay within their finite scan and I/O bounds");
}

#[test]
fn conversation_stream_reader_rejects_malformed_inventory_and_framing() {
    let workspace = empty_workspace("conversation-stream-validation");
    let cases = workspace.join("stream-cases");
    fs::create_dir_all(&cases).expect("case root is created");

    let missing_dir = cases.join("missing");
    fs::create_dir(&missing_dir).expect("missing case directory is created");
    assert!(read_jsonl::<serde_json::Value>(&missing_dir.join("records.jsonl")).is_err());

    let wrong_name_dir = cases.join("wrong-name");
    fs::create_dir(&wrong_name_dir).expect("wrong-name case directory is created");
    fs::write(wrong_name_dir.join("records.txt"), b"{}\n").expect("fixture writes");
    assert!(read_jsonl::<serde_json::Value>(&wrong_name_dir.join("records.txt")).is_err());

    let invalid_ordinal_dir = cases.join("invalid-ordinal");
    fs::create_dir(&invalid_ordinal_dir).expect("invalid ordinal case directory is created");
    fs::write(invalid_ordinal_dir.join("records.jsonl"), b"{}\n").expect("fixture writes");
    fs::write(invalid_ordinal_dir.join("records.2.jsonl"), b"{}\n").expect("fixture writes");
    assert!(read_jsonl::<serde_json::Value>(&invalid_ordinal_dir.join("records.jsonl")).is_err());

    let gap_dir = cases.join("gap");
    fs::create_dir(&gap_dir).expect("gap case directory is created");
    fs::write(gap_dir.join("records.jsonl"), b"{}\n").expect("fixture writes");
    fs::write(gap_dir.join("records.000003.jsonl"), b"{}\n").expect("fixture writes");
    assert!(read_jsonl::<serde_json::Value>(&gap_dir.join("records.jsonl")).is_err());

    let empty_non_final_dir = cases.join("empty-non-final");
    fs::create_dir(&empty_non_final_dir).expect("empty non-final directory is created");
    fs::write(empty_non_final_dir.join("records.jsonl"), "").expect("empty base writes");
    fs::write(empty_non_final_dir.join("records.000002.jsonl"), b"{}\n")
        .expect("second segment writes");
    assert!(read_jsonl::<serde_json::Value>(&empty_non_final_dir.join("records.jsonl")).is_err());

    let non_file_dir = cases.join("non-file");
    fs::create_dir(&non_file_dir).expect("non-file case directory is created");
    fs::create_dir(non_file_dir.join("records.jsonl")).expect("directory fixture is created");
    assert!(read_jsonl::<serde_json::Value>(&non_file_dir.join("records.jsonl")).is_err());

    let oversized_dir = cases.join("oversized-segment");
    fs::create_dir(&oversized_dir).expect("oversized case directory is created");
    let oversized = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(oversized_dir.join("records.jsonl"))
        .expect("sparse fixture opens");
    oversized
        .set_len(MAX_CONVERSATION_SEGMENT_BYTES + 1)
        .expect("sparse fixture grows");
    drop(oversized);
    assert!(read_jsonl::<serde_json::Value>(&oversized_dir.join("records.jsonl")).is_err());

    for (name, bytes) in [
        ("crlf", b"{}\r\n".as_slice()),
        ("unterminated", b"{}".as_slice()),
        ("noncanonical", b"{\"b\":1,\"a\":2}\n".as_slice()),
    ] {
        let directory = cases.join(name);
        fs::create_dir(&directory).expect("framing case directory is created");
        let path = directory.join("records.jsonl");
        fs::write(&path, bytes).expect("framing fixture writes");
        assert!(read_jsonl::<serde_json::Value>(&path).is_err());
    }

    let oversized_record_dir = cases.join("oversized-record");
    fs::create_dir(&oversized_record_dir).expect("oversized record directory is created");
    let mut oversized_record = vec![b' '; MAX_CONVERSATION_RECORD_BYTES + 1];
    oversized_record.push(b'\n');
    let oversized_record_path = oversized_record_dir.join("records.jsonl");
    fs::write(&oversized_record_path, oversized_record).expect("oversized record writes");
    assert!(read_jsonl::<serde_json::Value>(&oversized_record_path).is_err());
}

#[test]
fn conversation_stream_reader_bounds_unterminated_oversized_record() {
    let workspace = empty_workspace("conversation-unterminated-oversized-record");
    let stream = workspace.join("records.jsonl");
    fs::write(&stream, vec![b' '; MAX_CONVERSATION_RECORD_BYTES + 2])
        .expect("unterminated oversized record writes");

    let error = read_jsonl::<serde_json::Value>(&stream)
        .expect_err("an oversized record is rejected before framing validation");
    assert!(error.to_string().contains("record exceeds its byte limit"));
}
