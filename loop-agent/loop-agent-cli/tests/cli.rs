use std::{ffi::OsString, process::Command};

fn loop_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_loop"))
}

#[test]
fn version_flag_prints_package_version() {
    let output = loop_command()
        .arg("--version")
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        format!("loop {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn short_version_flag_prints_package_version() {
    let output = loop_command()
        .arg("-V")
        .output()
        .expect("loop binary should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        format!("loop {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn unknown_argument_exits_with_m0_notice() {
    let output = loop_command()
        .arg("run")
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
        format!("{}\n", loop_agent_core::m0_runtime_notice())
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn non_unicode_argument_exits_with_m0_notice() {
    let output = loop_command()
        .arg(non_unicode_argument())
        .output()
        .expect("loop binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr should be UTF-8"),
        format!("{}\n", loop_agent_core::m0_runtime_notice())
    );
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
fn non_unicode_argument() -> OsString {
    use std::os::unix::ffi::OsStringExt;

    OsString::from_vec(vec![0xff])
}

#[cfg(windows)]
fn non_unicode_argument() -> OsString {
    use std::os::windows::ffi::OsStringExt;

    OsString::from_wide(&[0xd800])
}
