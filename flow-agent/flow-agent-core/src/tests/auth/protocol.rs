use super::{jwt_with_account, token_body, token_body_with_id};
use crate::runtime::{
    auth::{
        DevicePoll, OAuthCallback, build_authorize_url, epoch_milliseconds, next_poll_interval,
        parse_device_authorization_body, parse_device_poll_body, parse_oauth_callback,
        parse_token_body, percent_encode, random_url_token, read_loopback_callback,
        read_loopback_callback_until, write_loopback_response,
    },
    oauth_credential::{MAX_OAUTH_FIELD_BYTES, MAX_OAUTH_SECRET_BYTES},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

#[test]
fn oauth_callback_accepts_encoded_fields_and_rejects_ambiguous_requests() {
    assert_eq!(
        parse_oauth_callback("ignored=x&state=s%2F1&code=a+b%2Fc", "s/1")
            .expect("encoded callback"),
        OAuthCallback::Code("a b/c".to_owned())
    );
    assert_eq!(
        parse_oauth_callback("state=s&error=access_denied", "s").expect("matching denial"),
        OAuthCallback::Denied
    );

    for query in [
        "state=s",
        "code=c",
        "code=c&state=wrong",
        "code=c&code=d&state=s",
        "code=c&state=s&state=s",
        "code=c&state",
        "code=%&state=s",
        "code=%GG&state=s",
        "code=%FF&state=s",
        "code=%00&state=s",
        "error=access_denied&state=wrong",
        "error=&state=s",
        "error=a&error=b&state=s",
        "code=c&error=access_denied&state=s",
    ] {
        assert!(
            parse_oauth_callback(query, "s").is_err(),
            "callback must be rejected: {query}"
        );
    }
}

#[test]
fn oauth_response_shapes_are_unambiguous() {
    assert!(matches!(
        parse_device_poll_body(br#"{"error":"authorization_pending"}"#),
        Ok(DevicePoll::Pending)
    ));
    assert!(matches!(
        parse_device_poll_body(br#"{"error":"slow_down"}"#),
        Ok(DevicePoll::SlowDown)
    ));
    for body in [
        br#"{}"#.as_slice(),
        br#"{"error":"denied"}"#,
        br#"{"authorization_code":"c"}"#,
        br#"{"code_verifier":"v"}"#,
        br#"{"error":"slow_down","authorization_code":"c","code_verifier":"v"}"#,
    ] {
        assert!(parse_device_poll_body(body).is_err());
    }

    for interval in [
        serde_json::Value::Null,
        serde_json::json!(0),
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!(""),
        serde_json::json!("1x"),
    ] {
        let body = serde_json::to_vec(&serde_json::json!({
            "device_auth_id": "d",
            "interval": interval,
            "user_code": "u",
            "verification_uri": "v"
        }))
        .expect("device JSON");
        assert!(parse_device_authorization_body(&body).is_err());
    }
    assert_eq!(next_poll_interval(7, false).expect("unchanged interval"), 7);
    assert!(next_poll_interval(0, false).is_err());
}

#[test]
fn oauth_credentials_derive_account_from_the_distinct_id_token() {
    let body = token_body_with_id(
        "opaque-access-token",
        &jwt_with_account("id-token-account"),
        "refresh",
        1.into(),
    );

    let credential = parse_token_body(&body, 10).expect("distinct token response");

    assert_eq!(credential.access, "opaque-access-token");
    assert_eq!(credential.account_id, "id-token-account");
    assert!(!credential.is_fedramp);

    for token in [
        "",
        "only-one-segment",
        "a..c",
        "a.b.",
        "a.b.c.d",
        "a.===.c",
        "a._.c",
    ] {
        assert!(parse_token_body(&token_body(token, "r", 1.into()), 0).is_err());
    }
}

#[test]
fn oauth_authorize_and_loopback_protocol_are_canonical() {
    let url = build_authorize_url("state value", "challenge/value");
    assert!(url.starts_with("https://auth.openai.com/oauth/authorize"));
    assert!(url.contains("state=state%20value"));
    assert!(url.contains("code_challenge=challenge%2Fvalue"));
    assert!(url.contains("originator=flow-agent"));
    assert_eq!(percent_encode("AZaz09-._~ /"), "AZaz09-._~%20%2F");
    assert!(epoch_milliseconds().expect("current epoch") > 0);
    let first = random_url_token(24).expect("random state");
    let second = random_url_token(24).expect("random state");
    assert_eq!(first.len(), 32);
    assert_ne!(first, second);

    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
    let address = listener.local_addr().expect("loopback address");
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("loopback connects");
        stream
            .write_all(b"GET /auth/callback?state=s&code=c HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("callback writes");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("response reads");
        response
    });
    let (mut stream, _) = listener.accept().expect("callback accepts");
    assert_eq!(
        read_loopback_callback(&mut stream, "s").expect("callback parses"),
        OAuthCallback::Code("c".to_owned())
    );
    write_loopback_response(&mut stream, "200 OK", "complete").expect("response writes");
    drop(stream);
    let response = client.join().expect("loopback client completes");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.ends_with("\r\n\r\ncomplete"));
    assert_eq!(
        parse_oauth_callback("state=s%2f1&code=c", "s/1")
            .expect("lowercase percent encoding parses"),
        OAuthCallback::Code("c".to_owned())
    );
}

#[test]
fn oauth_loopback_rejects_a_truncated_request_header() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
    let address = listener.local_addr().expect("loopback address");
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("loopback connects");
        stream
            .write_all(b"GET /auth/callback?state=s&code=c HTTP/1.1\r\n")
            .expect("partial callback writes");
    });
    let (mut stream, _) = listener.accept().expect("callback accepts");

    assert!(read_loopback_callback(&mut stream, "s").is_err());
    client.join().expect("partial callback client completes");
}

#[test]
fn oauth_loopback_waits_for_a_callback_on_an_inherited_nonblocking_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
    let address = listener.local_addr().expect("loopback address");
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("loopback connects");
        thread::sleep(Duration::from_millis(25));
        stream
            .write_all(b"GET /auth/callback?state=s&code=c HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("callback writes");
    });
    let (mut stream, _) = listener.accept().expect("callback accepts");
    stream
        .set_nonblocking(true)
        .expect("accepted stream becomes nonblocking");

    assert_eq!(
        read_loopback_callback_until(
            &mut stream,
            "s",
            Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .expect("callback waits for request bytes"),
        OAuthCallback::Code("c".to_owned())
    );
    client.join().expect("callback client completes");
}

#[test]
fn oauth_loopback_callback_is_bounded_by_its_absolute_and_connection_deadlines() {
    for (absolute, connection) in [
        (Duration::from_millis(25), Duration::from_secs(5)),
        (Duration::from_secs(5), Duration::from_millis(25)),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let address = listener.local_addr().expect("loopback address");
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("loopback connects");
            for byte in b"GET /auth/callback?code=c&state=s HTTP/1.1\r\nHost: localhost\r\n\r\n" {
                if stream.write_all(&[*byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let (mut stream, _) = listener.accept().expect("callback accepts");

        assert!(
            read_loopback_callback_until(&mut stream, "s", Instant::now() + absolute, connection,)
                .is_err(),
            "absolute {absolute:?}, connection {connection:?}"
        );
        client.join().expect("slow callback client completes");
    }
}

fn assert_limit(
    label: &str,
    limit: usize,
    body: impl Fn(usize) -> Vec<u8>,
    accepts: impl Fn(&[u8]) -> bool,
) {
    assert!(accepts(&body(limit)), "{label} at its limit must parse");
    assert!(
        !accepts(&body(limit + 1)),
        "{label} one byte over its limit must be rejected"
    );
}

fn device_authorization_body(field: &str, size: usize) -> Vec<u8> {
    let mut value = serde_json::json!({
        "device_auth_id": "d",
        "interval": 1,
        "user_code": "u",
        "verification_uri": "v",
    });
    value[field] = serde_json::Value::String("x".repeat(size));
    serde_json::to_vec(&value).expect("device JSON")
}

#[test]
fn oauth_verifier_field_budget() {
    assert_limit(
        "code_verifier",
        MAX_OAUTH_FIELD_BYTES,
        |size| {
            serde_json::to_vec(&serde_json::json!({
                "authorization_code": "c",
                "code_verifier": "v".repeat(size),
            }))
            .expect("device poll JSON")
        },
        |body| parse_device_poll_body(body).is_ok(),
    );
}

#[test]
fn oauth_device_authorization_field_budgets() {
    for field in ["device_auth_id", "user_code", "verification_uri"] {
        assert_limit(
            field,
            MAX_OAUTH_FIELD_BYTES,
            |size| device_authorization_body(field, size),
            |body| parse_device_authorization_body(body).is_ok(),
        );
    }
}

#[test]
fn oauth_account_id_field_budget() {
    assert_limit(
        "account id",
        MAX_OAUTH_FIELD_BYTES,
        |size| token_body(&jwt_with_account(&"a".repeat(size)), "r", 1.into()),
        |body| parse_token_body(body, 0).is_ok(),
    );
}

#[test]
fn oauth_access_and_id_jwt_token_field_budget() {
    let account_jwt = jwt_with_account("a");
    let tail = &account_jwt[account_jwt.find('.').expect("dot")..];
    assert_limit(
        "access token",
        MAX_OAUTH_SECRET_BYTES,
        |size| token_body_with_id(&"a".repeat(size), &account_jwt, "r", 1.into()),
        |body| parse_token_body(body, 0).is_ok(),
    );
    assert_limit(
        "ID token",
        MAX_OAUTH_SECRET_BYTES,
        |size| {
            let id = format!("{}{tail}", "h".repeat(size - tail.len()));
            token_body_with_id("access", &id, "r", 1.into())
        },
        |body| parse_token_body(body, 0).is_ok(),
    );
}

#[test]
fn oauth_refresh_token_field_budget() {
    let id_token = jwt_with_account("a");
    assert_limit(
        "refresh token",
        MAX_OAUTH_SECRET_BYTES,
        |size| token_body_with_id("access", &id_token, &"r".repeat(size), 1.into()),
        |body| parse_token_body(body, 0).is_ok(),
    );
}

#[test]
fn oauth_poll_interval_budget() {
    for interval in [serde_json::json!(60), serde_json::json!(" 60 ")] {
        let body = serde_json::to_vec(&serde_json::json!({
            "device_auth_id":"d", "interval":interval, "user_code":"u", "verification_uri":"v"
        }))
        .expect("device JSON");
        assert_eq!(
            parse_device_authorization_body(&body)
                .expect("interval")
                .interval_seconds,
            60
        );
    }
    let body = br#"{"device_auth_id":"d","interval":61,"user_code":"u","verification_uri":"v"}"#;
    assert!(parse_device_authorization_body(body).is_err());
    assert_eq!(next_poll_interval(55, true).expect("slow down"), 60);
    assert!(next_poll_interval(60, true).is_err());
}

#[test]
fn oauth_expiry_budget() {
    let access = jwt_with_account("a");
    assert_eq!(
        parse_token_body(&token_body(&access, "r", 86_400.into()), 1)
            .expect("bounded expiry")
            .expires,
        86_400_001
    );
    assert!(parse_token_body(&token_body(&access, "r", 86_401.into()), 0).is_err());
    assert!(parse_token_body(&token_body(&access, "r", 1.into()), u64::MAX).is_err());
}
