#[cfg(unix)]
use super::{
    flow_command,
    process::wait_with_output_before,
    test_support::{workspace_copy, workspace_session_dir},
};
#[cfg(unix)]
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};

#[cfg(unix)]
fn interrupt_and_wait_for_130(child: Child) {
    let signal = Command::new("kill")
        .args(["-s", "INT", &child.id().to_string()])
        .output()
        .expect("SIGINT sender starts");
    assert!(
        signal.status.success(),
        "{}",
        String::from_utf8_lossy(&signal.stderr)
    );
    let output = wait_with_output_before(child, Duration::from_secs(2));

    assert_eq!(output.status.code(), Some(130));
}

#[cfg(unix)]
#[test]
fn idle_sigint_exits_the_entire_program_with_130() {
    let workspace = workspace_copy("hello-flow");
    let mut child = flow_command()
        .current_dir(&workspace)
        .arg("chat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("idle chat process starts");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        child
            .try_wait()
            .expect("idle child status is readable")
            .is_none(),
        "chat must still be waiting for input"
    );

    interrupt_and_wait_for_130(child);
}

#[cfg(unix)]
#[test]
fn sigint_while_waiting_for_a_complete_root_input_exits_with_130() {
    let workspace = workspace_copy("smoke-flow");
    let mut child = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--inputs", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("stdin-backed run starts");
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        child
            .try_wait()
            .expect("stdin-backed child status is readable")
            .is_none(),
        "run must still be waiting for its complete request input"
    );

    interrupt_and_wait_for_130(child);
    assert!(
        !workspace_session_dir(&workspace).exists(),
        "an incomplete request must not create a Run"
    );
}
