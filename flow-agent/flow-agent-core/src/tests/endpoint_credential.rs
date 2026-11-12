use crate::runtime::endpoint_credential::EndpointCredential;

const BASE_URL: &str = "https://models.example.test/v1";

#[test]
fn endpoint_base_url_is_https_or_a_loopback_ip_literal() {
    for accepted in [
        "https://models.example.test/v1",
        "https://models.example.test",
        "http://127.0.0.1:8080/v1",
        "http://[::1]:8080/v1",
    ] {
        EndpointCredential::new(accepted, "sk-fixture")
            .unwrap_or_else(|error| panic!("{accepted} rejected: {error}"));
    }
    for rejected in [
        "http://models.example.test/v1",
        "http://localhost:8080/v1",
        "ftp://models.example.test/v1",
        "https://user:secret-fixture@models.example.test/v1",
        "https://models.example.test/v1?token=secret-fixture",
        "https://models.example.test/v1#secret-fixture",
        "not a url",
        "",
    ] {
        let error = EndpointCredential::new(rejected, "sk-fixture")
            .err()
            .unwrap_or_else(|| panic!("{rejected} accepted"));
        let message = error.to_string();
        assert!(
            !message.contains("secret-fixture") && !message.contains("sk-fixture"),
            "rejection for {rejected} echoed a secret: {message}"
        );
    }
}

#[test]
fn endpoint_key_must_be_a_visible_header_value() {
    for rejected in ["", "has space", "tab\there", "line\nbreak", "naïve"] {
        assert!(
            EndpointCredential::new(BASE_URL, rejected).is_err(),
            "accepted key {rejected:?}"
        );
    }
    EndpointCredential::new(BASE_URL, "sk-proj-0123_fixture").expect("visible key is valid");
}

#[test]
fn endpoint_credential_debug_never_prints_the_key() {
    let credential =
        EndpointCredential::new(BASE_URL, "sk-secret-fixture").expect("valid endpoint");
    let debug = format!("{credential:?}");
    assert!(!debug.contains("sk-secret-fixture"), "{debug}");
}
