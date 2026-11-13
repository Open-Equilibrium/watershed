use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::{
    AgentCommand, AgentEvent, Answer, CommandError, Delivery, EntryKind, History, ItemDetail,
    ItemId, ItemKind, ItemStatus, Model, NotificationLevel, Pending, Queue, RequestId, Session,
    SessionEvent, SessionState, SessionTree, TreeNode, TurnId, UserInput,
};

const WAIT: Duration = Duration::from_secs(10);

/// The extension path the fake agents are started with.
const EXTENSION: &str = "/w/meta-harness/pi-extension/watershed.ts";

/// The command pi lists when it loaded Watershed's extension from [`EXTENSION`].
const GATE: &str = r#"{"name":"watershed-branch","source":"extension","sourceInfo":{"path":"/w/meta-harness/pi-extension/watershed.ts","source":"cli"}}"#;

/// Starts a scripted stand-in for `pi --mode rpc`; `sh` takes pi's arguments as `$0` and `$@`.
fn fake_agent(script: &str) -> (Session, Receiver<SessionEvent>) {
    let command = AgentCommand {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        extension: EXTENSION.into(),
        session: None,
        cwd: std::env::temp_dir(),
    };
    Session::start(&command).expect("fake agent starts")
}

fn run<F: Future>(test: F) -> F::Output {
    crate::checkout::isolate_user_home();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime starts")
        .block_on(test)
}

/// A file the agent creates once it did everything a test waits for.
fn marker(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("meta-harness-{name}-{}", std::process::id()))
}

/// Waits until the agent created `marker`, then removes it.
async fn reached(marker: &std::path::Path) {
    let created = async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(WAIT, created).await.expect("the agent got there");
    let _ = std::fs::remove_file(marker);
}

/// Waits until the agent accepted a command that was sent.
async fn accepted(sent: Result<Pending, CommandError>) -> Result<(), CommandError> {
    sent?.accepted().await
}

async fn next(events: &mut Receiver<SessionEvent>) -> SessionEvent {
    timeout(WAIT, events.recv())
        .await
        .expect("event arrives in time")
        .expect("session reports an exit before closing its events")
}

#[test]
fn prompts_the_agent_and_streams_its_turn() {
    run(async {
        let (mut session, mut events) = fake_agent(&format!(
            r#"{}IFS= read -r line
case "$line" in *'"type":"prompt"'*) ;; *) exit 9 ;; esac
case "$line" in *'"message":"Say hi"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{{"id":"2","type":"response","command":"prompt","success":true,"data":{{"disposition":"started"}}}}'
printf '%s\n' '{{"type":"agent_start"}}' '{{"type":"message_end","message":{{"role":"user","content":"Say hi"}}}}' '{{"type":"agent_settled","aborted":false}}'
IFS= read -r line || exit 0"#,
            gate(1)
        ));
        assert_eq!(accepted(session.prompt("Say hi").await).await, Ok(()));
        let turn = TurnId(1);
        let (item, kind) = (ItemId(1), ItemKind::UserMessage);
        for expected in [
            AgentEvent::TurnStarted { turn },
            AgentEvent::ItemStarted {
                turn,
                item,
                kind,
                detail: ItemDetail::None,
            },
            AgentEvent::ItemCompleted {
                turn,
                item,
                kind,
                text: "Say hi".to_owned(),
                status: ItemStatus::Completed,
                duration_ms: None,
                lines: None,
            },
            AgentEvent::TurnCompleted {
                turn,
                checkpoint: None,
            },
        ] {
            assert_eq!(next(&mut events).await, SessionEvent::Agent(expected));
        }
        assert!(
            timeout(WAIT, session.shutdown(WAIT))
                .await
                .expect("agent exits")
        );
        assert_eq!(next(&mut events).await, exited(true));
    });
}

#[test]
fn reports_rejected_commands_and_crashes() {
    run(async {
        let (mut session, mut events) = fake_agent(&format!(
            r#"{}IFS= read -r line
printf '%s\n' '{{"id":"2","type":"response","command":"prompt","success":false,"error":"Agent is busy"}}'
exit 3"#,
            gate(1)
        ));
        assert_eq!(
            accepted(session.prompt("Say hi").await).await,
            Err(CommandError::Rejected("Agent is busy".to_owned()))
        );
        assert_eq!(next(&mut events).await, exited(false));
        assert_eq!(
            accepted(session.abort().await).await,
            Err(CommandError::Closed)
        );
    });
}

#[test]
fn stops_an_agent_that_breaks_the_protocol() {
    run(async {
        let (_session, mut events) = fake_agent(
            r#"printf '%s\n' 'not json'
while :; do sleep 1; done"#,
        );
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Failed("agent wrote a record that is not a JSON object".to_owned())
        );
        assert_eq!(next(&mut events).await, exited(false));
    });
}

#[test]
fn a_protocol_failure_stops_the_process_before_releasing_its_checkout_guard() {
    run(async {
        let project = super::checkpoint::Repo::new("protocol-failure-guard");
        let script = r#"printf '%s\n' "$$" > .git/agent-pid
until [ -e .git/release ]; do sleep 0.02; done
i=0
while [ "$i" -lt 256 ]; do
    printf '%s\n' '{"type":"queue_update","steering":[],"followUp":[]}'
    i=$((i+1))
done
printf '%s\n' 'not json'
exec sleep 30"#;
        let (session, mut events) = Session::start(&AgentCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            extension: EXTENSION.into(),
            session: None,
            cwd: project.0.clone(),
        })
        .expect("fake agent starts");
        session
            .checkpoints()
            .capture(&super::checkpoint::at("t1", crate::Phase::Start))
            .await
            .expect("the turn holds its checkout guard");
        let checkout = crate::checkout::Checkout::new(
            project.0.join(".git/watershed"),
            project.0.join(".git/watershed"),
        );
        assert!(matches!(
            checkout.alone().await,
            Err(crate::DiffError::AgentRunning(_))
        ));
        std::fs::write(project.0.join(".git/release"), "").expect("release the agent");
        let exclusive = timeout(WAIT, async {
            loop {
                if let Ok(guard) = checkout.alone().await {
                    break guard;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the failed process releases the checkout");
        let pid = std::fs::read_to_string(project.0.join(".git/agent-pid"))
            .expect("the process recorded its id");
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        drop(exclusive);
        drop(session);
        while !matches!(next(&mut events).await, SessionEvent::Exited { .. }) {}
        assert!(!alive, "the live process lost its checkout guard");
    });
}

#[test]
fn shutdown_kills_an_agent_that_ignores_it() {
    run(async {
        let (session, _events) = fake_agent(r#"trap '' TERM; while :; do sleep 1; done"#);
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn shutdown_ends_an_agent_whose_events_nobody_reads() {
    run(async {
        // Overfills the 256-event buffer while nobody reads; the agent stays alive until
        // shutdown, independently of the protocol-failure teardown tested above.
        let written = marker("unread");
        let (session, _events) = fake_agent(&format!(
            r#"i=0
while [ $i -lt 257 ]; do printf '%s\n' '{{"type":"queue_update","steering":[],"followUp":[]}}'; i=$((i+1)); done
trap '' TERM; : > {}; while :; do sleep 1; done"#,
            super::shell_word(written.display())
        ));
        reached(&written).await;
        // Time for the core to read it all and wait for room in the full buffer.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn reads_history_incrementally_and_rereads_when_the_cursor_is_gone() {
    run(async {
        let entry = |id: &str, parent: &str| {
            format!(
                r#"{{"type":"message","id":"{id}","parentId":{parent},"message":{{"role":"user","content":"{id}"}}}}"#
            )
        };
        let respond = |id: u32, entries: String, leaf: &str| {
            format!(
                "printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[{entries}],\"leafId\":\"{leaf}\"}}}}'\n"
            )
        };
        let without_cursor = "case \"$line\" in *'\"since\"'*) exit 9 ;; esac\n";
        let script = [
            no_session_file(1),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#]),
            without_cursor.to_owned(),
            respond(2, entry("e1", "null"), "e1"),
            no_session_file(3),
            no_session_file(4),
            expect_response(&[r#""id":"5""#, r#""since":"e1""#]),
            respond(5, entry("e2", r#""e1""#), "e2"),
            no_session_file(6),
            no_session_file(7),
            expect_response(&[r#""id":"8""#, r#""since":"e2""#]),
            r#"printf '%s\n' '{"id":"8","type":"response","command":"get_entries","success":false,"error":"unknown entry"}'
"#
            .to_owned(),
            expect_response(&[r#""type":"get_entries""#, r#""id":"9""#]),
            without_cursor.to_owned(),
            respond(9, entry("x1", "null"), "x1"),
            no_session_file(10),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        let mut ids = Vec::new();
        for _ in 0..3 {
            let history = session.history().await.expect("history");
            let tree = history.tree();
            ids.push((
                tree.nodes
                    .into_iter()
                    .map(|node| node.id)
                    .collect::<Vec<_>>(),
                tree.leaf,
            ));
        }
        assert_eq!(
            ids,
            [
                (vec!["e1".to_owned()], Some("e1".to_owned())),
                (
                    vec!["e1".to_owned(), "e2".to_owned()],
                    Some("e2".to_owned())
                ),
                (vec!["x1".to_owned()], Some("x1".to_owned())),
            ],
            "only new entries are read; an unknown cursor reads everything again"
        );
    });
}

#[test]
fn reads_the_history_anew_after_a_session_switch() {
    run(async {
        let entries = |id: u32, entries: &str| {
            format!(
                "printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[{entries}],\"leafId\":\"e1\"}}}}'\n"
            )
        };
        let e1 = r#"{"type":"message","id":"e1","parentId":null,"message":{"role":"user","content":"Hi"}}"#;
        let script = [
            no_session_file(1),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#]),
            entries(2, e1),
            no_session_file(3),
            expect_response(&[r#""type":"new_session""#, r#""id":"4""#]),
            r#"printf '%s\n' '{"id":"4","type":"response","command":"new_session","success":true,"data":{"cancelled":false}}'
"#
            .to_owned(),
            // The new session shares the entry id, yet its history is read whole again.
            no_session_file(5),
            expect_response(&[r#""type":"get_entries""#, r#""id":"6""#]),
            "case \"$line\" in *'\"since\"'*) exit 9 ;; esac\n".to_owned(),
            entries(6, e1),
            no_session_file(7),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        assert_eq!(
            session
                .history()
                .await
                .map(|history| history.tree().nodes.len()),
            Ok(1)
        );
        assert_eq!(accepted(session.new_session().await).await, Ok(()));
        assert_eq!(
            session
                .history()
                .await
                .map(|history| history.tree().nodes.len()),
            Ok(1)
        );
    });
}

/// A stand-in agent whose session turns from A into B once the request matching `flip` is
/// answered. A holds `e1` alone; B holds `x1`, its own `e1` and `e2`, so B shares `e1`'s id with A
/// and a read continuing after `e1` would mix the two sessions.
fn agent_switching_to_b(flip: &str) -> String {
    const SCRIPT: &str = r#"state=A
a1='{"type":"message","id":"e1","parentId":null,"message":{"role":"user","content":"A1","usage":{"input":1}}}'
x1='{"type":"message","id":"x1","parentId":null,"message":{"role":"user","content":"B0","usage":{"input":10}}}'
b1='{"type":"message","id":"e1","parentId":"x1","message":{"role":"user","content":"B1","usage":{"input":100}}}'
b2='{"type":"message","id":"e2","parentId":"e1","message":{"role":"user","content":"B2","usage":{"input":1000}}}'
while IFS= read -r line; do
  id=${line#*'"id":"'}
  id=${id%%'"'*}
  case "$line" in
    *'"type":"get_state"'*)
      printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"%s","isStreaming":false}}\n' "$id" "$state" ;;
    *'"since"'*)
      printf '{"id":"%s","type":"response","command":"get_entries","success":true,"data":{"entries":[%s],"leafId":"e2"}}\n' "$id" "$b2" ;;
    *'"type":"get_entries"'*)
      if [ "$state" = A ]; then
        printf '{"id":"%s","type":"response","command":"get_entries","success":true,"data":{"entries":[%s],"leafId":"e1"}}\n' "$id" "$a1"
      else
        printf '{"id":"%s","type":"response","command":"get_entries","success":true,"data":{"entries":[%s,%s,%s],"leafId":"e2"}}\n' "$id" "$x1" "$b1" "$b2"
      fi ;;
    *) exit 9 ;;
  esac
  case "$line" in
    FLIP) state=B ;;
  esac
done
"#;
    SCRIPT.replace("FLIP", flip)
}

/// Asserts that a history holds session B of [`agent_switching_to_b`] and nothing of A.
fn assert_session_b(history: &History) {
    let tree = history.tree();
    assert_eq!(
        tree.nodes
            .iter()
            .map(|node| (node.id.as_str(), node.parent.as_deref()))
            .collect::<Vec<_>>(),
        [("x1", None), ("e1", Some("x1")), ("e2", Some("e1"))]
    );
    assert_eq!(tree.leaf.as_deref(), Some("e2"));
    assert_eq!(history.usage().input, 1110);
}

#[test]
fn a_history_read_never_mixes_two_sessions() {
    run(async {
        // The switch lands while the read continues after e1, so the continuation is B's.
        let (mut session, _events) = fake_agent(&agent_switching_to_b(r#"*'"since"'*"#));
        session.history().await.expect("history");
        assert_session_b(session.history().await.expect("history"));
    });
}

#[test]
fn a_switch_between_history_reads_makes_the_next_read_start_over() {
    run(async {
        // The switch lands after the first read's closing state read, before the next read.
        let (mut session, _events) = fake_agent(&agent_switching_to_b(r#"*'"id":"3"'*"#));
        session.history().await.expect("history");
        assert_session_b(session.history().await.expect("history"));
    });
}

#[test]
fn a_history_read_gives_up_while_the_session_keeps_changing() {
    run(async {
        // Every state read names the other session, so no read sees the same one before and after.
        let (mut session, _events) = fake_agent(
            r#"n=0
while IFS= read -r line; do
  id=${line#*'"id":"'}
  id=${id%%'"'*}
  case "$line" in
    *'"type":"get_state"'*)
      n=$((n + 1))
      printf '{"id":"%s","type":"response","command":"get_state","success":true,"data":{"sessionId":"s%s","isStreaming":false}}\n' "$id" "$((n % 2))" ;;
    *'"type":"get_entries"'*)
      printf '{"id":"%s","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":null}}\n' "$id" ;;
    *) exit 9 ;;
  esac
done"#,
        );
        assert_eq!(
            session.history().await.map(drop),
            Err(CommandError::SessionChanged)
        );
    });
}

#[test]
fn reopens_a_session_larger_than_one_record_from_its_file() {
    run(async {
        // 2,000 entries of 9 KB: more than the 16 MiB record limit together, small each.
        let file =
            std::env::temp_dir().join(format!("watershed-large-{}.jsonl", std::process::id()));
        let text = "x".repeat(9000);
        let mut stored = String::from(
            r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-10-25T10:00:00.000Z","cwd":"/tmp"}"#,
        );
        stored.push('\n');
        let mut parent = "null".to_owned();
        for number in 0..2000 {
            stored.push_str(&format!(
                r#"{{"type":"message","id":"e{number}","parentId":{parent},"message":{{"role":"user","content":"{text}"}}}}"#
            ));
            stored.push('\n');
            parent = format!(r#""e{number}""#);
        }
        // pi is still writing the next entry; its answer holds it whole.
        stored.push_str(r#"{"type":"message","id":"e2000","parentId":"e19"#);
        std::fs::write(&file, stored).expect("session file");
        let script = [
            expect_response(&[r#""type":"get_state""#, r#""id":"1""#]),
            session_file(1, &file),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#, r#""since":"e1999""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"e2000","parentId":"e1999","message":{"role":"user","content":"last"}}],"leafId":"e2000"}}'
"#
            .to_owned(),
            expect_response(&[r#""type":"get_state""#, r#""id":"3""#]),
            session_file(3, &file),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        let history = session.history().await;
        let _ = std::fs::remove_file(&file);
        let tree = history.expect("history").tree();
        assert_eq!(
            (tree.nodes.len(), tree.leaf.as_deref()),
            (2001, Some("e2000")),
            "every stored entry once, and the one pi was still writing"
        );
    });
}

#[test]
fn rereads_the_session_file_when_new_entries_exceed_one_record() {
    run(async {
        let file =
            std::env::temp_dir().join(format!("watershed-grown-{}.jsonl", std::process::id()));
        let header = r#"{"type":"session","version":3,"id":"s1","cwd":"/tmp"}"#;
        let entry = |id: &str, parent: &str| {
            format!(
                r#"{{"type":"message","id":"{id}","parentId":{parent},"message":{{"role":"user","content":"{id}"}}}}"#
            )
        };
        std::fs::write(&file, format!("{header}\n{}\n", entry("e1", "null"))).expect("file");
        let huge = "head -c 17000000 /dev/zero | tr '\\0' a\n";
        let script = [
            expect_response(&[r#""type":"get_state""#, r#""id":"1""#]),
            session_file(1, &file),
            expect_response(&[r#""id":"2""#, r#""since":"e1""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":"e1"}}'
"#
            .to_owned(),
            expect_response(&[r#""type":"get_state""#, r#""id":"3""#]),
            session_file(3, &file),
            // A long run added more than one record holds before the next read.
            expect_response(&[r#""type":"get_state""#, r#""id":"4""#]),
            session_file(4, &file),
            expect_response(&[r#""id":"5""#, r#""since":"e1""#]),
            format!(
                r#"printf '%s' '{{"id":"5","type":"response","command":"get_entries","success":true,"data":"'
{huge}printf '"}}\n'
"#
            ),
            // The session file holds the entries since the last one read.
            expect_response(&[r#""id":"6""#, r#""since":"e2""#]),
            r#"printf '%s\n' '{"id":"6","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":"e2"}}'
"#
            .to_owned(),
            expect_response(&[r#""type":"get_state""#, r#""id":"7""#]),
            session_file(7, &file),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        assert_eq!(
            session
                .history()
                .await
                .map(|history| history.tree().nodes.len()),
            Ok(1)
        );
        std::fs::write(
            &file,
            format!(
                "{header}\n{}\n{}\n",
                entry("e1", "null"),
                entry("e2", r#""e1""#)
            ),
        )
        .expect("file grows");
        let grown = session.history().await;
        let _ = std::fs::remove_file(&file);
        let tree = grown.expect("history").tree();
        assert_eq!((tree.nodes.len(), tree.leaf.as_deref()), (2, Some("e2")));
    });
}

#[test]
fn shutdown_ends_an_agent_that_stopped_reading() {
    run(async {
        let (mut session, _events) = fake_agent("trap '' TERM; while :; do sleep 1; done");
        // Fills the pipe and the queue of records waiting to be written.
        let model = "x".repeat(100_000);
        for _ in 0..200 {
            let sent = timeout(
                Duration::from_millis(200),
                session.set_model("faux", &model),
            )
            .await;
            if sent.is_err() {
                break;
            }
        }
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn shutdown_ends_an_agent_that_stopped_reading_its_automatic_answers() {
    run(async {
        // Unreadable checkpoint requests are answered at once; the agent never reads the answers.
        let (session, _events) = fake_agent(&format!(
            "trap '' TERM\ni=0\nwhile [ $i -lt 3000 ]; do {}i=$((i+1)); done\nwhile :; do sleep 1; done",
            unreadable_checkpoint("'$i'")
        ));
        // The core stops reading once the agent's full input blocks its answers, so the agent
        // never finishes writing; a marker after its loop could not appear.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn cancels_a_dialog_too_large_to_read() {
    run(async {
        let huge = "head -c 17000000 /dev/zero | tr '\\0' a\n";
        let script = [
            format!(
                r#"printf '%s' '{{"type":"extension_ui_request","id":"q9","method":"editor","title":"Edit","prefill":"'
{huge}printf '"}}\n'
"#
            ),
            expect_response(&[r#""id":"q9""#, r#""cancelled":true"#]),
            r#"printf '%s\n' '{"type":"queue_update","steering":[],"followUp":[]}'
IFS= read -r line || exit 0
"#
            .to_owned(),
        ]
        .concat();
        let (_session, mut events) = fake_agent(&script);
        let skipped = next(&mut events).await;
        assert!(
            matches!(
                &skipped,
                SessionEvent::Agent(AgentEvent::Notification {
                    level: NotificationLevel::Warning,
                    ..
                })
            ),
            "{skipped:?}"
        );
        assert!(
            matches!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::QueueChanged(_))
            ),
            "the agent got its answer instead of waiting"
        );
    });
}

#[test]
fn skips_an_oversized_record_and_keeps_the_session() {
    run(async {
        // More than the 16 MiB record limit, without holding it in the script.
        let huge = "head -c 17000000 /dev/zero | tr '\\0' a\n";
        let script = [
            format!(
                r#"printf '%s' '{{"type":"message_update","assistantMessageEvent":{{"type":"text_delta","delta":"'
{huge}printf '"}}}}\n'
"#
            ),
            no_session_file(1),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#]),
            format!(
                r#"printf '%s' '{{"id":"2","type":"response","command":"get_entries","success":true,"data":"'
{huge}printf '"}}\n'
"#
            ),
            expect_response(&[r#""type":"get_state""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"get_state","success":true,"data":{"sessionId":"abc","isStreaming":false,"sessionFile":"/tmp/s.jsonl"}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let skipped = next(&mut events).await;
        assert!(
            matches!(
                &skipped,
                SessionEvent::Agent(AgentEvent::Notification {
                    level: NotificationLevel::Warning,
                    message,
                }) if message.starts_with("Skipped an agent record")
            ),
            "{skipped:?}"
        );
        assert_eq!(
            session.history().await.map(drop),
            Err(CommandError::TooLarge)
        );
        assert_eq!(
            session.state().await.map(|state| state.session_file),
            Ok(Some("/tmp/s.jsonl".into()))
        );
    });
}

#[test]
fn delivers_messages_as_pi_decides_and_clears_the_queue() {
    run(async {
        let script = [
            gate(1),
            expect_response(&[
                r#""type":"prompt""#,
                r#""message":"Use Redis""#,
                r#""streamingBehavior":"steer""#,
            ]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"prompt","success":true,"data":{"disposition":"queued"}}'
"#
            .to_owned(),
            gate(3),
            expect_response(&[r#""message":"Also docs""#, r#""streamingBehavior":"followUp""#]),
            r#"printf '%s\n' '{"id":"4","type":"response","command":"prompt","success":true,"data":{"disposition":"queued"}}'
printf '%s\n' '{"type":"agent_start"}'
printf '%s\n' '{"type":"queue_update","steering":["Use Redis"],"followUp":["Also docs"]}'
"#
            .to_owned(),
            expect_response(&[r#""type":"clear_queue""#, r#""id":"5""#]),
            r#"printf '%s\n' '{"id":"5","type":"response","command":"clear_queue","success":true,"data":{"steering":["Use Redis"],"followUp":["Also docs"]}}'
IFS= read -r line || exit 0
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let queue = Queue {
            steering: vec!["Use Redis".to_owned()],
            follow_up: vec!["Also docs".to_owned()],
        };
        assert!(!session.running());
        assert_eq!(
            accepted(session.send("Use Redis", Delivery::Steer).await).await,
            Ok(())
        );
        assert_eq!(
            accepted(session.send("Also docs", Delivery::FollowUp).await).await,
            Ok(())
        );
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::TurnStarted { .. })
        ));
        assert!(session.running(), "a started turn makes the session run");
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::QueueChanged(queue.clone()))
        );
        assert_eq!(session.clear_queue().await, Ok(queue));
    });
}

/// Shell lines that read one response and fail unless it contains every given fragment.
fn expect_response(fragments: &[&str]) -> String {
    let mut script = String::from("IFS= read -r line\n");
    for fragment in fragments {
        script.push_str(&format!(
            "case \"$line\" in *'{fragment}'*) ;; *) exit 9 ;; esac\n"
        ));
    }
    script
}

/// Answers the check before a message that may start a run: Watershed's extension is loaded.
fn gate(id: u32) -> String {
    format!(
        "{}printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_commands\",\"success\":true,\"data\":{{\"commands\":[{GATE}]}}}}'\n",
        expect_response(&[r#""type":"get_commands""#, &format!(r#""id":"{id}""#)])
    )
}

/// Answers the `get_state` that starts a first history read, for a session pi keeps in memory.
fn no_session_file(id: u32) -> String {
    format!(
        "{}printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s\",\"isStreaming\":false}}}}'\n",
        expect_response(&[r#""type":"get_state""#, &format!(r#""id":"{id}""#)])
    )
}

/// Answers `get_state` with request `id`: the session is stored in `file`.
fn session_file(id: u32, file: &std::path::Path) -> String {
    let answer = serde_json::json!({
        "id": id.to_string(),
        "type": "response",
        "command": "get_state",
        "success": true,
        "data": { "sessionId": "s1", "isStreaming": false, "sessionFile": file },
    });
    format!("printf '%s\\n' {}\n", super::shell_word(answer))
}

/// A question an extension asks the user.
fn question(id: &str) -> String {
    format!(
        "printf '%s\\n' '{{\"type\":\"extension_ui_request\",\"id\":\"{id}\",\"method\":\"select\",\"title\":\"Continue?\",\"options\":[\"Yes\",\"No\"]}}'\n"
    )
}

/// A checkpoint request the core answers at once with an error, as its payload is unreadable.
fn unreadable_checkpoint(id: &str) -> String {
    format!(
        "printf '%s\\n' '{{\"type\":\"extension_ui_request\",\"id\":\"{id}\",\"method\":\"input\",\"title\":\"watershed.checkpoint x\"}}'\n"
    )
}

/// A throwaway Git project with one commit, for an agent that works in it.
fn git_project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("meta-harness-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("project");
    std::fs::write(dir.join("a.txt"), "a\n").expect("file");
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "base",
        ],
    ] {
        let status = super::fixture_git()
            .args(args)
            .current_dir(&dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?}");
    }
    dir
}

#[test]
fn checkpoint_start_distinguishes_missing_guards_from_failed_snapshots() {
    run(async {
        use super::checkpoint::{Repo, at};
        use crate::{BranchTarget, Changes, Checkpoints, DiffError, Phase};

        for failure in ["opening", "registration", "snapshot", "outside"] {
            let repo = Repo::new(&format!("start-guard-{failure}"));
            repo.git(&["branch", "other"]);
            let independent = Checkpoints::new(repo.path());
            independent
                .capture(&at("base", Phase::Start))
                .await
                .expect("base");
            independent
                .capture(&at("base", Phase::End))
                .await
                .expect("end");
            let config = repo.path().join(".git/config");
            let saved = std::fs::read(&config).expect("fixture config");
            match failure {
                "opening" => std::fs::write(&config, b"[broken\n").expect("break config"),
                "registration" => {
                    let lock = repo.path().join(".git/watershed/checkout.lock");
                    std::fs::remove_file(&lock).expect("remove fixture lock");
                    std::fs::create_dir(&lock).expect("break lock");
                }
                "snapshot" => {
                    repo.git(&["config", "filter.fail.clean", "false"]);
                    repo.git(&["config", "filter.fail.required", "true"]);
                    repo.write(".gitattributes", "*.txt filter=fail\n");
                    repo.write("a.txt", "changed\n");
                }
                "outside" => {
                    std::fs::rename(repo.path().join(".git"), repo.path().join("saved-git"))
                        .expect("hide Git")
                }
                _ => unreachable!(),
            }
            let response = repo.path().join("answer");
            let script = format!(
                r#"printf '%s\n' '{{"type":"extension_ui_request","id":"c1","method":"input","title":"watershed.checkpoint {{\"session\":\"s1\",\"turn\":\"t1\",\"phase\":\"start\"}}"}}'
IFS= read -r line
printf '%s' "$line" > {written}
mv {written} {response}
IFS= read -r line
"#,
                written = super::shell_word(format!("{}.tmp", response.display())),
                response = super::shell_word(response.display())
            );
            let command = AgentCommand {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                extension: EXTENSION.into(),
                session: None,
                cwd: repo.path().to_owned(),
            };
            let (session, _events) = Session::start(&command).expect("session");
            timeout(WAIT, async {
                while !response.exists() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("checkpoint answer");
            let answer: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&response).expect("response")).expect("JSON");
            let outcome: serde_json::Value =
                serde_json::from_str(answer["value"].as_str().expect("answer value"))
                    .expect("outcome");
            // Repair while pi is still awaiting the run, before any settled event.
            if failure == "outside" {
                std::fs::rename(repo.path().join("saved-git"), repo.path().join(".git"))
                    .expect("restore Git");
            } else {
                std::fs::write(&config, &saved).expect("repair config");
            }
            if failure == "registration" {
                std::fs::remove_dir(repo.path().join(".git/watershed/checkout.lock"))
                    .expect("repair lock");
            }
            let can_run = failure == "snapshot" || failure == "outside";
            assert_eq!(outcome["canRun"], can_run, "{failure}: {outcome}");
            if failure == "snapshot" {
                for result in [
                    independent
                        .switch_branch(&BranchTarget::Local("other".to_owned()), Changes::Bring)
                        .await,
                    independent
                        .revert(&at("base", Phase::Start), &Default::default())
                        .await,
                    independent.restore_stash("missing").await,
                    independent.discard_stash("missing").await,
                ] {
                    assert!(
                        matches!(result, Err(DiffError::AgentRunning(_))),
                        "{result:?}"
                    );
                }
            }
            session.shutdown(Duration::from_secs(1)).await;
        }
    });
}

#[test]
fn takes_the_checkpoints_the_extension_asks_for_without_asking_the_user() {
    run(async {
        let project = git_project("checkpoints");
        let request = |id: &str, turn: &str, phase: &str| {
            format!(
                r#"printf '%s\n' '{{"type":"extension_ui_request","id":"{id}","method":"input","title":"watershed.checkpoint {{\"session\":\"s1\",\"turn\":\"{turn}\",\"phase\":\"{phase}\"}}","placeholder":""}}'
"#
            )
        };
        let script = [
            request("c1", "t1", "start"),
            expect_response(&[r#""id":"c1""#, r#"\"ok\":true"#]),
            // The turn changed one file between its checkpoints.
            "printf 'b\\n' > a.txt\n".to_owned(),
            request("c2", "t1", "end"),
            expect_response(&[
                r#""id":"c2""#,
                r#"\"ok\":true"#,
                r#"\"files\":1"#,
                r#"\"added\":1"#,
                r#"\"removed\":1"#,
            ]),
            // An id that could leave Watershed's refs is refused, and pi learns why.
            request("c3", "../x", "start"),
            expect_response(&[r#""id":"c3""#, "cannot name a checkpoint"]),
        ]
        .concat();
        let command = AgentCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            extension: EXTENSION.into(),
            session: None,
            cwd: project.clone(),
        };
        let (_session, mut events) = Session::start(&command).expect("fake agent starts");
        assert_eq!(
            next(&mut events).await,
            exited(true),
            "no question reached the user"
        );
        let refs = super::fixture_git()
            .args(["for-each-ref", "--format=%(refname)", "refs/watershed"])
            .current_dir(&project)
            .output()
            .expect("git runs");
        assert_eq!(
            String::from_utf8_lossy(&refs.stdout),
            "refs/watershed/checkpoints/s1/t1/end\nrefs/watershed/checkpoints/s1/t1/start\n"
        );
        let _ = std::fs::remove_dir_all(&project);
    });
}

#[test]
fn a_turn_that_waits_for_a_revert_says_so_in_the_status() {
    run(async {
        let project = git_project("checkpoint-waits");
        // Another window reverts the checkout, or deletes a session or commits; the turn cannot
        // tell which.
        let reverting = crate::checkout::Checkout::new(
            project.join(".git/watershed"),
            project.join(".git/watershed"),
        )
        .alone()
        .await
        .expect("nothing runs");
        let script = [
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"c1","method":"input","title":"watershed.checkpoint {\"session\":\"s1\",\"turn\":\"t1\",\"phase\":\"start\"}","placeholder":""}'
"#
            .to_owned(),
            expect_response(&[r#""id":"c1""#, r#"\"ok\":true"#]),
        ]
        .concat();
        let command = AgentCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            extension: EXTENSION.into(),
            session: None,
            cwd: project.clone(),
        };
        let (_session, mut events) = Session::start(&command).expect("fake agent starts");
        let status = |text: Option<&str>| {
            SessionEvent::Agent(AgentEvent::StatusChanged {
                key: "watershed.revert".to_owned(),
                text: text.map(str::to_owned),
            })
        };
        assert_eq!(
            next(&mut events).await,
            status(Some(
                "Waiting for another Git action in this checkout to finish"
            ))
        );
        drop(reverting);
        assert_eq!(next(&mut events).await, status(None));
        assert_eq!(next(&mut events).await, exited(true));
        let _ = std::fs::remove_dir_all(&project);
    });
}

#[test]
fn an_agent_that_exits_stops_the_checkpoint_it_asked_for() {
    run(async {
        let project = git_project("checkpoint-exit");
        // The project's clean filter says it runs, then holds the capture far longer than the
        // test; Git runs it at the top of the work tree.
        let config = project.join(".git/config");
        let mut text = std::fs::read_to_string(&config).expect("config");
        text.push_str(
            "[filter \"slow\"]\n\tclean = \"echo $$ > .git/filter-pid; sleep 30; cat\"\n",
        );
        std::fs::write(&config, text).expect("config");
        std::fs::write(project.join(".gitattributes"), "*.txt filter=slow\n").expect("attributes");
        std::fs::write(project.join("a.txt"), "changed\n").expect("file");
        let script = r#"printf '%s\n' '{"type":"extension_ui_request","id":"c1","method":"input","title":"watershed.checkpoint {\"session\":\"s1\",\"turn\":\"t1\",\"phase\":\"start\"}","placeholder":""}'
until [ -e .git/filter-pid ]; do sleep 0.05; done"#;
        let command = AgentCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            extension: EXTENSION.into(),
            session: None,
            cwd: project.clone(),
        };
        let (_session, mut events) = Session::start(&command).expect("fake agent starts");
        assert_eq!(next(&mut events).await, exited(true));
        let pid = std::fs::read_to_string(project.join(".git/filter-pid")).expect("the filter ran");
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if alive {
            let _ = std::process::Command::new("kill")
                .args(["-9", pid.trim()])
                .status();
        }
        assert!(!alive, "the capture's Git outlived the agent");
        let _ = std::fs::remove_dir_all(&project);
    });
}

#[test]
fn an_agent_that_exits_stops_its_checkpoint_while_a_descendant_holds_its_output() {
    run(async {
        let project = git_project("checkpoint-held");
        let config = project.join(".git/config");
        let mut text = std::fs::read_to_string(&config).expect("config");
        text.push_str(
            "[filter \"slow\"]\n\tclean = \"echo $$ > .git/filter-pid; sleep 30; cat\"\n",
        );
        std::fs::write(&config, text).expect("config");
        std::fs::write(project.join(".gitattributes"), "*.txt filter=slow\n").expect("attributes");
        std::fs::write(project.join("a.txt"), "changed\n").expect("file");
        // A program the agent left behind keeps its output open after the agent exits.
        let script = r#"sleep 30 & echo $! > .git/holder-pid
printf '%s\n' '{"type":"extension_ui_request","id":"c1","method":"input","title":"watershed.checkpoint {\"session\":\"s1\",\"turn\":\"t1\",\"phase\":\"start\"}","placeholder":""}'
until [ -e .git/filter-pid ]; do sleep 0.05; done"#;
        let command = AgentCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            extension: EXTENSION.into(),
            session: None,
            cwd: project.clone(),
        };
        let (_session, _events) = Session::start(&command).expect("fake agent starts");
        let filter = project.join(".git/filter-pid");
        timeout(WAIT, async {
            while !filter.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the filter runs");
        let alive = |pid: &str| {
            std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        let pid = std::fs::read_to_string(&filter).expect("the filter's id");
        let started = std::time::Instant::now();
        while alive(&pid) && started.elapsed() < Duration::from_secs(3) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let survived = alive(&pid);
        for program in [
            pid.clone(),
            std::fs::read_to_string(project.join(".git/holder-pid")).unwrap_or_default(),
        ] {
            let _ = std::process::Command::new("kill")
                .args(["-9", program.trim()])
                .status();
        }
        assert!(!survived, "the capture's Git outlived the agent");
        let _ = std::fs::remove_dir_all(&project);
    });
}

#[test]
fn an_agent_that_exits_stops_its_checkpoint_while_its_events_wait() {
    run(async {
        let project = git_project("checkpoint-backpressure");
        let config = project.join(".git/config");
        let mut text = std::fs::read_to_string(&config).expect("config");
        text.push_str(
            "[filter \"slow\"]\n\tclean = \"echo $$ > .git/filter-pid; sleep 30; cat\"\n",
        );
        std::fs::write(&config, text).expect("config");
        std::fs::write(project.join(".gitattributes"), "*.txt filter=slow\n").expect("attributes");
        std::fs::write(project.join("a.txt"), "changed\n").expect("file");
        // More events than the session buffers, which nobody reads, then the agent exits.
        let script = r#"printf '%s\n' '{"type":"extension_ui_request","id":"c1","method":"input","title":"watershed.checkpoint {\"session\":\"s1\",\"turn\":\"t1\",\"phase\":\"start\"}","placeholder":""}'
until [ -e .git/filter-pid ]; do sleep 0.05; done
i=0; while [ $i -lt 300 ]; do printf '%s\n' '{"type":"queue_update","steering":[],"followUp":[]}'; i=$((i+1)); done"#;
        let command = AgentCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            extension: EXTENSION.into(),
            session: None,
            cwd: project.clone(),
        };
        let (_session, _events) = Session::start(&command).expect("fake agent starts");
        let filter = project.join(".git/filter-pid");
        timeout(WAIT, async {
            while !filter.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the filter runs");
        let pid = std::fs::read_to_string(&filter).expect("the filter's id");
        let alive = || {
            std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        let started = std::time::Instant::now();
        while alive() && started.elapsed() < Duration::from_secs(3) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let survived = alive();
        let _ = std::process::Command::new("kill")
            .args(["-9", pid.trim()])
            .status();
        assert!(!survived, "the capture's Git outlived the agent");
        let _ = std::fs::remove_dir_all(&project);
    });
}

#[test]
fn answers_extension_questions() {
    run(async {
        let script = [
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"select","title":"Pick","options":["A","B"]}'
"#
            .to_owned(),
            expect_response(&[r#""type":"extension_ui_response""#, r#""id":"q1""#, r#""value":"B""#]),
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q2","method":"confirm","title":"Sure?","message":"Really"}'
"#
            .to_owned(),
            expect_response(&[r#""id":"q2""#, r#""confirmed":true"#]),
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q3","method":"input","title":"Name","placeholder":""}'
"#
            .to_owned(),
            expect_response(&[r#""id":"q3""#, r#""cancelled":true"#]),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let questions = [
            ("q1", Answer::Value("B".to_owned())),
            ("q2", Answer::Confirmed(true)),
            ("q3", Answer::Cancelled),
        ];
        for (id, answer) in questions {
            let request = RequestId(id.to_owned());
            let SessionEvent::Agent(AgentEvent::UserInputRequested {
                request: asked,
                input,
            }) = next(&mut events).await
            else {
                panic!("question {id} arrives");
            };
            assert_eq!(asked, request);
            if id == "q1" {
                assert_eq!(
                    input,
                    UserInput::Select {
                        title: "Pick".to_owned(),
                        options: vec!["A".to_owned(), "B".to_owned()],
                    }
                );
            }
            assert_eq!(session.answer(&request, answer).await, Ok(()));
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::UserInputResolved { request })
            );
        }
        assert_eq!(next(&mut events).await, exited(true));
    });
}

#[test]
fn a_cancelled_answer_still_reports_its_resolution() {
    run(async {
        let filled = marker("filled");
        let answered = marker("answered");
        let script = [
            question("q1"),
            // More events than the buffer holds, so the resolution waits for room and the answer
            // is given up there.
            r#"i=0; while [ $i -lt 300 ]; do printf '%s\n' '{"type":"extension_ui_request","id":"n","method":"notify","message":"x"}'; i=$((i+1)); done
"#
            .to_owned(),
            format!("touch {}\n", super::shell_word(filled.display())),
            expect_response(&[r#""id":"q1""#, r#""cancelled":true"#]),
            format!("touch {}\n", super::shell_word(answered.display())),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let SessionEvent::Agent(AgentEvent::UserInputRequested { request, .. }) =
            next(&mut events).await
        else {
            panic!("the question arrives");
        };
        // The session reads the printed events into the buffer after they were written.
        reached(&filled).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let given_up = timeout(
            Duration::from_millis(500),
            session.answer(&request, Answer::Cancelled),
        )
        .await;
        assert!(
            given_up.is_err(),
            "the answer waits for room in the full buffer"
        );
        reached(&answered).await;
        let mut resolved = false;
        while let Ok(Some(event)) = timeout(Duration::from_millis(500), events.recv()).await {
            resolved |= event
                == SessionEvent::Agent(AgentEvent::UserInputResolved {
                    request: request.clone(),
                });
        }
        assert!(
            resolved,
            "the agent got the answer, so the resolution is reported"
        );
        let _ = session.shutdown(Duration::from_millis(100)).await;
    });
}

#[test]
fn stopping_dismisses_a_question_that_arrives_late() {
    run(async {
        // The agent asked before the abort reached it; the question only arrives afterwards.
        let script = [
            expect_response(&[r#""type":"abort""#]),
            question("q1"),
            expect_response(&[r#""id":"q1""#, r#""cancelled":true"#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"abort","success":true}' '{"type":"agent_start"}'
"#
            .to_owned(),
            question("q2"),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let stopping = session.abort().await.expect("abort sent");
        assert_eq!(timeout(WAIT, stopping.accepted()).await, Ok(Ok(())));
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::TurnStarted { .. })
        ));
        assert!(
            matches!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::UserInputRequested { request, .. })
                    if request == RequestId("q2".to_owned())
            ),
            "the next turn asks again"
        );
    });
}

#[test]
fn an_abort_dismisses_questions_after_a_late_turn_start() {
    run(async {
        // The turn's start reaches the core only after the abort was sent, so its question is
        // still dismissed: pi answers the abort only once that question is answered.
        let script = [
            expect_response(&[r#""type":"abort""#]),
            r#"printf '%s\n' '{"type":"agent_start"}'
"#
            .to_owned(),
            question("q1"),
            expect_response(&[r#""id":"q1""#, r#""cancelled":true"#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"abort","success":true}'
"#
            .to_owned(),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let stopping = session.abort().await.expect("abort sent");
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::TurnStarted { .. })
        ));
        assert_eq!(
            timeout(WAIT, stopping.accepted()).await,
            Ok(Ok(())),
            "the abort finishes once the late question is dismissed"
        );
        assert!(
            events.try_recv().is_err(),
            "the late question was dismissed, not asked"
        );
    });
}

#[test]
fn asks_the_user_again_once_an_abort_found_the_agent_idle() {
    run(async {
        // pi answers the abort of an idle agent without any turn event.
        let script = [
            expect_response(&[r#""type":"abort""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"abort","success":true}' '{"type":"extension_ui_request","id":"q1","method":"select","title":"Start over?","options":["Yes","No"]}'
"#
            .to_owned(),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let stopping = session.abort().await.expect("abort sent");
        assert_eq!(timeout(WAIT, stopping.accepted()).await, Ok(Ok(())));
        assert!(
            matches!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::UserInputRequested { .. })
            ),
            "the question reaches the user"
        );
    });
}

#[test]
fn switches_sessions_and_models_through_pi() {
    run(async {
        let script = [
            expect_response(&[r#""type":"new_session""#, r#""id":"1""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"new_session","success":true,"data":{"cancelled":false}}'
"#
            .to_owned(),
            expect_response(&[
                r#""type":"switch_session""#,
                r#""id":"2""#,
                r#""sessionPath":"/tmp/old.jsonl""#,
            ]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"switch_session","success":true,"data":{"cancelled":true}}'
"#
            .to_owned(),
            expect_response(&[r#""type":"get_available_models""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"get_available_models","success":true,"data":{"models":[{"id":"faux-1","name":"Faux One","provider":"faux","contextWindow":200000},{"id":"faux-2","provider":"faux","contextWindow":0}]}}'
"#
            .to_owned(),
            expect_response(&[
                r#""type":"set_model""#,
                r#""id":"4""#,
                r#""provider":"faux""#,
                r#""modelId":"faux-2""#,
            ]),
            r#"printf '%s\n' '{"id":"4","type":"response","command":"set_model","success":true,"data":{}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        let started = session.new_session().await.expect("sent");
        assert_eq!(started.switched().await, Ok(true));
        let resumed = session
            .switch_session(std::path::Path::new("/tmp/old.jsonl"))
            .await
            .expect("sent");
        assert_eq!(
            resumed.switched().await,
            Ok(false),
            "an extension cancelled it"
        );
        let model = |id: &str, name: &str, context_window| Model {
            provider: "faux".to_owned(),
            id: id.to_owned(),
            name: name.to_owned(),
            context_window,
        };
        assert_eq!(
            session.models().await,
            Ok(vec![
                model("faux-1", "Faux One", Some(200_000)),
                model("faux-2", "faux-2", None)
            ]),
            "a context window of 0 leaves it unknown"
        );
        assert_eq!(
            accepted(session.set_model("faux", "faux-2").await).await,
            Ok(())
        );
    });
}

#[test]
fn forgets_a_question_once_its_time_is_up() {
    run(async {
        // pi answers a dialog with a timeout itself once the time is up, without telling.
        let (mut session, mut events) = fake_agent(
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Sure?","message":"Really","timeout":50}'
IFS= read -r line || exit 0"#,
        );
        let request = RequestId("q1".to_owned());
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::UserInputRequested { .. })
        ));
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::UserInputResolved {
                request: request.clone()
            })
        );
        assert_eq!(
            session.answer(&request, Answer::Confirmed(true)).await,
            Err(CommandError::UnknownRequest)
        );
    });
}

#[test]
fn keeps_a_dialog_without_a_time_limit_open() {
    run(async {
        // pi installs no timer for a zero timeout and waits for the answer.
        let script = [
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Sure?","message":"Really","timeout":0}'
"#
            .to_owned(),
            expect_response(&[r#""id":"q1""#, r#""confirmed":true"#]),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::UserInputRequested { .. })
        ));
        let early = timeout(Duration::from_millis(300), events.recv()).await;
        assert!(early.is_err(), "nothing expires: {early:?}");
        let request = RequestId("q1".to_owned());
        assert_eq!(
            session.answer(&request, Answer::Confirmed(true)).await,
            Ok(())
        );
    });
}

#[test]
fn reports_nothing_after_the_agent_exited() {
    run(async {
        let (_session, mut events) = fake_agent(
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Sure?","message":"Really","timeout":300}'
exit 3"#,
        );
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::UserInputRequested { .. })
        ));
        assert_eq!(next(&mut events).await, exited(false));
        let later = timeout(Duration::from_millis(800), events.recv()).await;
        assert!(later.is_err(), "the dialog died with the agent: {later:?}");
    });
}

#[test]
fn reads_state_and_tree_and_branches_through_the_extension() {
    run(async {
        let script = [
            expect_response(&[r#""type":"get_state""#, r#""id":"1""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"get_state","success":true,"data":{"model":{"id":"faux-1","provider":"faux"},"isStreaming":false,"sessionFile":"/tmp/s.jsonl","sessionId":"abc"}}'
"#
            .to_owned(),
            no_session_file(2),
            expect_response(&[r#""type":"get_entries""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"e1","parentId":null,"message":{"role":"user","content":"hi"}}],"leafId":"e1"}}'
"#
            .to_owned(),
            no_session_file(4),
            gate(5),
            expect_response(&[r#""type":"prompt""#, r#""id":"6""#, r#""message":"/watershed-branch e1 B""#]),
            r#"printf '%s\n' '{"id":"6","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        assert_eq!(
            session.state().await,
            Ok(SessionState {
                session_file: Some("/tmp/s.jsonl".into()),
                model: Some("faux/faux-1".to_owned()),
                session_id: Some("abc".to_owned()),
            })
        );
        let history = session.history().await.expect("history");
        assert_eq!(
            history.tree(),
            SessionTree {
                nodes: vec![TreeNode {
                    id: "e1".to_owned(),
                    parent: None,
                    kind: EntryKind::User,
                }],
                leaf: Some("e1".to_owned()),
            }
        );
        assert_eq!(
            accepted(session.branch("e1", Some("B")).await).await,
            Ok(())
        );
    });
}

#[test]
fn requires_the_extension_that_marks_checkpoints() {
    run(async {
        let answer = |names: &str| {
            format!(
                "{}printf '%s\\n' '{{\"id\":\"1\",\"type\":\"response\",\"command\":\"get_commands\",\"success\":true,\"data\":{{\"commands\":[{names}]}}}}'\nIFS= read -r line || exit 0\n",
                expect_response(&[r#""type":"get_commands""#])
            )
        };
        let (mut gated, _events) = fake_agent(&answer(GATE));
        assert_eq!(gated.require_extension().await, Ok(()));

        // Another extension loads fine, but nothing would mark the turn's checkpoints.
        let other = r#"{"name":"fix-tests","source":"prompt"},{"name":"watershed-branch","source":"prompt"}"#;
        let (mut ungated, _events) = fake_agent(&answer(other));
        assert_eq!(
            ungated.require_extension().await,
            Err(CommandError::ExtensionMissing)
        );

        // An extension from another file registers the same command, for example a project's
        // own copy that marks no checkpoints.
        let copy = r#"{"name":"watershed-branch","source":"extension","sourceInfo":{"path":"/p/.pi/extensions/watershed.ts","source":"local"}}"#;
        let (mut copied, _events) = fake_agent(&answer(copy));
        assert_eq!(
            copied.require_extension().await,
            Err(CommandError::ExtensionMissing)
        );
    });
}

#[test]
fn starts_pi_in_rpc_mode_with_watersheds_extension_and_the_stored_session() {
    run(async {
        let reopen = Some("/s/session.jsonl");
        let stored = " --session /s/session.jsonl";
        for (file, args) in [(None, ""), (reopen, stored)] {
            let script = format!(
                "[ \"$0 $*\" = '--mode rpc -e {EXTENSION}{args}' ] || exit 9\n{}printf '%s\\n' '{{\"id\":\"1\",\"type\":\"response\",\"command\":\"get_commands\",\"success\":true,\"data\":{{\"commands\":[{GATE}]}}}}'\nIFS= read -r line || exit 0\n",
                expect_response(&[r#""type":"get_commands""#])
            );
            let command = AgentCommand {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                extension: EXTENSION.into(),
                session: file.map(Into::into),
                cwd: std::env::temp_dir(),
            };
            let (mut session, _events) = Session::start(&command).expect("fake agent starts");
            assert_eq!(session.require_extension().await, Ok(()), "{args:?}");
        }
    });
}

#[test]
fn names_the_extension_as_pi_reports_it_when_its_path_has_dot_dot() {
    run(async {
        // pi resolves `..`, `.` and repeated slashes in the path it lists, as Liquid passes it on.
        let script = format!(
            "[ \"$0 $*\" = '--mode rpc -e {EXTENSION}' ] || exit 9\n{}IFS= read -r line || exit 0\n",
            gate(1)
        );
        let command = AgentCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            extension: "/w/./liquid/..//meta-harness/pi-extension/watershed.ts".into(),
            session: None,
            cwd: std::env::temp_dir(),
        };
        let (mut session, _events) = Session::start(&command).expect("fake agent starts");
        assert_eq!(session.require_extension().await, Ok(()));
    });
}

#[test]
fn refuses_an_extension_path_relative_to_the_agents_directory() {
    run(async {
        // pi would read the file inside the project, which could supply its own copy.
        let started = marker("relative-extension");
        let command = AgentCommand {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!("touch {}", super::shell_word(started.display())).into(),
            ],
            extension: "pi-extension/watershed.ts".into(),
            session: None,
            cwd: std::env::temp_dir(),
        };
        let Err(error) = Session::start(&command) else {
            panic!("the session is refused");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("absolute"), "{error}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!started.exists(), "the agent never starts");
    });
}

#[test]
fn checks_the_extension_before_every_message_that_starts_a_run() {
    run(async {
        let commands = |id: u32, names: &str| {
            format!(
                "{}printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_commands\",\"success\":true,\"data\":{{\"commands\":[{names}]}}}}'\n",
                expect_response(&[r#""type":"get_commands""#, &format!(r#""id":"{id}""#)])
            )
        };
        let prompt = |id: u32| {
            format!(
                "{}printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"prompt\",\"success\":true,\"data\":{{\"disposition\":\"started\"}}}}'\n",
                expect_response(&[r#""type":"prompt""#, &format!(r#""id":"{id}""#)])
            )
        };
        let script = [
            commands(1, GATE),
            prompt(2),
            // An extension command switched sessions unseen, and pi loaded the new session's
            // extensions without Watershed's.
            commands(3, ""),
            expect_response(&[r#""type":"new_session""#, r#""id":"4""#]),
            r#"printf '%s\n' '{"id":"4","type":"response","command":"new_session","success":true,"data":{"cancelled":false}}'
"#
            .to_owned(),
            commands(5, GATE),
            prompt(6),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        assert_eq!(accepted(session.prompt("Hi").await).await, Ok(()));
        assert_eq!(
            session.send("Again", Delivery::Steer).await.map(drop),
            Err(CommandError::ExtensionMissing),
            "the message never reaches an agent that would run without checkpoints"
        );
        assert_eq!(
            session.new_session().await.expect("sent").switched().await,
            Ok(true)
        );
        assert_eq!(accepted(session.prompt("Hi").await).await, Ok(()));
    });
}

/// The last event of an agent that wrote nothing to its error output.
fn exited(success: bool) -> SessionEvent {
    SessionEvent::Exited {
        success,
        error_output: String::new(),
    }
}

#[test]
fn an_exited_agents_stdout_drain_is_bounded_and_keeps_complete_records() {
    run(async {
        let dir = marker("stdout-drain");
        std::fs::create_dir_all(&dir).expect("temporary directory");
        let script = format!(
            r#"IFS= read -r line
printf '%s\n' '{{"type":"extension_ui_request","method":"notify","message":"before exit","notifyType":"info"}}'
/bin/sh -c '
sleep 0.1
printf "%s\n" '\''{{"type":"extension_ui_request","method":"notify","message":"after exit","notifyType":"info"}}'\''
printf "%s" '\''{{"type":"unfinished"'\''
exec sleep 30
' 2>/dev/null &
printf '%s\n' "$!" > {}/holder
exit 0"#,
            super::shell_word(dir.display())
        );
        let (mut session, mut events) = fake_agent(&script);
        let pending = session.abort().await.expect("command sent");
        let result = timeout(Duration::from_secs(5), async {
            for message in [
                "before exit",
                "after exit",
                "Some agent output may be missing.",
            ] {
                assert_eq!(
                    next(&mut events).await,
                    SessionEvent::Agent(AgentEvent::Notification {
                        level: if message.starts_with("Some") {
                            NotificationLevel::Warning
                        } else {
                            NotificationLevel::Info
                        },
                        message: message.to_owned(),
                    })
                );
            }
            assert_eq!(pending.accepted().await, Err(CommandError::Closed));
            assert_eq!(next(&mut events).await, exited(true));
        })
        .await;
        let pid = std::fs::read_to_string(dir.join("holder")).expect("holder pid");
        let _ = std::process::Command::new("kill").arg(pid.trim()).status();
        std::fs::remove_dir_all(&dir).expect("remove temporary directory");
        result.expect("reply and exit arrive within the drain bound");
    });
}

#[test]
fn an_exited_agents_continuously_writing_descendant_cannot_extend_the_drain() {
    run(async {
        let dir = marker("stdout-streaming");
        std::fs::create_dir_all(&dir).expect("temporary directory");
        let (mut session, mut events) = fake_agent(&format!(
            r#"IFS= read -r line
/bin/sh -c 'i=0
while [ "$i" -lt 1000 ]; do
    printf "%s\n" '\''{{"type":"queue_update","steering":[],"followUp":[]}}'\''
    i=$((i+1))
    sleep 0.01
done' 2>/dev/null &
printf '%s\n' "$!" > {}/holder"#,
            super::shell_word(dir.display())
        ));
        let pending = session.abort().await.expect("command sent");
        let result = timeout(Duration::from_secs(5), async {
            let mut records = 0;
            loop {
                match next(&mut events).await {
                    SessionEvent::Agent(AgentEvent::QueueChanged(_)) => records += 1,
                    warning => {
                        assert_eq!(
                            warning,
                            SessionEvent::Agent(AgentEvent::Notification {
                                level: NotificationLevel::Warning,
                                message: "Some agent output may be missing.".to_owned(),
                            })
                        );
                        break;
                    }
                }
            }
            assert!(records > 0, "complete descendant records were delivered");
            assert_eq!(pending.accepted().await, Err(CommandError::Closed));
            assert_eq!(next(&mut events).await, exited(true));
        })
        .await;
        let pid = std::fs::read_to_string(dir.join("holder")).expect("holder pid");
        let _ = std::process::Command::new("kill").arg(pid.trim()).status();
        std::fs::remove_dir_all(&dir).expect("remove temporary directory");
        result.expect("continuous output cannot extend the drain deadline");
    });
}

#[test]
fn an_exited_agents_stdout_drain_stops_at_the_aggregate_byte_cap() {
    post_exit_output(32 * 1024 * 1024);
}

#[test]
fn an_exited_agents_stdout_drain_preserves_the_byte_cap_boundary() {
    for bytes in [16 * 1024 * 1024 - 1, 16 * 1024 * 1024, 16 * 1024 * 1024 + 1] {
        post_exit_output(bytes);
    }
}

fn post_exit_output(bytes: usize) {
    run(async {
        use std::io::Write;

        const CAP: usize = 16 * 1024 * 1024;
        const RECORD: usize = 64 * 1024;
        let dir = marker(&format!("stdout-cap-{bytes}"));
        std::fs::create_dir_all(&dir).expect("temporary directory");
        let mut payload = std::io::BufWriter::new(
            std::fs::File::create(dir.join("payload")).expect("output fixture"),
        );
        // Stretch the last record by one byte for the boundary cases.
        let count = (bytes + RECORD / 2) / RECORD;
        for i in 0..count {
            let length = if i + 1 == count {
                bytes - i * RECORD
            } else {
                RECORD
            };
            let mut record = format!(
                r#"{{"type":"extension_ui_request","method":"notify","message":"{i}","notifyType":"info"}}"#
            ).into_bytes();
            record.resize(length - 1, b' ');
            record.push(b'\n');
            payload.write_all(&record).expect("fixture record");
        }
        payload.flush().expect("flush output fixture");
        drop(payload);
        let script = format!(
            r#"IFS= read -r line
/bin/sh -c '
while kill -0 "$1" 2>/dev/null; do sleep 0.01; done
: > "$2/ready"
until [ -e "$2/release" ]; do sleep 0.01; done
if cat "$2/payload"; then : > "$2/written"; else : > "$2/closed"; fi
: > "$2/done"
' holder "$$" {} 2>/dev/null &"#,
            super::shell_word(dir.display())
        );
        let (mut session, mut events) = fake_agent(&script);
        let pending = session.abort().await.expect("command sent");
        reached(&dir.join("ready")).await;
        std::fs::write(dir.join("release"), "").expect("release post-exit writer");
        // No events are consumed until collection has stopped and released the writer.
        reached(&dir.join("done")).await;
        if bytes > CAP + RECORD {
            assert!(
                dir.join("closed").exists(),
                "collection must close the pipe at the cap"
            );
        }
        let complete = if bytes > CAP && bytes < CAP + RECORD {
            CAP / RECORD - 1
        } else {
            count.min(CAP / RECORD)
        };
        for i in 0..complete {
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::Notification {
                    level: NotificationLevel::Info,
                    message: i.to_string(),
                }),
                "record {i}, {bytes} bytes"
            );
        }
        if bytes >= CAP {
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::Notification {
                    level: NotificationLevel::Warning,
                    message: "Some agent output may be missing.".to_owned(),
                })
            );
        }
        assert_eq!(pending.accepted().await, Err(CommandError::Closed));
        assert_eq!(next(&mut events).await, exited(true));
        drop(session);
        assert!(
            events.recv().await.is_none(),
            "no duplicate warning or later records"
        );
        std::fs::remove_dir_all(&dir).expect("remove temporary directory");
    });
}

#[test]
fn an_exited_agents_unread_stdin_cannot_block_collected_output_or_completion() {
    run(async {
        let dir = marker("stdin-after-exit");
        std::fs::create_dir_all(&dir).expect("temporary directory");
        // Large automatic answers fill both the inherited input pipe and its bounded queue.
        let descendant = format!(
            "i=0\nwhile [ $i -lt 512 ]; do {}i=$((i+1)); done\nprintf '%s\\n' '{{\"type\":\"extension_ui_request\",\"method\":\"notify\",\"message\":\"collected\",\"notifyType\":\"info\"}}'\nexec sleep 30",
            unreadable_checkpoint(&"x".repeat(4096))
        );
        let (mut session, mut events) = fake_agent(&format!(
            "IFS= read -r line\nexec 3<&0\n/bin/sh -c {} <&3 2>/dev/null &\nprintf '%s\\n' \"$!\" > {}/holder",
            super::shell_word(descendant),
            super::shell_word(dir.display())
        ));
        let pending = session.abort().await.expect("command sent");
        let result = timeout(Duration::from_secs(5), async {
            for (level, message) in [
                (NotificationLevel::Info, "collected"),
                (
                    NotificationLevel::Warning,
                    "Some agent output may be missing.",
                ),
            ] {
                assert_eq!(
                    next(&mut events).await,
                    SessionEvent::Agent(AgentEvent::Notification {
                        level,
                        message: message.to_owned(),
                    })
                );
            }
            assert_eq!(pending.accepted().await, Err(CommandError::Closed));
            assert_eq!(next(&mut events).await, exited(true));
        })
        .await;
        let pid = std::fs::read_to_string(dir.join("holder")).expect("holder pid");
        let _ = std::process::Command::new("kill").arg(pid.trim()).status();
        std::fs::remove_dir_all(&dir).expect("remove temporary directory");
        result.expect("unread inherited stdin cannot hold completion open");
    });
}

#[test]
fn an_exited_agents_buffered_stdout_reaches_eof_without_a_notice() {
    run(async {
        let written = marker("stdout-buffered");
        let (mut session, mut events) = fake_agent(&format!(
            r#"IFS= read -r line
i=0
while [ "$i" -lt 600 ]; do
    printf '%s\n' "{{\"type\":\"extension_ui_request\",\"method\":\"notify\",\"message\":\"$i\",\"notifyType\":\"info\"}}"
    i=$((i+1))
done
printf '%s\n' '{{"id":"1","type":"response","command":"abort","success":true}}'
: > {}"#,
            super::shell_word(written.display())
        ));
        let pending = session.abort().await.expect("command sent");
        reached(&written).await;
        // Hold event delivery past the drain grace: records already read still all arrive.
        tokio::time::sleep(Duration::from_secs(3)).await;
        for i in 0..600 {
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::Notification {
                    level: NotificationLevel::Info,
                    message: i.to_string(),
                })
            );
        }
        assert_eq!(pending.accepted().await, Ok(()));
        assert_eq!(next(&mut events).await, exited(true));
    });
}

#[test]
fn shutdown_keeps_the_output_of_an_agent_that_exited_while_delivery_waits() {
    run(async {
        let written = marker("shutdown-buffered");
        // More events than the session buffers, which nobody reads yet; it exits once asked.
        let (session, mut events) = fake_agent(&format!(
            r#"i=0
while [ "$i" -lt 300 ]; do
    printf '%s\n' "{{\"type\":\"extension_ui_request\",\"method\":\"notify\",\"message\":\"$i\",\"notifyType\":\"info\"}}"
    i=$((i+1))
done
: > {}
IFS= read -r line || exit 0"#,
            super::shell_word(written.display())
        ));
        reached(&written).await;
        // Time for the core to read it all and wait for room in the full buffer.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(300))).await;

        assert_eq!(stopped, Ok(true));
        for i in 0..300 {
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Agent(AgentEvent::Notification {
                    level: NotificationLevel::Info,
                    message: i.to_string(),
                })
            );
        }
        assert_eq!(next(&mut events).await, exited(true));
    });
}

#[test]
fn an_exited_agents_stderr_reader_closes_while_a_descendant_holds_the_pipe() {
    run(async {
        let dir = marker("stderr-reader");
        std::fs::create_dir_all(&dir).expect("temporary directory");
        let script = format!(
            r#"printf 'retained tail\n' >&2
/bin/sh -c 'trap "" PIPE
count=0
until [ -e "$1/release" ]; do
    [ "$count" -lt 500 ] || exit 1
    count=$((count + 1))
    sleep 0.02
done
if printf probe >&3 2>/dev/null; then result=open; else result=closed; fi
: > "$1/$result"
' holder {} 3>&2 >/dev/null 2>/dev/null &"#,
            super::shell_word(dir.display())
        );
        let (_session, mut events) = fake_agent(&script);
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited {
                success: true,
                error_output: "retained tail".to_owned(),
            }
        );
        std::fs::write(dir.join("release"), "").expect("release the descendant");
        timeout(WAIT, async {
            while !dir.join("open").exists() && !dir.join("closed").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the descendant probed stderr");
        let closed = dir.join("closed").exists();
        std::fs::remove_dir_all(&dir).expect("remove temporary directory");
        assert!(closed, "the stderr reader outlived the reported agent exit");
    });
}

#[test]
fn reports_the_end_of_what_a_failed_agent_wrote_to_its_error_output() {
    run(async {
        // More than a pipe holds and than is kept, with the reason at the end.
        let (_session, mut events) = fake_agent(
            r#"i=0; while [ $i -lt 2000 ]; do printf 'warning %s: something to ignore here\n' $i >&2; i=$((i+1)); done
printf 'Error: Failed to load extension\n' >&2
exit 1"#,
        );
        match next(&mut events).await {
            SessionEvent::Exited {
                success: false,
                error_output,
            } => {
                assert!(
                    error_output.ends_with("Error: Failed to load extension"),
                    "{error_output}"
                );
                assert!(error_output.len() <= 16 * 1024, "{}", error_output.len());
            }
            other => panic!("{other:?}"),
        }
    });
}

#[test]
fn finds_a_relative_session_file_in_the_agents_directory() {
    run(async {
        // A project's pi settings can name a relative session directory.
        let (mut session, _events) = fake_agent(&format!(
            "{}printf '%s\\n' '{{\"id\":\"1\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s\",\"isStreaming\":false,\"sessionFile\":\".sessions/s.jsonl\"}}}}'\nIFS= read -r line || exit 0\n",
            expect_response(&[r#""type":"get_state""#])
        ));
        assert_eq!(
            session.state().await.expect("state").session_file,
            Some(std::env::temp_dir().join(".sessions/s.jsonl"))
        );
    });
}

#[test]
fn stopping_dismisses_the_questions_the_agent_waits_for() {
    run(async {
        // pi answers an abort only once its open dialogs are answered: a question asked before
        // the abort, and one that arrives after it.
        let script = [
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"input","title":"Name","placeholder":""}'
"#
            .to_owned(),
            expect_response(&[r#""type":"abort""#]),
            expect_response(&[r#""id":"q1""#, r#""cancelled":true"#]),
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q2","method":"confirm","title":"Sure?","message":""}'
"#
            .to_owned(),
            expect_response(&[r#""id":"q2""#, r#""cancelled":true"#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"abort","success":true}'
"#
            .to_owned(),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::UserInputRequested { .. })
        ));
        let stopping = session.abort().await.expect("abort sent");
        assert_eq!(timeout(WAIT, stopping.accepted()).await, Ok(Ok(())));
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::UserInputResolved {
                request: RequestId("q1".to_owned())
            })
        );
    });
}

#[test]
fn reads_the_history_before_pi_wrote_the_session_file() {
    run(async {
        // pi writes the session file only after the first response, so a fresh session has a
        // path that does not exist yet.
        let missing =
            std::env::temp_dir().join(format!("watershed-unwritten-{}.jsonl", std::process::id()));
        let script = format!(
            "{}{}{}printf '%s\\n' '{{\"id\":\"2\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[{{\"type\":\"message\",\"id\":\"u1\",\"parentId\":null,\"message\":{{\"role\":\"user\",\"content\":\"Hi\"}}}}],\"leafId\":\"u1\"}}}}'\n{}{}IFS= read -r line || exit 0\n",
            expect_response(&[r#""type":"get_state""#]),
            session_file(1, &missing),
            expect_response(&[r#""type":"get_entries""#]),
            expect_response(&[r#""type":"get_state""#, r#""id":"3""#]),
            session_file(3, &missing),
        );
        let (mut session, _events) = fake_agent(&script);
        let history = session.history().await.expect("history");
        assert_eq!(history.tree().leaf.as_deref(), Some("u1"));
    });
}

#[test]
fn a_question_s_time_limit_keeps_nothing_alive_after_the_exit() {
    run(async {
        let (session, mut events) = fake_agent(
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Sure?","message":"","timeout":600000}'
exit 0"#,
        );
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Agent(AgentEvent::UserInputRequested { .. })
        ));
        assert_eq!(next(&mut events).await, exited(true));
        drop(session);
        assert_eq!(
            timeout(WAIT, events.recv()).await,
            Ok(None),
            "the event stream ends instead of waiting ten minutes"
        );
    });
}

#[test]
fn kept_checkpoints_keep_no_event_stream_open_after_the_exit() {
    run(async {
        let (session, mut events) = fake_agent("exit 0");
        let checkpoints = session.checkpoints();
        assert_eq!(next(&mut events).await, exited(true));
        drop(session);
        assert_eq!(
            timeout(WAIT, events.recv()).await,
            Ok(None),
            "the event stream ends while the checkpoints are still held"
        );
        drop(checkpoints);
    });
}
