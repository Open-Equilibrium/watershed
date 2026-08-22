use crate::runtime::{
    responses::{
        MAX_RESPONSES_DECODED_STREAM_BYTES, MAX_RESPONSES_EVENT_BYTES, MAX_RESPONSES_EVENTS,
        MAX_RESPONSES_LINE_BYTES, ParsedSseStream, SseDecoder,
    },
    types::RuntimeError,
};

fn parse_sse_stream(source: impl AsRef<str>) -> Result<Vec<serde_json::Value>, RuntimeError> {
    Ok(parse_sse_stream_with_metadata(source)?.values)
}

fn parse_sse_stream_with_metadata(
    source: impl AsRef<str>,
) -> Result<ParsedSseStream, RuntimeError> {
    let mut decoder = SseDecoder::default();
    decoder.push(source.as_ref().as_bytes())?;
    decoder.finish()
}

fn sse_event_with_size(target: usize) -> String {
    const CHUNK_BYTES: usize = 128 * 1024;
    assert!(target >= 3);
    let space_chunks = (target - 2).div_ceil(CHUNK_BYTES + 1);
    let mut whitespace = target - 2 - space_chunks;
    let mut event = String::new();
    for _ in 0..space_chunks {
        let bytes = whitespace.min(CHUNK_BYTES);
        event.push_str("data:");
        event.push_str(&"\t".repeat(bytes));
        event.push('\n');
        whitespace -= bytes;
    }
    assert_eq!(whitespace, 0);
    event.push_str("data:{}\n\n");
    event
}

fn sse_array_event_with_canonical_size(target: usize) -> String {
    const ITEMS_PER_LINE: usize = 32_000;
    assert!(target >= 5);
    let item_count = (target - 1) / 4;
    let base_size = 4 * item_count + 1;
    let first_item_length = target - base_size + 1;
    let mut event = format!("data:[\"{}\",\n", "x".repeat(first_item_length));

    for index in 1..item_count {
        if (index - 1) % ITEMS_PER_LINE == 0 {
            event.push_str("data:");
        }
        event.push_str("\"x\"");
        if index + 1 != item_count {
            event.push(',');
        } else {
            event.push(']');
        }
        if index % ITEMS_PER_LINE == 0 || index + 1 == item_count {
            event.push('\n');
        }
    }
    event.push('\n');
    event
}

#[test]
fn responses_line_budget() {
    let accepted = format!(":{}\n\n", "x".repeat(MAX_RESPONSES_LINE_BYTES - 1));
    assert!(parse_sse_stream(accepted).is_ok());
    let rejected = format!(":{}\n\n", "x".repeat(MAX_RESPONSES_LINE_BYTES));
    assert!(parse_sse_stream(rejected).is_err());
}

#[test]
fn responses_line_budget_rejects_before_retaining_an_oversized_fragment() {
    let mut decoder = SseDecoder::default();
    let oversized = vec![b'x'; 4 * 1024 * 1024];

    assert!(decoder.push(&oversized).is_err());
    assert!(decoder.pending_line_bytes() <= MAX_RESPONSES_LINE_BYTES + 1);
}

#[test]
fn responses_event_budget() {
    assert!(parse_sse_stream(sse_event_with_size(MAX_RESPONSES_EVENT_BYTES)).is_ok());
    assert!(parse_sse_stream(sse_event_with_size(MAX_RESPONSES_EVENT_BYTES + 1)).is_err());
}

#[test]
fn responses_decoded_stream_budget_is_checked_before_retention() {
    const BULK_ITEM_BYTES: usize = 1_000_000;
    let bulk_event = sse_array_event_with_canonical_size(BULK_ITEM_BYTES);
    let exact_count = MAX_RESPONSES_DECODED_STREAM_BYTES / BULK_ITEM_BYTES;
    let remainder = MAX_RESPONSES_DECODED_STREAM_BYTES % BULK_ITEM_BYTES;
    let remainder_event = sse_array_event_with_canonical_size(remainder);
    let mut decoder = SseDecoder::default();

    for _ in 0..exact_count {
        decoder
            .push(bulk_event.as_bytes())
            .expect("bounded bulk event");
    }
    decoder
        .push(remainder_event.as_bytes())
        .expect("exact decoded stream budget");
    assert_eq!(
        decoder.decoded_stream_bytes(),
        MAX_RESPONSES_DECODED_STREAM_BYTES
    );
    assert_eq!(decoder.retained_value_count(), exact_count + 1);

    assert!(decoder.push(b"data:true\n\n").is_err());
    assert_eq!(
        decoder.decoded_stream_bytes(),
        MAX_RESPONSES_DECODED_STREAM_BYTES
    );
    assert_eq!(decoder.retained_value_count(), exact_count + 1);
}

#[test]
fn responses_raw_event_data_cannot_bypass_the_stream_budget() {
    let event = sse_event_with_size(MAX_RESPONSES_EVENT_BYTES);
    let mut decoder = SseDecoder::default();
    let error = (0..=MAX_RESPONSES_DECODED_STREAM_BYTES / MAX_RESPONSES_EVENT_BYTES)
        .find_map(|_| decoder.push(event.as_bytes()).err())
        .expect("raw event data above the aggregate limit is rejected");
    assert!(error.to_string().contains("response stream"), "{error}");
}

#[test]
fn responses_event_count_budget() {
    let mut accepted = "data:{}\n\n".repeat(MAX_RESPONSES_EVENTS);
    assert_eq!(
        parse_sse_stream(&accepted).expect("bounded events").len(),
        MAX_RESPONSES_EVENTS
    );
    accepted.push_str("data:{}\n\n");
    assert!(parse_sse_stream(accepted).is_err());
}

#[test]
fn responses_terminal_sentinel_counts_as_dispatched_event() {
    let parsed = parse_sse_stream_with_metadata("data:{}\n\ndata:[DONE]\n\n")
        .expect("bounded terminal stream");
    assert_eq!(parsed.values, vec![serde_json::json!({})]);
    assert_eq!(parsed.dispatched_events, 2);
    assert!(parsed.terminal_sentinel);
}

#[test]
fn responses_reject_data_after_the_terminal_sentinel() {
    let trailing_data = parse_sse_stream("data:[DONE]\n\ndata:{}\n\n")
        .expect_err("data after the terminal sentinel is rejected");
    assert!(
        trailing_data
            .to_string()
            .contains("data after its terminal sentinel")
    );
}

#[test]
fn responses_reject_duplicate_json_object_keys() {
    let error = parse_sse_stream(
        "data:{\"type\":\"response.created\",\"type\":\"response.completed\"}\n\n",
    )
    .expect_err("ambiguous SSE JSON is rejected");

    assert!(
        error.to_string().contains("duplicate JSON object key"),
        "{error}"
    );
}

#[test]
fn responses_incremental_chunk_boundaries_preserve_sse_semantics() {
    let mut decoder = SseDecoder::default();
    decoder.push(b"data: {\"first\":").expect("first chunk");
    decoder.push(b"1}\r").expect("split carriage return");
    decoder
        .push(b"\n\ndata: {\"second\":2}\n\ndata:[DO")
        .expect("middle chunk");
    decoder.push(b"NE]\n\n").expect("terminal chunk");

    let parsed = decoder.finish().expect("incremental stream");
    assert_eq!(
        parsed.values,
        vec![
            serde_json::json!({"first": 1}),
            serde_json::json!({"second": 2})
        ]
    );
    assert_eq!(parsed.dispatched_events, 3);
    assert!(parsed.terminal_sentinel);
}

#[test]
fn responses_reject_unterminated_event_data_at_end_of_stream() {
    assert!(parse_sse_stream("data:{\"type\":\"response.completed\"}").is_err());
    assert!(parse_sse_stream("data:{\"type\":\"response.completed\"}\n").is_err());
}
