use crate::visible::{visible, visible_lines, visible_name};

#[test]
fn keeps_windows_line_ends_and_writes_out_other_carriage_returns() {
    assert_eq!(
        visible_lines("one\r\ntwo\rthree\r\nfour\u{202E}\n"),
        "one\r\ntwo<U+000D>three\r\nfour<U+202E>\n"
    );
    assert_eq!(visible_lines("abc\r"), "abc<U+000D>");
    assert_eq!(visible_lines("abc\r\n"), "abc\r\n");
    assert_eq!(visible_lines("abc\r\r\n"), "abc<U+000D>\r\n");
}

#[test]
fn never_shows_two_texts_alike() {
    let literal = visible("printf x <U+200B>keys");
    assert_eq!(literal, "printf x <U+003C>U+200B>keys");
    assert_ne!(literal, visible("printf x \u{200B}keys"));
    assert_ne!(visible("<U+003C>"), visible("<"));
}

#[test]
fn writes_out_every_kind_of_character_that_hides_or_reorders_text() {
    let characters = concat!(
        "\u{AD}\u{34F}\u{600}\u{61C}\u{6DD}\u{70F}\u{890}\u{8E2}\u{115F}\u{17B4}\u{180B}",
        "\u{200B}\u{2028}\u{2060}\u{3164}\u{FE00}\u{FEFF}\u{FFA0}\u{FFF0}\u{110BD}\u{110CD}",
        "\u{13430}\u{1BCA0}\u{1D173}\u{E0000}",
    );
    for character in characters.chars() {
        assert_eq!(
            visible(&character.to_string()),
            format!("<U+{:04X}>", u32::from(character))
        );
    }
}

#[test]
fn a_name_stays_on_one_line_with_its_breaks_and_tabs_written_out() {
    assert_eq!(
        visible_name("a\tb\nc\u{200B}"),
        "a<U+0009>b<U+000A>c<U+200B>"
    );
}
