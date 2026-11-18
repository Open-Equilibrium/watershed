use crate::runtime::{
    auth::base64url_encode,
    oauth_credential::{
        CredentialRecord, account_routing_from_id_token, base64url_decode,
        validate_credential_record,
    },
};

pub(super) fn credential_record(
    access: &str,
    refresh: &str,
    expires: u64,
    account_id: &str,
) -> CredentialRecord {
    CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: access.to_owned(),
        refresh: refresh.to_owned(),
        expires,
        account_id: account_id.to_owned(),
        is_fedramp: false,
    }
}

pub(super) fn jwt_with_account(account_id: &str) -> String {
    jwt_with_account_routing(account_id, None)
}

pub(super) fn jwt_with_account_routing(account_id: &str, is_fedramp: Option<bool>) -> String {
    let mut auth = serde_json::json!({"chatgpt_account_id": account_id});
    if let Some(is_fedramp) = is_fedramp {
        auth["chatgpt_account_is_fedramp"] = is_fedramp.into();
    }
    jwt_with_auth_claim(auth)
}

fn jwt_with_auth_claim(auth: serde_json::Value) -> String {
    let payload = serde_json::json!({"https://api.openai.com/auth": auth});
    format!(
        "e30.{}.x",
        base64url_encode(
            serde_json::to_string(&payload)
                .expect("JWT payload")
                .as_bytes()
        )
    )
}

#[test]
fn credential_record_validation_rejects_each_invalid_field() {
    let valid = credential_record("access", "refresh", 1, "account");
    validate_credential_record(&valid).expect("valid credential record");
    let mutations: [fn(&mut CredentialRecord); 6] = [
        |record| record.credential_type = "api-key".to_owned(),
        |record| record.expires = 0,
        |record| record.refresh = String::new(),
        |record| record.account_id = "account\nx".to_owned(),
        |record| record.access = "access x".to_owned(),
        |record| record.access = "access\u{e9}".to_owned(),
    ];
    for mutate in mutations {
        let mut invalid = valid.clone();
        mutate(&mut invalid);
        assert!(validate_credential_record(&invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn credential_record_debug_never_prints_tokens_or_account() {
    let credential = credential_record(
        "access-secret-fixture",
        "refresh-secret-fixture",
        1,
        "account-secret-fixture",
    );
    let debug = format!("{credential:?}");
    for secret in [
        "access-secret-fixture",
        "refresh-secret-fixture",
        "account-secret-fixture",
    ] {
        assert!(!debug.contains(secret), "{debug}");
    }
}

#[test]
fn account_routing_debug_never_prints_the_account() {
    let routing = account_routing_from_id_token(&jwt_with_account("account-secret-fixture"))
        .expect("account routing parses");
    let debug = format!("{routing:?}");
    assert!(!debug.contains("account-secret-fixture"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}

#[test]
fn account_routing_defaults_to_standard_and_preserves_fedramp() {
    for (claim, expected) in [(None, false), (Some(false), false), (Some(true), true)] {
        let routing = account_routing_from_id_token(&jwt_with_account_routing("account", claim))
            .expect("account routing parses");
        assert_eq!(routing.account_id, "account");
        assert_eq!(routing.is_fedramp, expected);
    }

    let token = jwt_with_auth_claim(serde_json::json!({
        "chatgpt_account_id": "account",
        "chatgpt_account_is_fedramp": "true"
    }));
    assert!(account_routing_from_id_token(&token).is_err());
}

#[test]
fn base64url_decoder_accepts_canonical_chunk_shapes() {
    for (encoded, expected) in [
        ("AA", Vec::from([0_u8])),
        ("AP8", Vec::from([0_u8, 255])),
        ("AH__", Vec::from([0_u8, 127, 255])),
        ("__79_A", Vec::from([255_u8, 254, 253, 252])),
    ] {
        assert_eq!(
            base64url_decode(encoded).expect("canonical base64url"),
            expected
        );
    }
    for invalid in ["", "A", "A=", "/w", "AB"] {
        assert!(
            base64url_decode(invalid).is_err(),
            "must reject {invalid:?}"
        );
    }
    assert_eq!(
        base64url_decode("-A").expect("URL-safe dash decodes"),
        [248]
    );
}
