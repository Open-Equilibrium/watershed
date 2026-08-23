use super::super::chat::{CHAT_REFERENCE_REQUIRED, read_chat_reference};
use flow_agent_core::RuntimeError;
use std::io::Cursor;

#[test]
fn chat_reader_rejects_increasing_overlong_references_before_consuming_the_tail() {
    let utf8_byte_bound = (core_script::MAX_BLOCK_NAME_CHARS + 1) * 4;
    for input_chars in [core_script::MAX_BLOCK_NAME_CHARS + 1, utf8_byte_bound * 8] {
        let mut input = vec![b'x'; input_chars];
        input.push(b'\n');
        let input_len = input.len() as u64;
        let mut reader = Cursor::new(input);
        let result = read_chat_reference(&mut reader);

        assert!(
            reader.position() < input_len,
            "{input_chars} overlong input characters must fail before consuming the tail"
        );
        assert!(
            matches!(result, Err(RuntimeError::Usage(message)) if message == CHAT_REFERENCE_REQUIRED),
            "{input_chars} overlong input characters must be a usage error"
        );
    }
}

#[test]
fn chat_reader_accepts_increasing_surrounding_whitespace() {
    let utf8_byte_bound = (core_script::MAX_BLOCK_NAME_CHARS + 1) * 4;
    for whitespace_chars in [utf8_byte_bound * 2, utf8_byte_bound * 16] {
        let whitespace = " ".repeat(whitespace_chars);
        let input = format!("{whitespace}\n{whitespace}/hello{whitespace}\n");
        let reference =
            read_chat_reference(&mut Cursor::new(input)).expect("surrounded reference is valid");

        assert_eq!(reference, "hello");
    }
}

#[test]
fn chat_reader_preserves_utf8_crlf_and_eof_surfaces() {
    let decomposed_reference = "Cafe\u{301}";
    let reference = read_chat_reference(&mut Cursor::new(format!(
        "\u{2003}\r\n\u{2003}/{decomposed_reference}\u{2003}\r\n"
    )))
    .expect("Unicode whitespace and a decomposed name reference are accepted");
    assert_eq!(reference, decomposed_reference);

    for input in [b"\xff\n".as_slice(), b"\xe2\x82".as_slice()] {
        let error = read_chat_reference(&mut Cursor::new(input))
            .expect_err("invalid or incomplete UTF-8 is rejected");
        assert!(matches!(
            error,
            RuntimeError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::InvalidData
        ));
    }
}
