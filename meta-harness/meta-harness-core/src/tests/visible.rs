use crate::visible_lines;

#[test]
fn keeps_windows_line_ends_and_writes_out_other_carriage_returns() {
    assert_eq!(
        visible_lines("one\r\ntwo\rthree\r\nfour\u{202E}\n"),
        "one\r\ntwo<U+000D>three\r\nfour<U+202E>\n"
    );
}
