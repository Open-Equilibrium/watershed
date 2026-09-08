use super::{flow_command, test_support::empty_workspace_under};
use std::path::Path;

#[test]
fn executor_help_is_specific_successful_and_human_readable() {
    for (args, expected) in [
        (
            vec!["executor", "--help"],
            concat!(
                "Usage:\n",
                "  flow executor check\n",
                "  flow executor configure --path <absolute-path>\n",
                "  flow executor configure --default\n",
            ),
        ),
        (
            vec!["executor", "check", "--help"],
            concat!(
                "Usage:\n",
                "  flow executor check\n",
                "\n",
                "Checks the configured Executor and reports its readiness.\n",
            ),
        ),
        (
            vec!["executor", "configure", "--help"],
            concat!(
                "Usage:\n",
                "  flow executor configure --path <absolute-path>\n",
                "  flow executor configure --default\n",
                "\n",
                "Options:\n",
                "  --path <absolute-path>  Select an administrator-supplied Executor.\n",
                "  --default               Remove the Custom override and restore default sibling resolution.\n",
            ),
        ),
    ] {
        let output = flow_command()
            .args(args)
            .output()
            .expect("flow help command runs");

        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            std::str::from_utf8(&output.stdout).expect("stdout is UTF-8"),
            expected
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn executor_commands_have_closed_grammar() {
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    for (args, expected) in [
        (vec!["executor"], "usage: flow executor"),
        (
            vec!["executor", "configure"],
            "usage: flow executor configure",
        ),
        (
            vec!["executor", "configure", "--path", "relative"],
            "absolute",
        ),
        (
            vec!["executor", "configure", "--default", "--path", "x"],
            "usage: flow executor configure",
        ),
        (vec!["executor", "unknown"], "usage: flow executor"),
    ] {
        let output = flow_command()
            .env("HOME", config_root.as_os_str())
            .env("XDG_CONFIG_HOME", config_root.as_os_str())
            .args(args)
            .output()
            .expect("flow command runs");

        assert_eq!(output.status.code(), Some(64), "{output:?}");
        let stderr = String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8");
        assert!(stderr.contains(expected), "{expected}: {output:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn executor_default_reports_when_no_custom_override_existed() {
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    let output = flow_command()
        .env("HOME", config_root.as_os_str())
        .env("XDG_CONFIG_HOME", config_root.as_os_str())
        .args(["executor", "configure", "--default"])
        .output()
        .expect("flow executor configuration runs");

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        std::str::from_utf8(&output.stdout).expect("stdout is UTF-8"),
        "Default sibling resolution already active\n"
    );
    assert!(output.stderr.is_empty(), "{output:?}");
}
