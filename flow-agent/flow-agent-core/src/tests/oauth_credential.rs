use crate::runtime::{
    auth::base64url_encode,
    oauth_credential::{
        CredentialRecord, account_routing_from_id_token, base64url_decode,
        validate_credential_record,
    },
};

pub(super) fn jwt_with_account(account_id: &str) -> String {
    jwt_with_account_routing(account_id, None)
}

pub(super) fn jwt_with_account_routing(account_id: &str, is_fedramp: Option<bool>) -> String {
    let mut auth = serde_json::json!({"chatgpt_account_id": account_id});
    if let Some(is_fedramp) = is_fedramp {
        auth["chatgpt_account_is_fedramp"] = is_fedramp.into();
    }
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
fn credential_record_validation_is_bounded() {
    let credential = CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: "access".to_owned(),
        refresh: "refresh".to_owned(),
        expires: 1,
        account_id: "account".to_owned(),
        is_fedramp: false,
    };
    validate_credential_record(&credential).expect("valid credential record");

    for invalid in [
        CredentialRecord {
            credential_type: "api-key".to_owned(),
            ..credential.clone()
        },
        CredentialRecord {
            expires: 0,
            ..credential.clone()
        },
        CredentialRecord {
            refresh: String::new(),
            ..credential.clone()
        },
    ] {
        assert!(validate_credential_record(&invalid).is_err());
    }
}

#[test]
fn account_routing_defaults_to_standard_and_preserves_fedramp() {
    for (claim, expected) in [(None, false), (Some(false), false), (Some(true), true)] {
        let routing = account_routing_from_id_token(&jwt_with_account_routing("account", claim))
            .expect("account routing parses");
        assert_eq!(routing.account_id, "account");
        assert_eq!(routing.is_fedramp, expected);
    }

    let payload = serde_json::json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "account",
            "chatgpt_account_is_fedramp": "true"
        }
    });
    let token = format!(
        "e30.{}.x",
        base64url_encode(
            serde_json::to_string(&payload)
                .expect("JWT payload")
                .as_bytes()
        )
    );
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
