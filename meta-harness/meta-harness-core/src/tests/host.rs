use std::path::{Path, PathBuf};

use serde_json::Value;

use super::fake_t3::Fake;
use super::{Dir, put, read};
use crate::AcpAgent;
use crate::host::write_settings;

fn home() -> Dir {
    let home = Dir::new();
    std::fs::create_dir(home.join("userdata")).unwrap();
    home
}

fn settings(home: &Path) -> PathBuf {
    home.join("userdata").join("settings.json")
}

fn agent() -> AcpAgent {
    AcpAgent {
        id: "scripted".to_owned(),
        command: PathBuf::from("/usr/bin/python3"),
        args: vec!["agent.py".to_owned()],
        models: vec!["scripted".to_owned()],
    }
}

#[test]
fn an_unreadable_settings_file_is_kept_and_reported() {
    for text in [
        "{not json",
        "[]",
        "null",
        r#"{"providerInstances":[]}"#,
        r#"{"providerInstances":"x"}"#,
    ] {
        let home = home();
        put(settings(&home), text);
        assert!(write_settings(&home, &[agent()]).is_err(), "{text}");
        assert_eq!(read(&settings(&home)), text);
    }
}

#[cfg(unix)]
#[test]
fn a_replaced_settings_file_stays_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let home = home();
    std::fs::write(settings(&home), r#"{"theme":"dark"}"#).unwrap();
    std::fs::set_permissions(settings(&home), std::fs::Permissions::from_mode(0o600)).unwrap();
    write_settings(&home, &[agent()]).unwrap();
    let mode = std::fs::metadata(settings(&home))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn a_settings_write_that_cannot_finish_keeps_the_previous_settings() {
    use std::os::unix::fs::PermissionsExt;

    let home = home();
    let previous = r#"{"theme":"dark"}"#;
    put(settings(&home), previous);
    std::fs::set_permissions(
        home.join("userdata"),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    assert!(write_settings(&home, &[agent()]).is_err());
    assert_eq!(read(&settings(&home)), previous);
    std::fs::set_permissions(
        home.join("userdata"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
}

#[test]
fn the_agents_are_merged_into_the_settings_and_no_temporary_file_is_left() {
    let home = home();
    put(settings(&home), r#"{"theme":"dark"}"#);
    write_settings(&home, &[agent()]).unwrap();
    let written: Value = serde_json::from_str(&read(&settings(&home))).unwrap();
    assert_eq!(written["theme"], "dark");
    assert_eq!(
        written["providerInstances"]["scripted"]["config"]["commandPath"],
        "/usr/bin/python3"
    );
    let files: Vec<String> = std::fs::read_dir(home.join("userdata"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["settings.json"]);
}

#[test]
fn a_remote_name_with_a_slash_is_removed_whole_before_the_branch_is_compared() {
    use crate::host::projects::tracking_reason;
    let tracks = Some("refs/remotes/team/origin/topic");
    assert_eq!(tracking_reason("topic", "team/origin", tracks), None);
    assert!(tracking_reason("origin/topic", "team/origin", tracks).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_never_lists_its_projects_fails_within_the_wait() {
    let fake = Fake::start(|_, _, _| Vec::new()).await;
    let client =
        crate::t3::Client::connect(&fake.origin, || Box::pin(async { Ok("token".to_owned()) }))
            .await
            .unwrap();
    let listed = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        crate::host::projects::projects(&client, None),
    )
    .await;
    assert!(
        matches!(listed, Ok(Err(_))),
        "the project list gives up at its wait instead of hanging"
    );
}
