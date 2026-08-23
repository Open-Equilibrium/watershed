use super::{flow_command, test_support::empty_workspace_under};
use std::{fs, path::Path, process::Output};

const SYNTHETIC_CREDENTIAL: &str = r#"{"openai-codex":{"type":"oauth","access":"test-access","refresh":"test-refresh","expires":4102444800000,"accountId":"test-account","isFedramp":false}}"#;

fn run_auth(config_root: &Path, action: &str) -> Output {
    flow_command()
        .current_dir(config_root)
        .env("APPDATA", config_root)
        .env("HOME", config_root)
        .env("XDG_CONFIG_HOME", config_root)
        .args(["auth", action, "openai-codex"])
        .output()
        .expect("authentication command should run")
}

fn credential_path(config_root: &Path) -> std::path::PathBuf {
    if cfg!(windows) {
        config_root.join("flow-agent/credentials.json")
    } else if cfg!(target_os = "macos") {
        config_root.join("Library/Application Support/flow-agent/credentials.json")
    } else {
        config_root.join("flow-agent/credentials.json")
    }
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
