use rusqlite::Connection;

use crate::host_move::{self, Facts, Harness, MovePackage, MoveRecord, SCHEMA, State};
use crate::{ChatItem, ModelChoice, Permissions};

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(SCHEMA).unwrap();
    db
}

fn model(instance: &str) -> ModelChoice {
    ModelChoice {
        instance: instance.to_owned(),
        model: "m".to_owned(),
        options: Vec::new(),
    }
}

fn facts() -> Facts {
    Facts {
        title: "Fix the port test".to_owned(),
        model: Some(model("claudeAgent")),
        permissions: Permissions::Supervised,
        running: true,
        uncommitted_files: 3,
        branch: Some("fix/ports".to_owned()),
        remote_url: Some("git@example.invalid:acme/app.git".to_owned()),
        queued_attachments: false,
    }
}

#[test]
fn a_seal_and_its_pending_pause_are_recorded_together_or_not_at_all() {
    let db = db();
    host_move::begin(&db, "t1", "/repo", "tr-1", ("addr", "studio", "env"), 10).unwrap();
    host_move::hand(&db, "tr-1", "{}", 11).unwrap();
    db.execute_batch(
        "CREATE TRIGGER refuse_pause BEFORE INSERT ON pause_pending \
         BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    assert!(host_move::seal(&db, "tr-1", "studio", 12).is_err());
    assert_eq!(
        host_move::phase(&db, "tr-1").unwrap(),
        Some(host_move::Phase::Handed)
    );
    db.execute_batch("DROP TRIGGER refuse_pause").unwrap();
    host_move::seal(&db, "tr-1", "studio", 13).unwrap();
    assert_eq!(
        host_move::pauses_pending(&db).unwrap(),
        vec![("t1".to_owned(), "/repo".to_owned())]
    );
}

#[test]
fn a_pause_that_did_not_finish_stays_pending_until_it_is_done() {
    let db = db();
    host_move::begin(&db, "t1", "/repo", "tr-1", ("addr", "studio", "env"), 10).unwrap();
    host_move::hand(&db, "tr-1", "{}", 11).unwrap();
    host_move::seal(&db, "tr-1", "studio", 12).unwrap();
    host_move::seal(&db, "tr-1", "studio", 13).unwrap();
    assert_eq!(
        host_move::pauses_pending(&db).unwrap(),
        vec![("t1".to_owned(), "/repo".to_owned())]
    );
    host_move::pause_done(&db, "t1").unwrap();
    assert!(host_move::pauses_pending(&db).unwrap().is_empty());
}

#[test]
fn a_session_moves_only_when_it_is_claude_code_or_codex_and_nothing_blocks_it() {
    let blocked = |change: fn(&mut Facts), state: Option<State>| {
        let mut facts = facts();
        change(&mut facts);
        host_move::check(&facts, state).blocked
    };
    assert_eq!(host_move::check(&facts(), None).blocked, None);
    assert_eq!(
        host_move::check(&facts(), None).harness,
        Some(Harness::ClaudeCode)
    );
    assert_eq!(
        blocked(|f| f.model = Some(model("codex")), None),
        None,
        "Codex moves too"
    );
    assert_eq!(
        blocked(|f| f.model = Some(model("scripted")), None).as_deref(),
        Some("Only Claude Code and Codex sessions can move.")
    );
    assert_eq!(
        blocked(|f| f.branch = None, None).as_deref(),
        Some("This session has no branch to move.")
    );
    assert_eq!(
        blocked(|f| f.remote_url = None, None).as_deref(),
        Some(
            "This project has no Git remote that T3 can resolve. A remote given as a folder path does not count, so the session cannot move."
        )
    );
    assert_eq!(
        blocked(|_| {}, Some(State::Moving)).as_deref(),
        Some("This session is already moving to another host.")
    );
    assert_eq!(
        blocked(|_| {}, Some(State::Sealed)).as_deref(),
        Some("This session has already moved to another host.")
    );
}

#[test]
fn a_move_begins_hands_over_and_seals_or_releases_in_the_store() {
    let db = db();
    assert_eq!(host_move::state(&db, "t1").unwrap(), None);
    host_move::begin(&db, "t1", "/repo", "tr-1", ("addr", "studio", "env"), 10).unwrap();
    assert_eq!(host_move::state(&db, "t1").unwrap(), Some(State::Moving));
    assert!(host_move::sealed_threads(&db).unwrap().is_empty());
    host_move::release(&db, "tr-1").unwrap();
    assert_eq!(host_move::state(&db, "t1").unwrap(), None);

    host_move::begin(&db, "t1", "/repo", "tr-2", ("addr", "studio", "env"), 20).unwrap();
    assert!(
        host_move::seal(&db, "tr-2", "studio", 30).is_err(),
        "a move seals only once handed over"
    );
    host_move::hand(&db, "tr-2", "{}", 25).unwrap();
    host_move::seal(&db, "tr-2", "studio", 30).unwrap();
    assert_eq!(host_move::state(&db, "t1").unwrap(), Some(State::Sealed));
    assert_eq!(
        host_move::sealed_threads(&db).unwrap(),
        ["t1".to_owned()].into()
    );
    assert!(
        host_move::release(&db, "tr-2").is_err(),
        "a sealed session is not released"
    );
    assert!(host_move::seal(&db, "tr-unknown", "studio", 40).is_err());
}

#[test]
fn a_finish_acts_only_on_the_move_it_began() {
    let db = db();
    host_move::begin(
        &db,
        "t1",
        "/repo",
        "tr-first",
        ("addr", "studio", "env"),
        10,
    )
    .unwrap();
    host_move::release(&db, "tr-first").unwrap();
    host_move::begin(
        &db,
        "t1",
        "/repo",
        "tr-second",
        ("addr", "studio", "env"),
        20,
    )
    .unwrap();

    assert!(
        host_move::hand(&db, "tr-first", "{}", 30).is_err(),
        "a stale transfer never hands over a newer move"
    );
    host_move::release(&db, "tr-first").unwrap();
    assert_eq!(host_move::state(&db, "t1").unwrap(), Some(State::Moving));
    host_move::hand(&db, "tr-second", "{}", 30).unwrap();
    host_move::seal(&db, "tr-second", "studio", 30).unwrap();
    assert_eq!(host_move::state(&db, "t1").unwrap(), Some(State::Sealed));
}

#[test]
fn the_new_host_keeps_the_record_of_its_move() {
    let db = db();
    let record = MoveRecord {
        from_host: "this-mac".to_owned(),
        to_host: "studio".to_owned(),
        at_ms: 1_700_000_000_000,
        transcript: Vec::<ChatItem>::new(),
    };
    assert_eq!(host_move::record(&db, "new").unwrap(), None);
    host_move::put_record(&db, "new", "tr-new", &record).unwrap();
    assert_eq!(host_move::record(&db, "new").unwrap(), Some(record));
}

#[test]
fn a_cancelled_message_waits_with_its_move_until_the_move_ends() {
    let db = db();
    host_move::begin(&db, "t1", "/repo", "tr-1", ("addr", "studio", "env"), 10).unwrap();
    host_move::enqueue(&db, "tr-1", 1, "one").unwrap();
    host_move::enqueue(&db, "tr-1", 2, "two").unwrap();
    assert_eq!(host_move::queued(&db, "tr-1").unwrap(), ["one", "two"]);
    host_move::forget(&db, "tr-1", 1).unwrap();
    assert_eq!(host_move::queued(&db, "tr-1").unwrap(), ["two"]);
    host_move::hand(&db, "tr-1", "{}", 20).unwrap();
    host_move::seal(&db, "tr-1", "studio", 20).unwrap();
    assert!(host_move::queued(&db, "tr-1").is_err());
}

#[test]
fn a_handoff_over_the_budget_is_refused_with_its_size_in_bytes() {
    let mut package = MovePackage {
        from_host: "this-mac".to_owned(),
        title: "Fix the port test".to_owned(),
        harness: Harness::ClaudeCode,
        model: model("claudeAgent"),
        permissions: Permissions::Supervised,
        branch: "fix/ports".to_owned(),
        remote_url: "git@example.invalid:acme/app.git".to_owned(),
        handoff: "x".repeat(16_001),
        transcript: Vec::new(),
        transcript_capped: false,
        queued: Vec::new(),
        transfer: "tr-budget".to_owned(),
    };
    assert_eq!(
        host_move::within_budget(&package),
        Err("The handoff is 16001 bytes; a move carries at most 16000 bytes.".to_owned())
    );
    package.handoff.pop();
    assert_eq!(host_move::within_budget(&package), Ok(()));
}

#[test]
fn a_session_with_an_attached_queued_message_cannot_move_yet() {
    let mut facts = facts();
    facts.queued_attachments = true;
    assert_eq!(
        host_move::check(&facts, None).blocked.as_deref(),
        Some(
            "A queued message has attachments, which a move does not carry, so the session cannot move yet."
        )
    );
}

#[test]
fn a_remote_that_carries_a_credential_blocks_the_move_and_an_ssh_user_does_not() {
    let blocked = |remote: &str| {
        let mut facts = facts();
        facts.remote_url = Some(remote.to_owned());
        host_move::check(&facts, None).blocked
    };
    for remote in [
        "git@example.invalid:acme/app.git",
        "ssh://git@example.invalid/acme/app.git",
        "https://example.invalid/acme/app.git",
        "ssh://git@[::1]/acme/app.git",
    ] {
        assert_eq!(blocked(remote), None, "{remote}");
    }
    for remote in [
        "https://ghp_token@example.invalid/acme/app.git",
        "https://user:pass@example.invalid/acme/app.git",
        "ssh://git:pass@example.invalid/acme/app.git",
        "ssh://example.invalid/acme/app.git?token=abc",
        "https::https://token@example.invalid/acme/app.git",
        "ext::ssh example.invalid git-upload-pack acme/app.git",
    ] {
        assert!(blocked(remote).is_some(), "{remote}");
    }
    let mut helper = facts();
    helper.remote_url = Some("https::https://token@example.invalid/acme/app.git".to_owned());
    assert_eq!(host_move::check(&helper, None).remote_url, None);
}

#[test]
fn a_check_omits_a_remote_that_carries_a_credential() {
    let mut facts = facts();
    facts.remote_url = Some("https://ghp_token@example.invalid/acme/app.git".to_owned());
    let check = host_move::check(&facts, None);
    assert_eq!(check.remote_url, None);
    assert!(check.blocked.is_some());
    assert_eq!(
        host_move::check(&self::facts(), None).remote_url.as_deref(),
        Some("git@example.invalid:acme/app.git")
    );
}

#[test]
fn a_check_says_whether_the_session_has_already_moved() {
    assert!(!host_move::check(&facts(), None).moved);
    assert!(!host_move::check(&facts(), Some(State::Moving)).moved);
    assert!(host_move::check(&facts(), Some(State::Sealed)).moved);
}

#[test]
fn an_arrival_row_is_kept_per_transfer() {
    let db = db();
    assert_eq!(host_move::arrival(&db, "tr-1").unwrap(), None);
    host_move::set_arrival(
        &db,
        "tr-1",
        &host_move::ArrivalRow {
            state: "landed".to_owned(),
            root: "/repo".to_owned(),
            thread: "thread-1".to_owned(),
            queued: vec!["carry one".to_owned()],
            delivered: 1,
        },
        10,
    )
    .unwrap();
    assert_eq!(
        host_move::arrival(&db, "tr-1")
            .unwrap()
            .map(|row| row.thread),
        Some("thread-1".to_owned())
    );
}
