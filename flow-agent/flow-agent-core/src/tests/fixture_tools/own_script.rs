use super::super::helpers::fixture_runtime_policy;
use crate::runtime::{
    fixture_tools::{
        compile_own_script_operations, evaluate_script_command, normalize_script_write_target,
        script_redirection,
    },
    types::RuntimeError,
};

#[test]
fn helpers_reject_unsupported_m1_shell_shapes() {
    let (_registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let command_policy = policy
        .commands
        .iter()
        .find(|command| command.tool_id == "write-summary")
        .expect("write-summary policy exists");

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
    assert!(matches!(
        script_redirection("printf 'x' >> out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("append redirection")
    ));
    assert!(matches!(
        script_redirection("> out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("must include a command")
    ));
    assert!(matches!(
        script_redirection("printf 'x' > out/a > out/b"),
        Err(RuntimeError::Protocol(message)) if message.contains("multiple redirections")
    ));
    assert!(matches!(
        script_redirection("printf 'unterminated > out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("unterminated quote")
    ));
    assert!(matches!(
        script_redirection("printf 'x' > out/summary one.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("one literal path")
    ));
    assert!(matches!(
        script_redirection("printf 'x' > \"out/summary.txt\"suffix"),
        Err(RuntimeError::Protocol(message)) if message.contains("one literal path")
    ));

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
    assert!(matches!(
        evaluate_script_command("printf \"bad\""),
        Err(RuntimeError::Protocol(message)) if message.contains("single-quoted")
    ));
    assert!(matches!(
        evaluate_script_command("printf 'bad"),
        Err(RuntimeError::Protocol(message)) if message.contains("unterminated")
    ));
    assert!(matches!(
        evaluate_script_command("printf 'bad\\t'"),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported")
    ));
    assert!(matches!(
        evaluate_script_command("printf 'bad\\'"),
        Err(RuntimeError::Protocol(message)) if message.contains("dangling escape")
    ));
    assert!(matches!(
        evaluate_script_command("printf '%s' OTHER"),
        Err(RuntimeError::Protocol(message)) if message.contains("printf argument")
    ));
    assert!(matches!(
        evaluate_script_command("echo $SUMMARY"),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported own-script argument")
    ));
    assert!(matches!(
        evaluate_script_command("echo \"$SUMMARY\""),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported own-script argument")
    ));
    assert!(matches!(
        evaluate_script_command("cat out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported own-script command")
    ));

    assert!(
        compile_own_script_operations(command_policy, "\n# comment\n---\necho noop\n")
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
