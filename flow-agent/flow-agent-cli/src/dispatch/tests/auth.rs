use super::super::auth::{api_key_from_input, authenticated_status_message};

#[test]
fn authenticated_status_labels_the_expiry_unit() {
    assert_eq!(
        authenticated_status_message(1_234),
        "openai-codex authenticated; credential expires at Unix epoch millisecond 1234\n"
    );
}

#[test]
fn api_key_input_drops_one_final_line_break_only() {
    assert_eq!(api_key_from_input("sk-fixture\n".to_owned()), "sk-fixture");
    assert_eq!(
        api_key_from_input("sk-fixture\r\n".to_owned()),
        "sk-fixture"
    );
    assert_eq!(api_key_from_input("sk-fixture".to_owned()), "sk-fixture");
    assert_eq!(
        api_key_from_input("sk-fixture\n\n".to_owned()),
        "sk-fixture\n"
    );
}
