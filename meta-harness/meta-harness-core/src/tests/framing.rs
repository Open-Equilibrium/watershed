use crate::{Frame, LineFramer};

#[test]
fn frames_records_only_at_line_feeds() {
    // pi frames RPC records with LF only; U+2028 and U+2029 inside strings are content.
    let mut framer = LineFramer::new(1024);
    let chunks: [&[u8]; 3] = [
        b"{\"type\":\"agent_start\"}\n{\"type\":\"message_update\",\"delta\":\"a",
        "\u{2028}b\u{2029}c\"}".as_bytes(),
        b"\n",
    ];
    let records: Vec<Frame> = chunks.iter().flat_map(|chunk| framer.push(chunk)).collect();
    assert_eq!(
        records,
        [
            Frame::Record(b"{\"type\":\"agent_start\"}".to_vec()),
            Frame::Record(
                "{\"type\":\"message_update\",\"delta\":\"a\u{2028}b\u{2029}c\"}"
                    .as_bytes()
                    .to_vec()
            ),
        ]
    );
}

#[test]
fn skips_records_longer_than_the_limit_and_keeps_their_start() {
    let mut framer = LineFramer::new(8);
    assert_eq!(
        framer.push(b"{\"a\":12}\n"),
        [Frame::Record(b"{\"a\":12}".to_vec())]
    );
    // A record past the limit is dropped up to its line feed; only its start is kept, and the
    // records after it still arrive.
    assert_eq!(framer.push(b"{\"type\":"), []);
    assert_eq!(framer.push(b"1234567890"), []);
    assert_eq!(
        framer.push(b"}\n{\"b\":1}\n"),
        [
            Frame::Oversized {
                start: b"{\"type\":1234567890".to_vec()
            },
            Frame::Record(b"{\"b\":1}".to_vec()),
        ]
    );
    assert_eq!(
        LineFramer::new(4).push(b"{\"a\":1}\n"),
        [Frame::Oversized {
            start: b"{\"a\":1}".to_vec()
        }]
    );
    let mut huge = LineFramer::new(4);
    let frames = huge.push(&[b'x'; 1000]);
    assert!(frames.is_empty());
    assert_eq!(
        huge.push(b"\n"),
        [Frame::Oversized {
            start: vec![b'x'; 256]
        }],
        "the kept start is bounded"
    );
}
