use crate::runtime::{
    auth::{
        device_login_remaining, run_device_login_with_components,
        run_device_login_with_deadlines_and_clock,
    },
    deadlines::DEVICE_POLL_OVERALL_DEADLINE,
    oauth_credential::CredentialRecord,
    types::RuntimeError,
};
use std::{cell::Cell, collections::VecDeque, time::Duration};

#[test]
fn device_login_honors_pending_and_slow_down_before_exchange() {
    let mut presented = String::new();
    let mut sleeps = Vec::new();
    let mut polls = VecDeque::from([
        br#"{"error":"authorization_pending"}"#.to_vec(),
        br#"{"error":"slow_down"}"#.to_vec(),
        br#"{"authorization_code":"fixture-code","code_verifier":"fixture-verifier"}"#.to_vec(),
    ]);

    let credential = run_device_login_with_components(
        &mut |message| {
            presented = message.to_owned();
            Ok(())
        },
        || {
            Ok(br#"{"device_auth_id":"device","interval":1,"user_code":"USER-CODE","verification_uri":"https://auth.openai.com/codex/device"}"#.to_vec())
        },
        |_authorization| {
            polls
                .pop_front()
                .ok_or_else(|| RuntimeError::Protocol("poll fixture exhausted".to_owned()))
        },
        |code, verifier, redirect| {
            assert_eq!(code, "fixture-code");
            assert_eq!(verifier, "fixture-verifier");
            assert_eq!(redirect, "https://auth.openai.com/deviceauth/callback");
            Ok(CredentialRecord {
                credential_type: "oauth".to_owned(),
                access: "access".to_owned(),
                refresh: "refresh".to_owned(),
                expires: 84,
                account_id: "account".to_owned(),
                is_fedramp: false,
            })
        },
        |duration| sleeps.push(duration),
    )
    .expect("device login completes");

    assert_eq!(
        presented,
        "Open https://auth.openai.com/codex/device and enter code USER-CODE"
    );
    assert_eq!(
        sleeps,
        [
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(6),
        ]
    );
    assert!(polls.is_empty());
    assert_eq!(credential.expires, 84);

    assert!(run_device_login_with_components(
        &mut |_| Ok(()),
        || {
            Ok(br#"{"device_auth_id":"device","interval":1,"user_code":"USER-CODE","verification_uri":"https://example.invalid"}"#.to_vec())
        },
        |_| unreachable!("invalid verification URI must stop before polling"),
        |_, _, _| unreachable!("invalid verification URI must stop before exchange"),
        |_| {},
    )
    .is_err());
}

#[test]
fn device_poll_overall_deadline() {
    assert_eq!(DEVICE_POLL_OVERALL_DEADLINE, Duration::from_secs(15 * 60));
    assert_eq!(
        device_login_remaining(DEVICE_POLL_OVERALL_DEADLINE - Duration::from_nanos(1))
            .expect("the final nanosecond remains eligible"),
        Duration::from_nanos(1)
    );
    assert!(device_login_remaining(DEVICE_POLL_OVERALL_DEADLINE).is_err());
    let elapsed = Cell::new(Duration::ZERO);
    let polls = Cell::new(0_u8);
    let exchange_called = Cell::new(false);

    let error = run_device_login_with_deadlines_and_clock(
        &mut |_| Ok(()),
        || {
            Ok(br#"{"device_auth_id":"device","interval":1,"user_code":"USER-CODE","verification_uri":"https://auth.openai.com/codex/device"}"#.to_vec())
        },
        |_, deadlines| {
            let poll = polls.get() + 1;
            polls.set(poll);
            match poll {
                1 => {
                    assert_eq!(deadlines.overall, Duration::from_secs(60));
                    elapsed.set(DEVICE_POLL_OVERALL_DEADLINE - Duration::from_secs(5));
                    Ok(br#"{"error":"authorization_pending"}"#.to_vec())
                }
                2 => {
                    assert_eq!(deadlines.overall, Duration::from_secs(4));
                    Ok(br#"{"error":"slow_down"}"#.to_vec())
                }
                _ => unreachable!("the absolute deadline prevents a third poll"),
            }
        },
        |_, _, _, _| {
            exchange_called.set(true);
            unreachable!("an expired device login must not exchange the code")
        },
        |duration| elapsed.set(elapsed.get() + duration),
        || elapsed.get(),
    )
    .expect_err("a poll that reaches the overall deadline is rejected");

    assert!(
        error
            .to_string()
            .contains("authentication protocol failure")
    );
    assert_eq!(polls.get(), 2);
    assert!(!exchange_called.get());
}
