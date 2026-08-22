use super::jwt_with_account;
use crate::{
    runtime::{
        auth::{
            DevicePoll, MAX_OAUTH_DEVICE_POLL_BODY_BYTES, MAX_OAUTH_TOKEN_BODY_BYTES,
            MAX_OAUTH_USER_CODE_BODY_BYTES, base64url_encode,
            exchange_authorization_code_body_with_transport, parse_device_authorization_body,
            parse_device_poll_body, parse_token_body,
            poll_device_authorization_body_with_transport, post_json_with_deadlines,
            read_loopback_callback, refresh_credential_with_transport,
            request_device_authorization_body_with_transport, send_auth_request_async,
        },
        deadlines::{AUTH_HTTP_DEADLINES, HttpDeadlines, build_http_client},
        oauth_credential::CredentialRecord,
        types::RuntimeError,
    },
    tests::deadlines::{
        assert_pending, block_on_paused_network, expect_ready, send_scripted_http_bytes,
        settle_pending, spawn_scripted_http_server,
    },
};
use std::{
    cell::Cell,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

fn padded_json(mut prefix: String, target: usize) -> Vec<u8> {
    let suffix = "\"}";
    prefix.push_str(",\"padding\":\"");
    assert!(prefix.len() + suffix.len() <= target);
    prefix.push_str(&"p".repeat(target - prefix.len() - suffix.len()));
    prefix.push_str(suffix);
    assert_eq!(prefix.len(), target);
    prefix.into_bytes()
}

#[test]
fn oauth_callback_http_head_budget() {
    const MAXIMUM: usize = 16_384;
    let prefix = b"GET /auth/callback?state=s&code=c HTTP/1.1\r\nX-Pad: ";
    let suffix = b"\r\n\r\n";

    for (head_bytes, accepted) in [(MAXIMUM, true), (MAXIMUM + 1, false)] {
        let mut request = Vec::from(prefix);
        request.resize(head_bytes - suffix.len(), b'x');
        request.extend_from_slice(suffix);
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
        let address = listener.local_addr().expect("loopback address");
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("loopback connects");
            stream.write_all(&request).expect("callback head writes");
        });
        let (mut stream, _) = listener.accept().expect("callback accepts");
        let result = read_loopback_callback(&mut stream, "s");
        client.join().expect("callback client completes");

        assert_eq!(result.is_ok(), accepted, "{head_bytes}");
    }
}

#[test]
fn oauth_user_code_body_budget() {
    let prefix = r#"{"device_auth_id":"d","interval":1,"user_code":"u","verification_uri":"https://auth.openai.com/codex""#.to_owned();
    let exact = padded_json(prefix, MAX_OAUTH_USER_CODE_BODY_BYTES);
    let mut oversized = exact.clone();
    oversized.insert(oversized.len() - 2, b'p');
    for (body, accepted) in [(exact, true), (oversized, false)] {
        let delivered = Cell::new(false);
        let result =
            request_device_authorization_body_with_transport(|endpoint, request, maximum| {
                assert_eq!(
                    endpoint,
                    "https://auth.openai.com/api/accounts/deviceauth/usercode"
                );
                assert_eq!(
                    request,
                    &serde_json::json!({"client_id": "app_EMoamEEZ73f0CkXaXp7hrann"})
                );
                assert_eq!(maximum, MAX_OAUTH_USER_CODE_BODY_BYTES);
                if body.len() > maximum {
                    return Err(RuntimeError::Protocol(
                        "fixture response exceeds selected maximum".to_owned(),
                    ));
                }
                delivered.set(true);
                Ok(body)
            })
            .and_then(|body| parse_device_authorization_body(&body));
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(delivered.get(), accepted);
    }
}

#[test]
fn oauth_device_poll_body_budget() {
    let prefix = r#"{"authorization_code":"c","code_verifier":"v""#.to_owned();
    let exact = padded_json(prefix, MAX_OAUTH_DEVICE_POLL_BODY_BYTES);
    let mut oversized = exact.clone();
    oversized.insert(oversized.len() - 2, b'p');
    let authorization = crate::runtime::auth::DeviceAuthorization {
        device_auth_id: "device".to_owned(),
        interval_seconds: 1,
        user_code: "USER-CODE".to_owned(),
        verification_uri: "https://auth.openai.com/codex/device".to_owned(),
    };
    for (body, accepted) in [(exact, true), (oversized, false)] {
        let delivered = Cell::new(false);
        let result = poll_device_authorization_body_with_transport(
            &authorization,
            AUTH_HTTP_DEADLINES,
            |endpoint, request, maximum, deadlines| {
                assert_eq!(
                    endpoint,
                    "https://auth.openai.com/api/accounts/deviceauth/token"
                );
                assert_eq!(
                    request,
                    &serde_json::json!({
                        "device_auth_id": "device",
                        "user_code": "USER-CODE",
                    })
                );
                assert_eq!(maximum, MAX_OAUTH_DEVICE_POLL_BODY_BYTES);
                assert_eq!(deadlines, AUTH_HTTP_DEADLINES);
                if body.len() > maximum {
                    return Err(RuntimeError::Protocol(
                        "fixture response exceeds selected maximum".to_owned(),
                    ));
                }
                delivered.set(true);
                Ok(body)
            },
        )
        .and_then(|body| parse_device_poll_body(&body));
        assert_eq!(
            matches!(result, Ok(DevicePoll::Authorization { .. })),
            accepted
        );
        assert_eq!(delivered.get(), accepted);
    }
}

#[test]
fn oauth_token_body_budget() {
    let access = jwt_with_account("a");
    let prefix = format!(
        "{{\"access_token\":{access:?},\"id_token\":{access:?},\"expires_in\":1,\"refresh_token\":\"r\""
    );
    let exact = padded_json(prefix, MAX_OAUTH_TOKEN_BODY_BYTES);
    let mut oversized = exact.clone();
    oversized.insert(oversized.len() - 2, b'p');
    for (body, accepted) in [(exact, true), (oversized, false)] {
        let delivered = Cell::new(false);
        let result = exchange_authorization_code_body_with_transport(
            "code",
            "verifier",
            "https://auth.openai.com/deviceauth/callback",
            AUTH_HTTP_DEADLINES,
            |endpoint, fields, maximum, deadlines| {
                assert_eq!(endpoint, "https://auth.openai.com/oauth/token");
                assert_eq!(
                    fields,
                    [
                        ("grant_type", "authorization_code"),
                        ("client_id", "app_EMoamEEZ73f0CkXaXp7hrann"),
                        ("code", "code"),
                        ("code_verifier", "verifier"),
                        (
                            "redirect_uri",
                            "https://auth.openai.com/deviceauth/callback"
                        ),
                    ]
                );
                assert_eq!(maximum, MAX_OAUTH_TOKEN_BODY_BYTES);
                assert_eq!(deadlines, AUTH_HTTP_DEADLINES);
                if body.len() > maximum {
                    return Err(RuntimeError::Protocol(
                        "fixture response exceeds selected maximum".to_owned(),
                    ));
                }
                delivered.set(true);
                Ok(body)
            },
        )
        .and_then(|body| parse_token_body(&body, 0));
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(delivered.get(), accepted);
    }
}

#[test]
fn oauth_refresh_body_budget() {
    let access = jwt_with_account("a");
    let prefix = format!("{{\"access_token\":{access:?},\"expires_in\":1,\"refresh_token\":\"r\"");
    let exact = padded_json(prefix, MAX_OAUTH_TOKEN_BODY_BYTES);
    let mut oversized = exact.clone();
    oversized.insert(oversized.len() - 2, b'p');
    let prior = CredentialRecord {
        credential_type: "oauth".to_owned(),
        access,
        refresh: "prior-refresh".to_owned(),
        expires: 1,
        account_id: "a".to_owned(),
        is_fedramp: false,
    };

    for (body, accepted) in [(exact, true), (oversized, false)] {
        let delivered = Cell::new(false);
        let result = refresh_credential_with_transport(&prior, 0, |endpoint, fields, maximum| {
            assert_eq!(endpoint, "https://auth.openai.com/oauth/token");
            assert_eq!(
                fields,
                [
                    ("grant_type", "refresh_token"),
                    ("refresh_token", prior.refresh.as_str()),
                    ("client_id", "app_EMoamEEZ73f0CkXaXp7hrann"),
                ]
            );
            assert_eq!(maximum, MAX_OAUTH_TOKEN_BODY_BYTES);
            if body.len() > maximum {
                return Err(RuntimeError::Protocol(
                    "fixture response exceeds selected maximum".to_owned(),
                ));
            }
            delivered.set(true);
            Ok(body)
        });
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(delivered.get(), accepted);
    }
}

#[test]
fn oauth_refresh_requires_a_replacement_access_token() {
    let prior = CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: jwt_with_account("account"),
        refresh: "prior-refresh".to_owned(),
        expires: 1,
        account_id: "account".to_owned(),
        is_fedramp: false,
    };

    for body in [b"{}".as_slice(), br#"{"expires_in":3600}"#] {
        assert!(refresh_credential_with_transport(&prior, 0, |_, _, _| Ok(body.to_vec())).is_err());
    }
}

#[test]
fn oauth_refresh_uses_returned_access_expiry_and_id_token_routing() {
    let prior = CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: "prior-access".to_owned(),
        refresh: "prior-refresh".to_owned(),
        expires: 1,
        account_id: "account".to_owned(),
        is_fedramp: false,
    };
    let access_payload = base64url_encode(br#"{"exp":2}"#);
    let access = format!("e30.{access_payload}.x");
    let body = serde_json::to_vec(&serde_json::json!({
        "access_token": access,
        "id_token": super::jwt_with_account_routing("account", Some(true))
    }))
    .expect("refresh JSON");

    let refreshed = refresh_credential_with_transport(&prior, 1_000, |_, _, _| Ok(body))
        .expect("returned refresh fields replace prior values");

    assert_eq!(refreshed.access, access);
    assert_eq!(refreshed.refresh, prior.refresh);
    assert_eq!(refreshed.expires, 2_000);
    assert!(refreshed.is_fedramp);

    let changed_account = serde_json::to_vec(&serde_json::json!({
        "id_token": super::jwt_with_account("other-account")
    }))
    .expect("changed-account JSON");
    assert!(
        refresh_credential_with_transport(&prior, 1_000, |_, _, _| Ok(changed_account)).is_err()
    );
}

#[test]
fn oauth_refresh_access_expiry_fallback_obeys_the_one_day_budget() {
    let prior = CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: "prior-access".to_owned(),
        refresh: "prior-refresh".to_owned(),
        expires: 1,
        account_id: "account".to_owned(),
        is_fedramp: false,
    };

    for (expiry_seconds, accepted) in [(86_401, true), (86_402, false)] {
        let access_payload = base64url_encode(format!(r#"{{"exp":{expiry_seconds}}}"#).as_bytes());
        let access = format!("e30.{access_payload}.x");
        let body =
            serde_json::to_vec(&serde_json::json!({"access_token": access})).expect("refresh JSON");

        let refreshed =
            refresh_credential_with_transport(&prior, 1_000, |_, _, _| Ok(body.clone()));

        assert_eq!(refreshed.is_ok(), accepted, "expiry {expiry_seconds}");
    }
}

#[test]
fn authentication_http_response_enforces_status_and_size() {
    fn request(response: &'static str, maximum: usize) -> Result<Vec<u8>, RuntimeError> {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake auth server binds");
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("one auth request");
            let mut request = [0_u8; 4096];
            assert!(stream.read(&mut request).expect("request reads") > 0);
            stream
                .write_all(response.as_bytes())
                .expect("response writes");
        });
        let result = post_json_with_deadlines(
            &endpoint,
            &serde_json::json!({"request": true}),
            maximum,
            HttpDeadlines {
                connect: Duration::from_secs(1),
                header: Duration::from_secs(1),
                read: Duration::from_secs(1),
                overall: Duration::from_secs(2),
            },
        );
        server.join().expect("fake auth server completes");
        result
    }

    assert_eq!(
        request(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            2,
        )
        .expect("bounded response"),
        b"{}"
    );
    assert!(
        request(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            2,
        )
        .is_err()
    );
    assert!(
        request(
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
            2,
        )
        .is_err()
    );
    assert!(request(
        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
        2,
    )
    .is_err());
}

#[test]
fn auth_body_deadline() {
    assert_eq!(AUTH_HTTP_DEADLINES.read, Duration::from_secs(30));
    let (endpoint, connected, writes, written, server) = spawn_scripted_http_server("/token");
    block_on_paused_network(async {
        let client = build_http_client(HttpDeadlines {
            read: AUTH_HTTP_DEADLINES.read + Duration::from_secs(1),
            overall: AUTH_HTTP_DEADLINES.overall + Duration::from_secs(1),
            ..AUTH_HTTP_DEADLINES
        })
        .expect("auth client builds");
        let request = tokio::spawn(send_auth_request_async(
            client.post(endpoint).json(&serde_json::json!({})),
            16,
            AUTH_HTTP_DEADLINES,
        ));
        settle_pending(&request).await;
        connected
            .recv_timeout(Duration::from_secs(2))
            .expect("auth client connects");
        send_scripted_http_bytes(
            &writes,
            &written,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\nx",
        );
        settle_pending(&request).await;
        tokio::time::advance(AUTH_HTTP_DEADLINES.read - Duration::from_nanos(1)).await;
        assert_pending(&request).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        assert!(expect_ready(request).await.is_err());
    });
    drop(writes);
    server.join().expect("fake auth server completes");
}

#[test]
fn auth_header_deadline() {
    assert_eq!(AUTH_HTTP_DEADLINES.header, Duration::from_secs(30));
    let (endpoint, connected, writes, _written, server) = spawn_scripted_http_server("/token");
    block_on_paused_network(async {
        let client = build_http_client(HttpDeadlines {
            read: AUTH_HTTP_DEADLINES.header + Duration::from_secs(1),
            overall: AUTH_HTTP_DEADLINES.overall + Duration::from_secs(1),
            ..AUTH_HTTP_DEADLINES
        })
        .expect("auth client builds");
        let request = tokio::spawn(send_auth_request_async(
            client.post(endpoint).json(&serde_json::json!({})),
            16,
            AUTH_HTTP_DEADLINES,
        ));
        assert_pending(&request).await;
        connected
            .recv_timeout(Duration::from_secs(2))
            .expect("auth client connects");
        assert_pending(&request).await;
        tokio::time::advance(AUTH_HTTP_DEADLINES.header - Duration::from_nanos(1)).await;
        assert_pending(&request).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        assert!(expect_ready(request).await.is_err());
    });
    drop(writes);
    server.join().expect("fake auth server completes");
}

#[test]
fn auth_overall_deadline() {
    assert_eq!(AUTH_HTTP_DEADLINES.overall, Duration::from_secs(60));
    let (endpoint, connected, writes, written, server) = spawn_scripted_http_server("/token");
    block_on_paused_network(async {
        let isolated_deadlines = HttpDeadlines {
            read: AUTH_HTTP_DEADLINES.overall + Duration::from_secs(1),
            ..AUTH_HTTP_DEADLINES
        };
        let client = build_http_client(isolated_deadlines).expect("auth client builds");
        let request = tokio::spawn(send_auth_request_async(
            client.post(endpoint).json(&serde_json::json!({})),
            1_000,
            isolated_deadlines,
        ));
        settle_pending(&request).await;
        connected
            .recv_timeout(Duration::from_secs(2))
            .expect("auth client connects");
        send_scripted_http_bytes(
            &writes,
            &written,
            "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n",
        );
        settle_pending(&request).await;
        tokio::time::advance(AUTH_HTTP_DEADLINES.overall - Duration::from_nanos(1)).await;
        assert_pending(&request).await;
        tokio::time::advance(Duration::from_nanos(1)).await;
        assert!(expect_ready(request).await.is_err());
    });
    drop(writes);
    server.join().expect("fake auth server completes");
}
