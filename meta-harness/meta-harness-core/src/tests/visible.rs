use crate::visible::{visible, visible_lines};

#[test]
fn keeps_windows_line_ends_and_writes_out_other_carriage_returns() {
    assert_eq!(
        visible_lines("one\r\ntwo\rthree\r\nfour\u{202E}\n"),
        "one\r\ntwo<U+000D>three\r\nfour<U+202E>\n"
    );
}

#[test]
fn never_shows_two_texts_alike() {
    // A literal marker would read like a hidden character: `<U+200B>` in a shell command is two
    // redirections, around a file named `U+200B`.
    let literal = visible("printf x <U+200B>keys");
    assert_eq!(literal, "printf x <U+003C>U+200B>keys");
    assert_ne!(literal, visible("printf x \u{200B}keys"));
    assert_ne!(visible("<U+003C>"), visible("<"));
}
