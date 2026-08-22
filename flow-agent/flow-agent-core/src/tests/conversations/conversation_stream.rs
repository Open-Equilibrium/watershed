mod reader;
mod writer;

fn padded_json_value(canonical_bytes: usize) -> serde_json::Value {
    let empty = proto::canonical_json(&serde_json::json!({"padding": ""})).unwrap();
    assert!(canonical_bytes >= empty.len());
    serde_json::json!({"padding": "x".repeat(canonical_bytes - empty.len())})
}
