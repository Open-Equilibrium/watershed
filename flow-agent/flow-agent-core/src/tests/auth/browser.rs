use super::{jwt_with_account, token_body};
use crate::{
    runtime::{
        RuntimeError,
        auth::{
            AuthStatus, BrowserLauncher, auth_status_from_store, logout_from_store,
            parse_token_body, run_browser_login_with_components, store_login_credential,
            system_browser_launcher,
        },
        credential_store::CredentialStore,
        oauth_credential::CredentialRecord,
    },
    tests::helpers::empty_workspace,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};

#[test]
fn system_browser_launcher_does_not_depend_on_path_lookup() {
    #[cfg(windows)]
    assert_eq!(system_browser_launcher(), BrowserLauncher::NativeWindows);
    #[cfg(unix)]
    match system_browser_launcher() {
        BrowserLauncher::Executable(executable) => assert!(
            std::path::Path::new(executable).is_absolute(),
            "browser launcher must be an absolute trusted executable: {executable}"
        ),
    }
}

#[test]
fn browser_login_presents_opens_and_exchanges_one_matching_callback() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
    let address = listener.local_addr().expect("loopback address");
    let mut presented = String::new();
    let mut opened = String::new();
    let mut exchange = None;
    let mut client = None;

    let credential = run_browser_login_with_components(
        listener,
        "fixture-state".to_owned(),
        "fixture-verifier".to_owned(),
        &mut |message| {
            presented = message.to_owned();
            Ok(())
        },
        |url| {
            opened = url.to_owned();
            client = Some(thread::spawn(move || {
                let mut stream = TcpStream::connect(address).expect("callback connects");
                stream
                    .write_all(
                        b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n",
                    )
                    .expect("invalid callback writes");
                let mut response = String::new();
                stream
                    .read_to_string(&mut response)
                    .expect("invalid callback response reads");
                let mut stream = TcpStream::connect(address).expect("valid callback connects");
                stream
                    .write_all(
                        b"GET /auth/callback?state=fixture-state&code=fixture-code HTTP/1.1\r\nHost: localhost\r\n\r\n",
                    )
                    .expect("valid callback writes");
                let mut valid_response = String::new();
                stream
                    .read_to_string(&mut valid_response)
                    .expect("valid callback response reads");
                (response, valid_response)
            }));
            Ok(())
        },
        |code, verifier, redirect| {
            exchange = Some((code.to_owned(), verifier.to_owned(), redirect.to_owned()));
            Ok(CredentialRecord {
                credential_type: "oauth".to_owned(),
                access: "access".to_owned(),
                refresh: "refresh".to_owned(),
                expires: 42,
                account_id: "account".to_owned(),
                is_fedramp: false,
            })
        },
    )
    .expect("browser login completes");

    assert!(presented.starts_with("Open this URL to authenticate: "));
    assert!(presented.contains(
        "If the browser callback does not complete, cancel and run flow auth login openai-codex --device."
    ));
    assert_eq!(
        opened,
        presented
            .strip_prefix("Open this URL to authenticate: ")
            .and_then(|message| message.split_once('\n'))
            .map(|(url, _)| url)
            .expect("presentation contains the browser URL first")
    );
    assert!(opened.contains("state=fixture-state"));
    assert_eq!(
        exchange,
        Some((
            "fixture-code".to_owned(),
            "fixture-verifier".to_owned(),
            "http://localhost:1455/auth/callback".to_owned(),
        ))
    );
    assert_eq!(credential.expires, 42);
    let (invalid_response, valid_response) = client
        .expect("callback client starts")
        .join()
        .expect("callback client completes");
    assert!(invalid_response.starts_with("HTTP/1.1 400 Bad Request"));
    assert!(valid_response.starts_with("HTTP/1.1 200 OK"));
}

#[test]
fn browser_login_does_not_claim_completion_before_exchange_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
    let address = listener.local_addr().expect("loopback address");
    let mut client = None;

    let error = run_browser_login_with_components(
        listener,
        "fixture-state".to_owned(),
        "fixture-verifier".to_owned(),
        &mut |_| Ok(()),
        |_| {
            client = Some(thread::spawn(move || {
                let mut stream = TcpStream::connect(address).expect("callback connects");
                stream
                    .write_all(
                        b"GET /auth/callback?state=fixture-state&code=fixture-code HTTP/1.1\r\nHost: localhost\r\n\r\n",
                    )
                    .expect("callback writes");
                let mut response = String::new();
                stream
                    .read_to_string(&mut response)
                    .expect("callback response reads");
                response
            }));
            Ok(())
        },
        |_, _, _| Err(RuntimeError::Protocol("exchange rejected".to_owned())),
    )
    .expect_err("exchange failure rejects browser login");

    assert!(matches!(error, RuntimeError::Protocol(message) if message == "exchange rejected"));
    let response = client
        .expect("callback client starts")
        .join()
        .expect("callback client completes");
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(!response.contains("Authentication complete"));
    assert!(response.contains("terminal for the final result"));
}

#[test]
fn auth_status_and_logout_are_redacted_store_operations() {
    let workspace = empty_workspace("redacted-auth-store");
    let store = CredentialStore::at(workspace.join("credentials.json"));

    assert_eq!(
        auth_status_from_store(&store).expect("empty status"),
        AuthStatus {
            authenticated: false,
            expires_epoch_milliseconds: None,
        }
    );
    let credential = parse_token_body(
        &token_body(&jwt_with_account("account"), "refresh", 60.into()),
        1_000,
    )
    .expect("fixture credential");
    let status = store_login_credential(&store, credential).expect("credential stores");
    assert!(status.authenticated);
    assert!(status.expires_epoch_milliseconds.is_some());
    assert_eq!(
        auth_status_from_store(&store).expect("stored status"),
        status
    );
    assert!(logout_from_store(&store).expect("credential removes"));
    assert!(!logout_from_store(&store).expect("second logout is empty"));
}
