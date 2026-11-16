//! Pairing codes and the tokens they issue (ADR-0299): a code is single use and expires, at most
//! five are live, and a token admits only the kind of client that paired with it.

use rusqlite::Connection;

use crate::hosts::{self, Kind, same};

const MINUTE: u64 = 60_000;

fn store() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(hosts::SCHEMA).unwrap();
    db
}

#[test]
fn a_pairing_code_pairs_once_within_five_minutes_however_it_is_typed() {
    let db = store();
    let code = hosts::invite(&db, Kind::Device, 0).unwrap();
    let typed = code.replace('-', "").to_lowercase();
    assert!(hosts::pair(&db, &typed, "phone", MINUTE).is_ok());
    assert!(hosts::pair(&db, &code, "phone", MINUTE).is_err(), "used");

    let expired = hosts::invite(&db, Kind::Device, 10 * MINUTE).unwrap();
    assert!(
        hosts::pair(&db, &expired, "tablet", 15 * MINUTE).is_err(),
        "expired"
    );
}

#[test]
fn at_most_five_pairing_codes_are_live_at_once() {
    let db = store();
    for _ in 0..5 {
        hosts::invite(&db, Kind::Host, 0).unwrap();
    }
    assert!(hosts::invite(&db, Kind::Host, 0).is_err());
    assert!(
        hosts::invite(&db, Kind::Host, 5 * MINUTE).is_ok(),
        "expired codes do not count"
    );
}

#[test]
fn a_token_admits_the_kind_of_client_that_paired_with_it_until_revoked() {
    let db = store();
    let device = hosts::invite(&db, Kind::Device, 0).unwrap();
    let host = hosts::invite(&db, Kind::Host, 0).unwrap();
    let device_token = hosts::pair(&db, &device, "phone", 0).unwrap();
    let host_token = hosts::pair(&db, &host, "other", 0).unwrap();
    assert_eq!(device_token.len(), 64, "256 bits in hex");
    assert_eq!(
        hosts::client(&db, &device_token)
            .unwrap()
            .map(|(kind, _)| kind),
        Some(Kind::Device)
    );
    assert_eq!(
        hosts::client(&db, &host_token)
            .unwrap()
            .map(|(kind, _)| kind),
        Some(Kind::Host)
    );
    assert_eq!(
        hosts::client(&db, &device_token[..63])
            .unwrap()
            .map(|(kind, _)| kind),
        None
    );

    hosts::revoke(&db, "phone").unwrap();
    assert_eq!(
        hosts::client(&db, &device_token)
            .unwrap()
            .map(|(kind, _)| kind),
        None
    );
    assert!(
        hosts::revoke(&db, "phone").is_err(),
        "nothing left to revoke"
    );
}

#[test]
fn comparison_accepts_only_equal_values() {
    assert!(same(b"abc", b"abc"));
    assert!(!same(b"abc", b"abd"));
    assert!(!same(b"abc", b"ab"));
}
