use super::super::auth::api_key_from_input;

#[test]
fn api_key_input_drops_one_final_line_break_only() {
    for (input, expected) in [
        ("sk-fixture\n", "sk-fixture"),
        ("sk-fixture\r\n", "sk-fixture"),
        ("sk-fixture", "sk-fixture"),
        ("sk-fixture\n\n", "sk-fixture\n"),
    ] {
        assert_eq!(api_key_from_input(input.to_owned()), expected, "{input:?}");
    }
}
