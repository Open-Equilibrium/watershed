use crate::runtime::productive::productive_execution_supported_release;

#[test]
fn productive_execution_support_matrix_is_closed() {
    assert!(productive_execution_supported_release(
        "linux",
        "x86_64",
        "ID=ubuntu\nVERSION_ID=24.04\n"
    ));
    assert!(productive_execution_supported_release(
        "macos", "aarch64", "26.0"
    ));

    for (target_os, target_arch) in [
        ("linux", "aarch64"),
        ("macos", "x86_64"),
        ("freebsd", "x86_64"),
        ("windows", "x86_64"),
    ] {
        assert!(
            !productive_execution_supported_release(
                target_os,
                target_arch,
                "ID=ubuntu\nVERSION_ID=24.04\n",
            ),
            "{target_os}/{target_arch} must be unavailable"
        );
    }
}

#[test]
fn productive_execution_support_requires_the_exact_pinned_release() {
    assert!(productive_execution_supported_release(
        "linux",
        "x86_64",
        "ID=ubuntu\nVERSION_ID=\"24.04\"\n"
    ));
    for (target_os, target_arch, release) in [
        ("linux", "x86_64", "ID=ubuntu\nVERSION_ID=\"24.10\"\n"),
        ("linux", "x86_64", "ID=debian\nVERSION_ID=\"24.04\"\n"),
        ("linux", "x86_64", "ID=ubuntu\n"),
        ("macos", "aarch64", "25.9"),
        ("macos", "aarch64", "260"),
    ] {
        assert!(
            !productive_execution_supported_release(target_os, target_arch, release),
            "{target_os}/{target_arch}/{release:?} must be unavailable"
        );
    }
}
