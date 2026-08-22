use super::super::auth::authenticated_status_message;

#[test]
fn authenticated_status_labels_the_expiry_unit() {
    assert_eq!(
        authenticated_status_message(1_234),
        "openai-codex authenticated; credential expires at Unix epoch millisecond 1234\n"
    );
}
