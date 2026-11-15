use serde_json::{Value, json};

use super::checkpoint::{Repo, run, turn};
use crate::{Checkpoints, session_record};

/// Two turns of session `s1`: T1 changes `a.txt` and completes, T2 adds `b.txt` and was aborted.
const ENTRIES: &str = r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-11-01T10:00:00.000Z","cwd":"/project"}
{"type":"custom","id":"c1","parentId":null,"timestamp":"2026-11-01T10:00:01.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"start"}}
{"type":"message","id":"u1","parentId":"c1","timestamp":"2026-11-01T10:00:01.100Z","message":{"role":"user","content":"Rewrite a.txt\nplease"}}
{"type":"message","id":"a1","parentId":"u1","timestamp":"2026-11-01T10:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Done."}],"stopReason":"stop"}}
{"type":"custom","id":"c2","parentId":"a1","timestamp":"2026-11-01T10:00:03.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"end","files":1,"added":1,"removed":1}}
{"type":"custom","id":"c3","parentId":"c2","timestamp":"2026-11-01T10:01:00.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T2","phase":"start"}}
{"type":"message","id":"u2","parentId":"c3","timestamp":"2026-11-01T10:01:00.100Z","message":{"role":"user","content":"Add b.txt"}}
{"type":"message","id":"a2","parentId":"u2","timestamp":"2026-11-01T10:01:02.000Z","message":{"role":"assistant","content":[],"stopReason":"aborted"}}
{"type":"custom","id":"c4","parentId":"a2","timestamp":"2026-11-01T10:01:03.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T2","phase":"end","files":1,"added":1,"removed":0}}
"#;

#[test]
fn a_record_lists_each_turn_with_its_checkpoints_and_changed_files() {
    let repo = Repo::new("record");
    let checkpoints = Checkpoints::new(repo.path());
    run(async {
        turn(&checkpoints, "T1", || repo.write("a.txt", "new\n")).await;
        turn(&checkpoints, "T2", || repo.write("b.txt", "x\n")).await;
    });
    let file = std::env::temp_dir().join(format!("watershed-record-{}.jsonl", std::process::id()));
    std::fs::write(&file, ENTRIES).expect("session file");
    let record = run(session_record(
        &checkpoints,
        &file,
        repo.path(),
        "test-host",
        "s1",
        "Rewrite a.txt",
    ));
    let _ = std::fs::remove_file(&file);
    let record: Value = serde_json::from_str(&record.expect("record")).expect("record JSON");
    let commit = |reference: &str| repo.git(&["rev-parse", reference]).trim().to_owned();
    let branch = repo
        .git(&["symbolic-ref", "--short", "HEAD"])
        .trim()
        .to_owned();
    assert_eq!(
        record,
        json!({
            "schema": "watershed-session-record-v1",
            "host": "test-host",
            "harness": "pi",
            "project": repo.path().display().to_string(),
            "branch": branch,
            "session": { "id": "s1", "title": "Rewrite a.txt" },
            "turns": [
                {
                    "turn": "T1",
                    "started_ms": 1_793_527_201_000u64,
                    "ended_ms": 1_793_527_203_000u64,
                    "outcome": "completed",
                    "error": null,
                    "stop_reason": "stop",
                    "recorded": { "files": 1, "added": 1, "removed": 1 },
                    "start_commit": commit("refs/watershed/checkpoints/s1/T1/start"),
                    "end_commit": commit("refs/watershed/checkpoints/s1/T1/end"),
                    "files": [
                        { "path": "a.txt", "status": "modified", "added": 1, "removed": 1 }
                    ]
                },
                {
                    "turn": "T2",
                    "started_ms": 1_793_527_260_000u64,
                    "ended_ms": 1_793_527_263_000u64,
                    "outcome": "aborted",
                    "error": null,
                    "stop_reason": "aborted",
                    "recorded": { "files": 1, "added": 1, "removed": 0 },
                    "start_commit": commit("refs/watershed/checkpoints/s1/T2/start"),
                    "end_commit": commit("refs/watershed/checkpoints/s1/T2/end"),
                    "files": [
                        { "path": "b.txt", "status": "added", "added": 1, "removed": 0 }
                    ]
                }
            ]
        })
    );
}

#[test]
fn a_turn_outside_git_without_an_end_names_the_error_pi_stored() {
    let file = std::env::temp_dir().join(format!(
        "watershed-record-error-{}.jsonl",
        std::process::id()
    ));
    std::fs::write(
        &file,
        r#"{"type":"session","version":3,"id":"s2","timestamp":"2026-11-01T10:00:00.000Z","cwd":"/project"}
{"type":"custom","id":"c1","parentId":null,"timestamp":"2026-11-01T10:00:01.000Z","customType":"watershed.checkpoint","data":{"session":"s2","turn":"T1","phase":"start"}}
{"type":"message","id":"u1","parentId":"c1","timestamp":"2026-11-01T10:00:01.100Z","message":{"role":"user","content":"Try it"}}
{"type":"message","id":"a1","parentId":"u1","timestamp":"2026-11-01T10:00:02.000Z","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"529 overloaded"}}
"#,
    )
    .expect("session file");
    let dir = std::env::temp_dir().join(format!("watershed-record-outside-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("project");
    let checkpoints = Checkpoints::new(&dir);
    let record = run(session_record(
        &checkpoints,
        &file,
        &dir,
        "test-host",
        "s2",
        "Untitled session",
    ));
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&dir);
    let record: Value = serde_json::from_str(&record.expect("record outside Git")).expect("JSON");
    assert_eq!(record["branch"], Value::Null);
    assert_eq!(
        record["turns"],
        json!([{
            "turn": "T1",
            "started_ms": 1_793_527_201_000u64,
            "ended_ms": null,
            "outcome": "error",
            "error": "529 overloaded",
            "stop_reason": "error",
            "recorded": null,
            "start_commit": null,
            "end_commit": null,
            "files": null
        }])
    );
}
