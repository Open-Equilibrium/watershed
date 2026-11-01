use serde_json::Value;

use crate::{ChangeCount, History, TurnCheckpoints};

/// Turn T1 has two prompts, as a steering message joins its run; T2 still runs; a fork from
/// before T2's prompt holds T3, whose end checkpoint failed.
const ENTRIES: &str = r#"{"entries":[
  {"type":"model_change","id":"e0","parentId":null,"timestamp":"2026-11-01T10:00:00.000Z"},
  {"type":"custom","id":"c1","parentId":"e0","timestamp":"2026-11-01T10:00:01.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"start"}},
  {"type":"message","id":"u1","parentId":"c1","timestamp":"2026-11-01T10:00:01.100Z","message":{"role":"user","content":"Add rate limiting\nto /login"}},
  {"type":"message","id":"a1","parentId":"u1","timestamp":"2026-11-01T10:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"On it."}]}},
  {"type":"message","id":"u2","parentId":"a1","timestamp":"2026-11-01T10:00:03.000Z","message":{"role":"user","content":"and tests"}},
  {"type":"custom","id":"c2","parentId":"u2","timestamp":"2026-11-01T10:00:09.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T1","phase":"end","files":3,"added":12,"removed":1}},
  {"type":"custom","id":"c3","parentId":"c2","timestamp":"2026-11-01T10:01:00.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T2","phase":"start"}},
  {"type":"message","id":"u3","parentId":"c3","timestamp":"2026-11-01T10:01:00.100Z","message":{"role":"user","content":"Return 429"}},
  {"type":"custom","id":"c4","parentId":"c2","timestamp":"2026-11-01T10:02:00.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T3","phase":"start"}},
  {"type":"message","id":"u4","parentId":"c4","timestamp":"2026-11-01T10:02:00.100Z","message":{"role":"user","content":"Use a token bucket"}},
  {"type":"custom","id":"c5","parentId":"u4","timestamp":"2026-11-01T10:02:05.000Z","customType":"watershed.checkpoint","data":{"session":"s1","turn":"T3","phase":"end","error":"git did not finish within 60 s"}}
],"leafId":"u3"}"#;

#[test]
fn a_branch_lists_its_turns_with_their_checkpoints() {
    let data: Value = serde_json::from_str(ENTRIES).expect("JSON");
    let history = History::from_entries(&data);
    let turn = |id: &str, entry: &str, number, prompt: &str, started: u64| TurnCheckpoints {
        session: "s1".to_owned(),
        turn: id.to_owned(),
        start_entry: entry.to_owned(),
        end_entry: None,
        number,
        prompt: prompt.to_owned(),
        started_ms: Some(started),
        ended_ms: None,
        ended: false,
        failure: None,
        changed: None,
    };
    assert_eq!(
        history.turns(None),
        [
            TurnCheckpoints {
                ended_ms: Some(1_793_527_209_000),
                end_entry: Some("c2".to_owned()),
                ended: true,
                changed: Some(ChangeCount {
                    files: 3,
                    added: 12,
                    removed: 1,
                }),
                ..turn("T1", "c1", 1, "Add rate limiting", 1_793_527_201_000)
            },
            // Still running: its end is not recorded yet. The steering prompt counted as turn 2.
            turn("T2", "c3", 3, "Return 429", 1_793_527_260_000),
        ]
    );
    assert_eq!(
        history.turns(Some("c5"))[1],
        TurnCheckpoints {
            ended_ms: Some(1_793_527_325_000),
            end_entry: Some("c5".to_owned()),
            failure: Some("git did not finish within 60 s".to_owned()),
            ..turn("T3", "c4", 3, "Use a token bucket", 1_793_527_320_000)
        }
    );
}
