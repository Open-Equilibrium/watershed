use super::flow_command;
use std::{
    io::{Read, Write},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

pub(super) fn closed_pipe_stdout() -> Stdio {
    let mut reader = flow_command()
        .arg("--version")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("pipe reader should spawn");
    let writer = reader.stdin.take().expect("pipe writer is available");
    assert!(
        reader.wait().expect("pipe reader should exit").success(),
        "pipe reader should close its stdin"
    );
    Stdio::from(writer)
}

pub(super) fn wait_with_input_and_output_before(
    child: Child,
    input: &[u8],
    timeout: Duration,
) -> Output {
    wait_with_io_before(child, Some(input.to_vec()), timeout)
}

pub(super) fn wait_with_output_before(child: Child, timeout: Duration) -> Output {
    wait_with_io_before(child, None, timeout)
}

fn wait_with_io_before(mut child: Child, input: Option<Vec<u8>>, timeout: Duration) -> Output {
    let started = Instant::now();
    let stdin_writer = input.map(|input| {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        std::thread::spawn(move || stdin.write_all(&input))
    });
    let stdout_reader =
        read_to_end_in_thread(child.stdout.take().expect("stdout is piped"), "stdout");
    let stderr_reader =
        read_to_end_in_thread(child.stderr.take().expect("stderr is piped"), "stderr");
    let status = loop {
        if let Some(status) = child.try_wait().expect("child status should be readable") {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill().expect("timed-out child should stop");
            child.wait().expect("timed-out child should be reaped");
            if let Some(stdin_writer) = stdin_writer {
                let _ = stdin_writer.join();
            }
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            panic!("child did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if let Some(stdin_writer) = stdin_writer {
        stdin_writer
            .join()
            .expect("stdin writer should finish")
            .expect("stdin write");
    }
    Output {
        status,
        stdout: stdout_reader.join().expect("stdout reader should finish"),
        stderr: stderr_reader.join().expect("stderr reader should finish"),
    }
}

pub(super) fn read_to_end_in_thread(
    mut reader: impl Read + Send + 'static,
    stream: &'static str,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        reader
            .read_to_end(&mut output)
            .unwrap_or_else(|error| panic!("{stream} should be readable: {error}"));
        output
    })
}

#[test]
fn wait_with_output_drains_large_captured_streams() {
    if std::env::var_os("WATERSHED_FLOW_CLI_LARGE_OUTPUT_CHILD").is_some() {
        let output = vec![b'x'; 512 * 1024];
        std::io::stdout()
            .write_all(&output)
            .expect("large stdout should write");
        std::io::stderr()
            .write_all(&output)
            .expect("large stderr should write");
        return;
    }
    let test_name = crate::test_support::current_test_name();
    let child = Command::new(std::env::current_exe().expect("test executable should resolve"))
        .args(["--exact", &test_name, "--nocapture"])
        .env("WATERSHED_FLOW_CLI_LARGE_OUTPUT_CHILD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("large-output child should spawn");

    let output = wait_with_output_before(child, Duration::from_secs(10));

    assert!(output.status.success());
    assert!(output.stdout.len() >= 512 * 1024);
    assert!(output.stderr.len() >= 512 * 1024);
}

#[test]
fn input_child_is_stopped_at_the_watchdog_deadline() {
    if std::env::var_os("WATERSHED_FLOW_CLI_HANGING_INPUT_CHILD").is_some() {
        std::thread::sleep(Duration::from_secs(3));
        return;
    }
    let test_name = crate::test_support::current_test_name();
    let child = Command::new(std::env::current_exe().expect("test executable should resolve"))
        .args(["--exact", &test_name, "--nocapture"])
        .env("WATERSHED_FLOW_CLI_HANGING_INPUT_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hanging input child should spawn");
    let started = Instant::now();
    let input = vec![b'x'; 512 * 1024];

    let result = std::panic::catch_unwind(|| {
        wait_with_input_and_output_before(child, &input, Duration::from_millis(100));
    });

    assert!(result.is_err(), "the watchdog must fail a hanging child");
    assert!(started.elapsed() < Duration::from_secs(2));
}
