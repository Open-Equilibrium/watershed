//! T3's settings file: the agents Meta-Harness writes into it, and what happens to the rest
//! (ADR-0282).

use std::path::PathBuf;

use serde_json::Value;

use crate::AcpAgent;
use crate::host::write_settings;

/// A T3 data folder with its `userdata` folder, removed on drop.
struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let home = std::env::temp_dir().join(format!("mh-home-{name}"));
        std::fs::create_dir_all(home.join("userdata")).unwrap();
        Self(home)
    }

    fn settings(&self) -> PathBuf {
        self.0.join("userdata").join("settings.json")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        // A test may have made the folder read-only.
        #[cfg(unix)]
        let _ = std::fs::set_permissions(
            self.0.join("userdata"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        );
        let _ = std::fs::remove_dir_all(&self.0);
    }
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
    // A providerInstances value that is no object cannot take the agents either.
    for text in [
        "{not json",
        "[]",
        "null",
        r#"{"providerInstances":[]}"#,
        r#"{"providerInstances":"x"}"#,
    ] {
        let home = Home::new();
        std::fs::write(home.settings(), text).unwrap();
        assert!(write_settings(&home.0, &[agent()]).is_err(), "{text}");
        assert_eq!(std::fs::read_to_string(home.settings()).unwrap(), text);
    }
}

// A read-only folder needs Unix permissions; the other settings tests run on every platform.
#[cfg(unix)]
#[test]
fn a_settings_write_that_cannot_finish_keeps_the_previous_settings() {
    use std::os::unix::fs::PermissionsExt;

    let home = Home::new();
    let previous = r#"{"theme":"dark"}"#;
    std::fs::write(home.settings(), previous).unwrap();
    // Read-only folder: the write cannot create its temporary file, while the old file stays
    // writable.
    std::fs::set_permissions(
        home.0.join("userdata"),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    assert!(write_settings(&home.0, &[agent()]).is_err());
    assert_eq!(std::fs::read_to_string(home.settings()).unwrap(), previous);
}

#[test]
fn the_agents_are_merged_into_the_settings_and_no_temporary_file_is_left() {
    let home = Home::new();
    std::fs::write(home.settings(), r#"{"theme":"dark"}"#).unwrap();
    write_settings(&home.0, &[agent()]).unwrap();
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(home.settings()).unwrap()).unwrap();
    assert_eq!(written["theme"], "dark");
    assert_eq!(
        written["providerInstances"]["scripted"]["config"]["commandPath"],
        "/usr/bin/python3"
    );
    let files: Vec<String> = std::fs::read_dir(home.0.join("userdata"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["settings.json"]);
}
