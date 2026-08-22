use crate::runtime::{
    fs_guards::{
        decode_utf8, for_each_reader_line_with_limit, read_opened_file_with_limit,
        retry_event_segment_discovery,
    },
    types::RuntimeError,
};
use std::{
    io::{self, Read},
    path::Path,
};

struct ReadRequestObserver {
    max_request_bytes: usize,
    remaining: usize,
}

impl Read for ReadRequestObserver {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.max_request_bytes = self.max_request_bytes.max(buffer.len());
        let read = buffer.len().min(self.remaining);
        buffer[..read].fill(b'x');
        self.remaining -= read;
        Ok(read)
    }
}

#[test]
fn bounded_line_reader_consumes_at_most_one_byte_beyond_limit() {
    let mut source = io::Cursor::new(vec![b'x'; 64]);
    let mut visits = 0usize;

    let err = for_each_reader_line_with_limit(&mut source, Path::new("growing.jsonl"), 8, |_| {
        visits += 1;
        Ok(())
    })
    .expect_err("newline-free growth beyond the byte limit is rejected");

    assert!(err.to_string().contains("9 bytes exceeds max 8"), "{err}");
    assert_eq!(source.position(), 9);
    assert_eq!(visits, 0);
}

#[test]
fn bounded_readers_preserve_lines_and_reject_invalid_input() {
    let source = b"first\nsecond";
    let mut observed = Vec::new();
    assert_eq!(
        for_each_reader_line_with_limit(
            io::Cursor::new(source),
            Path::new("records.jsonl"),
            source.len() as u64,
            |line| {
                observed.push(line.to_owned());
                Ok(())
            },
        )
        .expect("bounded lines"),
        source.len() as u64
    );
    assert_eq!(observed, ["first\n", "second"]);

    let visitor_error = for_each_reader_line_with_limit(
        io::Cursor::new(b"line\n"),
        Path::new("records.jsonl"),
        5,
        |_| Err(RuntimeError::Protocol("visitor stopped".to_owned())),
    )
    .expect_err("visitor errors propagate");
    assert!(visitor_error.to_string().contains("visitor stopped"));

    let utf8_error = for_each_reader_line_with_limit(
        io::Cursor::new([0xff, b'\n']),
        Path::new("records.jsonl"),
        2,
        |_| Ok(()),
    )
    .expect_err("invalid UTF-8 rejects");
    assert!(utf8_error.to_string().contains("not valid UTF-8"));
    assert!(
        decode_utf8(Path::new("config.yaml"), vec![0xff])
            .expect_err("invalid config UTF-8 rejects")
            .to_string()
            .contains("not valid UTF-8")
    );

    assert_eq!(
        read_opened_file_with_limit(io::Cursor::new(b"data"), 4, Path::new("config.yaml"), 4,)
            .expect("exact bounded file"),
        b"data"
    );
    assert!(
        read_opened_file_with_limit(io::Cursor::new(b"data"), 5, Path::new("config.yaml"), 4,)
            .is_err()
    );
    assert!(
        read_opened_file_with_limit(io::Cursor::new(b"growing"), 4, Path::new("config.yaml"), 4,)
            .is_err()
    );
}

#[test]
fn bounded_file_reader_limits_each_read_request() {
    const FILE_BYTES: usize = 16 * 1024 * 1024;
    const REQUEST_BYTES: usize = 1024 * 1024;
    let mut source = ReadRequestObserver {
        max_request_bytes: 0,
        remaining: FILE_BYTES,
    };

    let bytes = read_opened_file_with_limit(
        &mut source,
        FILE_BYTES as u64,
        Path::new("large-canonical-input"),
        FILE_BYTES as u64,
    )
    .expect("legal large input is materialized exactly");

    assert_eq!(bytes.len(), FILE_BYTES);
    assert!(
        source.max_request_bytes <= REQUEST_BYTES,
        "observed {}-byte read request",
        source.max_request_bytes
    );
}

#[test]
fn event_segment_discovery_retries_one_transient_protocol_error() {
    let mut attempts = 0;
    let stream = retry_event_segment_discovery(|| {
        attempts += 1;
        if attempts == 1 {
            Err(RuntimeError::Protocol("concurrent rotation".to_owned()))
        } else {
            Ok("complete")
        }
    })
    .expect("transient discovery recovers");

    assert_eq!((stream, attempts), ("complete", 2));
}
