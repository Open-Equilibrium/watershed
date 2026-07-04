use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn cli_argument_helpers_cover_emit_tail_and_usage_edges() {
    assert_eq!(
        emit_mode(&args(&["run", "hello-loop"])).unwrap(),
        EmitMode::Human
    );
    assert_eq!(
        emit_mode(&args(&["run", "hello-loop", "--emit", "jsonl"])).unwrap(),
        EmitMode::Jsonl
    );
    assert!(matches!(
        emit_mode(&args(&["run", "hello-loop", "--emit", "human"])),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported emit mode")
    ));
    assert!(matches!(
        emit_mode(&args(&["run", "hello-loop", "--bad"])),
        Err(RuntimeError::Usage(message)) if message.contains("unknown argument")
    ));

    let (emit, options) = tail_args(&args(&[
        "tail",
        "meta001",
        "--emit",
        "jsonl",
        "--no-follow",
        "--timeout-ms",
        "25",
    ]))
    .expect("tail args parse");
    assert_eq!(emit, EmitMode::Jsonl);
    assert!(!options.follow);
    assert_eq!(options.timeout, Some(Duration::from_millis(25)));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--emit"])),
        Err(RuntimeError::Usage(message)) if message.contains("missing value for --emit")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--emit", "human"])),
        Err(RuntimeError::Usage(message)) if message.contains("unsupported emit mode")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--timeout-ms"])),
        Err(RuntimeError::Usage(message)) if message.contains("missing value for --timeout-ms")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--timeout-ms", "soon"])),
        Err(RuntimeError::Usage(message)) if message.contains("invalid --timeout-ms value")
    ));
    assert!(matches!(
        tail_args(&args(&["tail", "meta001", "--bad"])),
        Err(RuntimeError::Usage(message)) if message.contains("unknown argument")
    ));

    let run_args = args(&["run", "hello-loop"]);
    assert_eq!(
        positional(&run_args, 1, "loop name").expect("loop name exists"),
        "hello-loop"
    );
    assert!(matches!(
        positional(&run_args, 2, "session_id"),
        Err(RuntimeError::Usage(message)) if message.contains("missing session_id")
    ));
    assert!(reject_extra_args(&args(&["sessions"]), 1).is_ok());
    assert!(matches!(
        reject_extra_args(&args(&["sessions", "--bad"]), 1),
        Err(RuntimeError::Usage(message)) if message.contains("unknown argument")
    ));

    assert_eq!(
        os_string_to_string(OsString::from("--version")).expect("version flag converts"),
        "--version"
    );
    assert_eq!(
        os_string_to_string(OsString::from("-V")).expect("short version flag converts"),
        "-V"
    );
    assert_eq!(
        os_string_to_string(OsString::from("run")).expect("utf-8 arg converts"),
        "run"
    );
    assert!(usage().contains("loop run <loop>"));
}
