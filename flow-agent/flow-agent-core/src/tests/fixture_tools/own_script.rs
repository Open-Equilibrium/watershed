use crate::runtime::{
    fixture_tools::{
        compile_own_script_operations, evaluate_script_command, normalize_script_write_target,
        script_redirection,
    },
    types::RuntimeError,
};

#[test]
fn helpers_reject_unsupported_m1_shell_shapes() {
    assert_eq!(
        script_redirection("printf 'hello > world\\n' > \"out/quoted.txt\"")
            .expect("quoted redirection parses"),
        Some((
            "printf 'hello > world\\n'".to_owned(),
            "out/quoted.txt".to_owned()
        ))
    );
    assert_eq!(
        script_redirection("printf 'hello\\n' > \"out/quoted summary.txt\"")
            .expect("quoted redirection target with spaces parses"),
        Some((
            "printf 'hello\\n'".to_owned(),
            "out/quoted summary.txt".to_owned()
        ))
    );
    assert_eq!(
        script_redirection("echo no-redirection").expect("plain command parses"),
        None
    );
    for (command, expected) in [
        ("printf 'x' >> out/summary.txt", "append redirection"),
        ("> out/summary.txt", "must include a command"),
        ("printf 'x' > out/a > out/b", "multiple redirections"),
        (
            "printf 'unterminated > out/summary.txt",
            "unterminated quote",
        ),
        ("printf 'x' > out/summary one.txt", "one literal path"),
        ("printf 'x' > \"out/summary.txt\"suffix", "one literal path"),
    ] {
        assert!(
            matches!(
                script_redirection(command),
                Err(RuntimeError::Protocol(message)) if message.contains(expected)
            ),
            "{command}"
        );
    }

    for target in [
        "",
        "/abs",
        "C:/abs",
        r"out\summary.txt",
        "out/$SUMMARY",
        "out/*.txt",
        "out/?.txt",
    ] {
        assert!(matches!(
            normalize_script_write_target(target),
            Err(RuntimeError::Protocol(message))
                if message.contains("literal workspace-relative path")
        ));
    }
    for target in [
        "out//summary.txt",
        "out/./summary.txt",
        "out/../summary.txt",
        "out/a|b",
    ] {
        assert!(matches!(
            normalize_script_write_target(target),
            Err(RuntimeError::Protocol(message)) if message.contains("inside the workspace")
        ));
    }
    for target in [
        ".ssh./id_rsa",
        "NUL",
        "out./summary.txt",
        "out/COM1",
        "out/lPt9.log",
        "out/nul.txt",
        "out/summary.txt.",
    ] {
        assert!(matches!(
            normalize_script_write_target(target),
            Err(RuntimeError::Protocol(message)) if message.contains("Windows path alias")
        ));
    }

    assert_eq!(
        evaluate_script_command("printf 'hi\\n'").expect("printf without args evaluates"),
        b"hi\n"
    );
    assert_eq!(
        evaluate_script_command("printf 'a\\\\b'").expect("printf backslash escape"),
        b"a\\b"
    );
    assert_eq!(
        evaluate_script_command("printf '%s\\n' $SUMMARY").expect("stub SUMMARY evaluates"),
        b"hello\n"
    );
    assert_eq!(
        evaluate_script_command("echo plain").expect("echo evaluates"),
        b"plain\n"
    );
    for (command, expected) in [
        ("printf \"bad\"", "single-quoted"),
        ("printf 'bad", "unterminated"),
        ("printf 'bad\\t'", "unsupported"),
        ("printf 'bad\\'", "dangling escape"),
        ("printf '%s' OTHER", "printf argument"),
        ("echo $SUMMARY", "unsupported own-script argument"),
        ("echo \"$SUMMARY\"", "unsupported own-script argument"),
        ("cat out/summary.txt", "unsupported own-script command"),
    ] {
        assert!(
            matches!(
                evaluate_script_command(command),
                Err(RuntimeError::Protocol(message)) if message.contains(expected)
            ),
            "{command}"
        );
    }

    assert!(
        compile_own_script_operations("\n# comment\n---\necho noop\n")
            .expect("noop-like lines and echo compile")
            .is_none()
    );
}

#[test]
fn printf_uses_bounded_posix_string_conversions() {
    for (command, expected) in [
        ("printf '%s:%s\\n' \"$SUMMARY\"", b"hello:\n".as_slice()),
        ("printf '%%:%s\\n' $SUMMARY", b"%:hello\n".as_slice()),
        ("printf '[%s]\\n'", b"[]\n".as_slice()),
    ] {
        assert_eq!(
            evaluate_script_command(command).expect("supported printf evaluates"),
            expected,
            "{command}"
        );
    }
    for command in [
        "printf '%d' $SUMMARY",
        "printf '%'",
        "printf '%1$s' $SUMMARY",
    ] {
        assert!(
            matches!(
                evaluate_script_command(command),
                Err(RuntimeError::Protocol(message))
                    if message.contains("unsupported own-script printf conversion")
            ),
            "{command}"
        );
    }
}
