use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::pi::settings::{agent_dir_from, compacts_at};

const WINDOW: u64 = 200_000;
const FAUX: &str = "faux/faux-1";

/// The threshold for each `(global, project, model, reserve)` case: the reserve pi keeps for the
/// response, or `None` when it is unknown. pi reads a project's settings only when it trusts the
/// project, which depends on extensions Watershed cannot see, so a project file that sets
/// `compaction` leaves the threshold unknown. Otherwise the global settings decide, and a model's
/// override beats the ordinary reserve.
#[test]
fn compacts_the_window_less_the_global_reserve_and_gives_none_when_the_project_sets_compaction() {
    let cases = [
        (None, None, FAUX, Some(16_384)),
        // The global reserve applies where the project's file says nothing about compaction.
        (
            Some(r#"{"compaction":{"reserveTokens":2000}}"#),
            None,
            FAUX,
            Some(2000),
        ),
        (
            Some(r#"{"compaction":{"reserveTokens":2000}}"#),
            Some(r#"{"theme":"dark"}"#),
            FAUX,
            Some(2000),
        ),
        (
            Some(r#"{"compaction":{"modelOverrides":{"faux/faux-1":{"reserveTokens":500}}}}"#),
            None,
            FAUX,
            Some(500),
        ),
        // Another model's override does not apply, and model ids keep their slashes.
        (
            Some(r#"{"compaction":{"modelOverrides":{"other/model":{"reserveTokens":500}}}}"#),
            None,
            FAUX,
            Some(16_384),
        ),
        // A project that sets compaction leaves the threshold unknown, whatever it sets.
        (
            Some(r#"{"compaction":{"reserveTokens":2000}}"#),
            Some(r#"{"compaction":{"reserveTokens":1000}}"#),
            FAUX,
            None,
        ),
        (None, Some(r#"{"compaction":{}}"#), FAUX, None),
        (
            None,
            Some(
                r#"{"compaction":{"modelOverrides":{"openrouter/anthropic/claude":{"reserveTokens":700}}}}"#,
            ),
            "openrouter/anthropic/claude",
            None,
        ),
        // A value pi refuses, or a file it cannot parse, leaves the threshold unknown.
        (
            Some(r#"{"compaction":{"reserveTokens":"lots"}}"#),
            None,
            FAUX,
            None,
        ),
        (Some("{not json"), None, FAUX, None),
        (None, Some("{not json"), FAUX, None),
        // A setting the model's override does not use is still refused when invalid.
        (
            Some(
                r#"{"compaction":{"reserveTokens":-1,"modelOverrides":{"faux/faux-1":{"reserveTokens":500}}}}"#,
            ),
            None,
            FAUX,
            None,
        ),
    ];
    for (index, (global, project, model, reserve)) in cases.into_iter().enumerate() {
        let root = std::env::temp_dir().join(format!(
            "meta-harness-compaction-{}-{index}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (global_dir, project_dir) = (root.join("agent"), root.join("project"));
        write(&global_dir.join("settings.json"), global);
        write(&project_dir.join(".pi").join("settings.json"), project);
        let threshold = compacts_at(&global_dir, &project_dir, Some(model), WINDOW);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(
            threshold,
            reserve.map(|reserve| WINDOW - reserve),
            "case {index}"
        );
    }
}

/// pi strips a leading byte order mark before it parses a settings file.
#[test]
fn reads_a_settings_file_that_begins_with_a_byte_order_mark() {
    let root = std::env::temp_dir().join(format!(
        "meta-harness-compaction-bom-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let (global_dir, project_dir) = (root.join("agent"), root.join("project"));
    write(
        &global_dir.join("settings.json"),
        Some("\u{feff}{\"compaction\":{\"reserveTokens\":1000}}"),
    );
    let threshold = compacts_at(&global_dir, &project_dir, Some(FAUX), WINDOW);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(threshold, Some(WINDOW - 1000));
}

/// `PI_CODING_AGENT_DIR` with a leading `~` starts in the home folder, as pi expands it; without
/// the variable, pi's folder lies under the home folder.
#[test]
fn reads_the_agent_dir_variable_with_a_leading_tilde_expanded() {
    let home = Some(OsString::from("/home/pi"));
    let dir = |var: &str| agent_dir_from(Some(OsString::from(var)), home.clone());
    assert_eq!(dir("~/agents"), Some(PathBuf::from("/home/pi/agents")));
    assert_eq!(dir("~"), Some(PathBuf::from("/home/pi")));
    assert_eq!(dir("/srv/pi"), Some(PathBuf::from("/srv/pi")));
    assert_eq!(
        agent_dir_from(Some(OsString::new()), home.clone()),
        Some(PathBuf::from("/home/pi/.pi/agent"))
    );
    assert_eq!(
        agent_dir_from(None, home),
        Some(PathBuf::from("/home/pi/.pi/agent"))
    );
    assert_eq!(agent_dir_from(None, None), None);
}

/// Writes `text` into `path` when it is given; a missing file is what pi reads as defaults.
fn write(path: &Path, text: Option<&str>) {
    if let Some(text) = text {
        std::fs::create_dir_all(path.parent().expect("settings folder")).expect("folder");
        std::fs::write(path, text).expect("settings file");
    }
}

/// A pipe as the project's settings file, plain or linked, gives no threshold without waiting for a
/// writer that never comes.
#[test]
fn a_pipe_as_the_project_settings_file_is_refused_without_waiting() {
    for linked in [false, true] {
        let root = std::env::temp_dir().join(format!(
            "meta-harness-settings-pipe-{}-{linked}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (global, project) = (root.join("agent"), root.join("project"));
        let settings = project.join(".pi").join("settings.json");
        std::fs::create_dir_all(settings.parent().expect("settings folder")).expect("folder");
        if linked {
            super::pipe(&root.join("pipe"));
            std::os::unix::fs::symlink(root.join("pipe"), &settings).expect("link");
        } else {
            super::pipe(&settings);
        }
        let threshold =
            super::unblocked(move || compacts_at(&global, &project, Some(FAUX), WINDOW));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(threshold, None, "linked: {linked}");
    }
}
