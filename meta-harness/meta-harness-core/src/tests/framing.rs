use crate::{FrameError, LineFramer};

#[test]
fn frames_records_only_at_line_feeds() {
    // pi frames RPC records with LF only; U+2028 and U+2029 inside strings are content.
    let mut framer = LineFramer::new(1024);
    let chunks: [&[u8]; 3] = [
        b"{\"type\":\"agent_start\"}\n{\"type\":\"message_update\",\"delta\":\"a",
        "\u{2028}b\u{2029}c\"}".as_bytes(),
        b"\n",
    ];
    let records: Vec<Vec<u8>> = chunks
        .iter()
        .flat_map(|chunk| framer.push(chunk).expect("records stay within the limit"))
        .collect();
    assert_eq!(
        records,
        [
            b"{\"type\":\"agent_start\"}".to_vec(),
            "{\"type\":\"message_update\",\"delta\":\"a\u{2028}b\u{2029}c\"}"
                .as_bytes()
                .to_vec(),
        ]
    );
}

#[test]
fn rejects_records_longer_than_the_limit() {
    let mut framer = LineFramer::new(8);
    assert_eq!(
        framer.push(b"{\"a\":12}\n"),
        Ok(vec![b"{\"a\":12}".to_vec()])
    );
    let error = framer
        .push(b"{\"type\":")
        .and_then(|_| framer.push(b"1"))
        .expect_err("a partial record past the limit fails closed");
    assert_eq!(error, FrameError::RecordTooLong { limit: 8 });
    assert_eq!(error.to_string(), "record exceeds 8 bytes");
    assert_eq!(
        LineFramer::new(4).push(b"{\"a\":1}\n"),
        Err(FrameError::RecordTooLong { limit: 4 })
    );
}
