use super::{flow_command, test_support::empty_workspace_under};
use std::{fs, path::Path};

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
            .env("APPDATA", config_root.as_os_str())
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
fn executor_commands_fail_closed_on_unsupported_platform_without_config_mutation() {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return;
    }
    let config_root = empty_workspace_under(Path::new(env!("CARGO_TARGET_TMPDIR")));
    let candidate = config_root.join("flow-executor");
    let candidate = candidate.to_str().expect("candidate path is UTF-8");
    let commands = [
        vec!["executor", "check"],
        vec!["executor", "configure", "--path", candidate],
        vec!["executor", "configure", "--default"],
    ];

    for args in commands {
        let output = flow_command()
            .env_remove("PATH")
            .env("APPDATA", config_root.as_os_str())
            .env("HOME", config_root.as_os_str())
            .env("XDG_CONFIG_HOME", config_root.as_os_str())
            .args(args)
            .output()
            .expect("flow command runs without PATH");

        assert_eq!(output.status.code(), Some(65), "{output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            "error: executor_policy_unsupported: productive Executor support requires Ubuntu 24.04 x64\n"
        );
    }
    assert!(
        fs::read_dir(&*config_root)
            .expect("configuration root remains readable")
            .next()
            .is_none(),
        "unsupported commands must not create configuration state"
    );
}
