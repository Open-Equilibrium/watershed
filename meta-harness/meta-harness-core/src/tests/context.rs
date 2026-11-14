use std::path::Path;

use crate::context::compacts_at;

const WINDOW: u64 = 200_000;
const FAUX: &str = "faux/faux-1";

/// The settings pi reads, as `(global, project, model, reserve)`: the reserve pi keeps for the
/// response, or `None` when a file pi would refuse makes the threshold unknown. Files merge key by
/// key, project over global, before a model's override wins over the ordinary value.
#[test]
fn compacts_the_window_less_the_reserve_pi_reads_from_its_settings() {
    let cases = [
        (None, None, FAUX, Some(16_384)),
        (
            Some(r#"{"compaction":{"reserveTokens":2000}}"#),
            Some(r#"{"compaction":{"reserveTokens":1000}}"#),
            FAUX,
            Some(1000),
        ),
        // A global model override beats the project's ordinary value, as the files merge first.
        (
            Some(r#"{"compaction":{"modelOverrides":{"faux/faux-1":{"reserveTokens":500}}}}"#),
            Some(r#"{"compaction":{"reserveTokens":1000}}"#),
            FAUX,
            Some(500),
        ),
        (
            Some(r#"{"compaction":{"modelOverrides":{"faux/faux-1":{"reserveTokens":500}}}}"#),
            Some(r#"{"compaction":{"modelOverrides":{"faux/faux-1":{"reserveTokens":300}}}}"#),
            FAUX,
            Some(300),
        ),
        // Another model's override does not apply, and model ids keep their slashes.
        (
            Some(r#"{"compaction":{"modelOverrides":{"other/model":{"reserveTokens":500}}}}"#),
            None,
            FAUX,
            Some(16_384),
        ),
        (
            None,
            Some(
                r#"{"compaction":{"modelOverrides":{"openrouter/anthropic/claude":{"reserveTokens":700}}}}"#,
            ),
            "openrouter/anthropic/claude",
            Some(700),
        ),
        // A value pi refuses, or a file it cannot parse, leaves the threshold unknown.
        (
            None,
            Some(r#"{"compaction":{"reserveTokens":"lots"}}"#),
            FAUX,
            None,
        ),
        (Some("{not json"), None, FAUX, None),
        // A setting the model's override does not use is still refused when invalid.
        (
            None,
            Some(
                r#"{"compaction":{"reserveTokens":-1,"modelOverrides":{"faux/faux-1":{"reserveTokens":500}}}}"#,
            ),
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

/// Writes `text` into `path` when it is given; a missing file is what pi reads as defaults.
fn write(path: &Path, text: Option<&str>) {
    if let Some(text) = text {
        std::fs::create_dir_all(path.parent().expect("settings folder")).expect("folder");
        std::fs::write(path, text).expect("settings file");
    }
}
