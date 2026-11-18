use rusqlite::Connection;

use crate::hosts::{self, Access, ClientKind, printable};

const MINUTE: u64 = 60_000;

fn store() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(hosts::SCHEMA).unwrap();
    db
}

fn kind_of(db: &Connection, token: &str) -> Option<ClientKind> {
    hosts::client(db, token).unwrap().map(|(kind, _, _)| kind)
}

#[test]
fn a_pairing_code_pairs_once_within_five_minutes_however_it_is_typed() {
    let db = store();
    let code = hosts::invite(&db, ClientKind::Device, Access::Auto, 0).unwrap();
    let typed = code.replace('-', "").to_lowercase();
    assert!(hosts::pair(&db, &typed, "phone", MINUTE).is_ok());
    assert!(hosts::pair(&db, &code, "phone", MINUTE).is_err(), "used");

    let expired = hosts::invite(&db, ClientKind::Device, Access::Auto, 10 * MINUTE).unwrap();
    assert!(
        hosts::pair(&db, &expired, "tablet", 15 * MINUTE).is_err(),
        "expired"
    );
}

#[test]
fn at_most_five_pairing_codes_are_live_at_once() {
    let db = store();
    for _ in 0..5 {
        hosts::invite(&db, ClientKind::Host, Access::Auto, 0).unwrap();
    }
    assert!(hosts::invite(&db, ClientKind::Host, Access::Auto, 0).is_err());
    assert!(
        hosts::invite(&db, ClientKind::Host, Access::Auto, 5 * MINUTE).is_ok(),
        "expired codes do not count"
    );
}

#[test]
fn a_token_admits_the_kind_of_client_that_paired_with_it_until_revoked() {
    let db = store();
    let device = hosts::invite(&db, ClientKind::Device, Access::Auto, 0).unwrap();
    let host = hosts::invite(&db, ClientKind::Host, Access::Auto, 0).unwrap();
    let device_token = hosts::pair(&db, &device, "phone", 0).unwrap().0;
    let host_token = hosts::pair(&db, &host, "other", 0).unwrap().0;
    assert_eq!(device_token.len(), 64, "256 bits in hex");
    assert_eq!(kind_of(&db, &device_token), Some(ClientKind::Device));
    assert_eq!(kind_of(&db, &host_token), Some(ClientKind::Host));
    assert_eq!(kind_of(&db, &device_token[..63]), None);

    assert!(hosts::revoke(&db, "phone").unwrap(), "a client was revoked");
    assert_eq!(kind_of(&db, &device_token), None);
    assert!(
        !hosts::revoke(&db, "phone").unwrap(),
        "nothing left to revoke"
    );
}

#[test]
fn a_name_is_trimmed_and_limited_as_t3_limits_it() {
    assert_eq!(printable("\u{3000} phone \u{feff}"), "phone");
    assert_eq!(
        printable("   \n\t"),
        "unnamed",
        "an empty name is the default one"
    );
    assert_eq!(printable(&"a".repeat(70)).chars().count(), 64);
    let emoji = printable(&"\u{1F600}".repeat(60));
    assert!(emoji.encode_utf16().count() <= 100, "{emoji}");
    assert!(!emoji.ends_with(char::is_whitespace));
}
