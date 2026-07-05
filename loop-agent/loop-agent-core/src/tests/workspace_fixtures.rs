#[test]
fn workspace_copy_skips_fixture_runtime_state() {
    let fixture = fixture_dir("hello-loop");
    let stale_session = fixture.join(".loop/sessions/stale.jsonl");
    let stale_output = fixture.join("out/summary.txt");
    let _guard = FixtureRuntimeStateGuard::new([stale_session.clone(), stale_output.clone()]);
    fs::create_dir_all(stale_session.parent().expect("session path has parent"))
        .expect("stale session parent created");
    fs::write(&stale_session, "{}\n").expect("stale session created");
    fs::write(&stale_output, "stale\n").expect("stale output created");

    let workspace = workspace_copy("hello-loop");

    assert!(
        workspace.join(".loop/config.yaml").exists(),
        "workspace config must still be copied"
    );
    assert!(
        workspace.join("out").is_dir(),
        "output directory shape must still be copied"
    );
    assert!(
        !workspace.join(".loop/sessions/stale.jsonl").exists(),
        "fixture runtime session state must not be copied"
    );
    assert!(
        !workspace.join("out/summary.txt").exists(),
        "fixture output state must not be copied"
    );
}

