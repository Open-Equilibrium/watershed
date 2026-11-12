use super::{flow_command, test_support::empty_workspace_under};
use std::{fs, io::Write, path::Path, process::Output};

const SYNTHETIC_CREDENTIAL: &str = r#"{"openai-codex":{"type":"oauth","access":"test-access","refresh":"test-refresh","expires":4102444800000,"accountId":"test-account","isFedramp":false}}"#;

fn run_auth(config_root: &Path, action: &str) -> Output {
    flow_command()
        .current_dir(config_root)
        .env("HOME", config_root)
        .env("XDG_CONFIG_HOME", config_root)
        .args(["auth", action, "openai-codex"])
        .output()
        .expect("authentication command should run")
}

fn credential_path(config_root: &Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        config_root.join("Library/Application Support/flow-agent/credentials.json")
    } else {
        config_root.join("flow-agent/credentials.json")
    }
}

fn run_endpoint_auth(config_root: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = flow_command()
        .current_dir(config_root)
        .env("HOME", config_root)
        .env("XDG_CONFIG_HOME", config_root)
        .args(["auth"])
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("authentication command should run");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("key is written to stdin");
    child
        .wait_with_output()
        .expect("authentication command ends")
}

#[test]
fn endpoint_login_reads_the_key_from_stdin_and_manages_only_its_slot() {
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    let base_url = "https://models.example.test/v1";

    let missing = run_endpoint_auth(&config_root, &["status", "openai-compatible"], "");
    assert_eq!(missing.stdout, b"openai-compatible not authenticated\n");

    let login = run_endpoint_auth(
        &config_root,
        &[
            "login",
            "openai-compatible",
            "--base-url",
            base_url,
            "--api-key",
        ],
        "sk-cli-fixture\n",
    );
    assert!(login.status.success(), "{login:?}");
    assert_eq!(login.stdout, b"openai-compatible authenticated\n");
    let document: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(credential_path(&config_root)).expect("credential document reads"),
    )
    .expect("credential document is JSON");
    assert_eq!(
        document["openai-compatible"],
        serde_json::json!({"type": "api-key", "baseUrl": base_url, "key": "sk-cli-fixture"})
    );

    let rejected = run_endpoint_auth(
        &config_root,
        &[
            "login",
            "openai-compatible",
            "--base-url",
            "http://models.example.test/v1",
            "--api-key",
        ],
        "sk-rejected-fixture\n",
    );
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(
        !String::from_utf8_lossy(&rejected.stderr).contains("sk-rejected-fixture"),
        "{rejected:?}"
    );

    let logout = run_endpoint_auth(&config_root, &["logout", "openai-compatible"], "");
    assert_eq!(logout.stdout, b"openai-compatible authentication removed\n");
    let absent = run_endpoint_auth(&config_root, &["logout", "openai-compatible"], "");
    assert_eq!(absent.stdout, b"openai-compatible was not authenticated\n");
}

#[test]
fn auth_status_reports_an_undecodable_record_as_reauthentication_required() {
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    assert!(run_auth(&config_root, "logout").status.success());
    fs::write(credential_path(&config_root), "not a credential document")
        .expect("undecodable credential is staged");

    let status = run_auth(&config_root, "status");

    assert!(status.status.success(), "{status:?}");
    assert!(status.stderr.is_empty(), "{status:?}");
    assert_eq!(
        status.stdout,
        b"openai-codex authentication is required; run flow auth login openai-codex\n"
    );
}

#[test]
fn auth_status_and_logout_manage_only_the_isolated_flow_agent_credential() {
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));

    let missing = run_auth(&config_root, "status");
    assert!(missing.status.success(), "{missing:?}");
    assert!(missing.stderr.is_empty());
    assert_eq!(missing.stdout, b"openai-codex not authenticated\n");

    let absent_logout = run_auth(&config_root, "logout");
    assert!(absent_logout.status.success(), "{absent_logout:?}");
    assert!(absent_logout.stderr.is_empty());
    assert_eq!(
        absent_logout.stdout,
        b"openai-codex was not authenticated\n"
    );

    let credential = credential_path(&config_root);
    fs::write(&credential, SYNTHETIC_CREDENTIAL).expect("synthetic credential is staged");

    let authenticated = run_auth(&config_root, "status");
    assert!(authenticated.status.success(), "{authenticated:?}");
    assert!(authenticated.stderr.is_empty());
    assert_eq!(
        authenticated.stdout,
        b"openai-codex authenticated; credential expires at Unix epoch millisecond 4102444800000\n"
    );

    let logout = run_auth(&config_root, "logout");
    assert!(logout.status.success(), "{logout:?}");
    assert!(logout.stderr.is_empty());
    assert_eq!(logout.stdout, b"openai-codex authentication removed\n");
    assert_eq!(
        fs::read_to_string(credential).expect("credential document remains readable"),
        "{}\n"
    );
}
