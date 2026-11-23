use std::collections::BTreeMap;

use super::Dir;
use crate::config::{
    Action, Blocks, Change, Difference, Direction, McpServer, Proposed, Store, diff_blocks,
};

struct Fixture {
    store: Store,
    dir: Dir,
}

impl Fixture {
    fn new() -> Self {
        let dir = Dir::new();
        let store = Store::open(&dir.join("meta-harness.sqlite")).unwrap();
        Self { store, dir }
    }

    fn reopen(&self) -> Store {
        Store::open(&self.dir.join("meta-harness.sqlite")).unwrap()
    }

    fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.join("meta-harness.sqlite")).unwrap()
    }
}

fn instruction(name: &str, text: &str) -> Change {
    Change::Instruction {
        name: name.to_owned(),
        text: Some(text.to_owned()),
    }
}

fn skill(name: &str) -> Change {
    Change::Skill {
        name: name.to_owned(),
        files: Some(BTreeMap::from([(
            "SKILL.md".to_owned(),
            "---\nname: x\n---\nDo it.".to_owned(),
        )])),
    }
}

fn skill_files(pairs: &[(&str, &str)]) -> Change {
    Change::Skill {
        name: "s".to_owned(),
        files: Some(
            pairs
                .iter()
                .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
                .collect(),
        ),
    }
}

fn server(name: &str) -> Change {
    Change::McpServer {
        name: name.to_owned(),
        server: Some(McpServer::Command {
            command: "/usr/local/bin/docs-mcp".to_owned(),
            args: vec!["--stdio".to_owned()],
            env: BTreeMap::from([("DOCS_ROOT".to_owned(), "/srv/docs".to_owned())]),
        }),
    }
}

fn command_server(command: &str, env: &[(&str, &str)]) -> Change {
    Change::McpServer {
        name: "m".to_owned(),
        server: Some(McpServer::Command {
            command: command.to_owned(),
            args: vec![],
            env: env
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }),
    }
}

fn propose(store: &Store, changes: Vec<Change>) -> u64 {
    store
        .propose("cli", "change", Proposed::Changes(changes))
        .unwrap()
}

#[test]
fn a_proposal_takes_effect_only_once_approved_and_every_step_is_logged() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    assert_eq!(store.current().unwrap().revision, 0);

    let number = propose(
        store,
        vec![
            instruction("rules", "Run the tests."),
            skill("review"),
            server("docs"),
        ],
    );
    assert_eq!(store.current().unwrap().blocks, Blocks::default());
    assert_eq!(
        store
            .pending()
            .unwrap()
            .iter()
            .map(|p| p.number)
            .collect::<Vec<_>>(),
        vec![number]
    );

    let current = store.approve("liquid", number, |_| Ok(())).unwrap();
    assert_eq!(current.revision, 1);
    assert_eq!(current.blocks.instructions["rules"], "Run the tests.");
    assert!(current.blocks.skills["review"].contains_key("SKILL.md"));
    assert!(current.blocks.mcp_servers.contains_key("docs"));
    assert!(store.pending().unwrap().is_empty());

    let log: Vec<_> = store
        .log()
        .unwrap()
        .into_iter()
        .map(|e| (e.client, e.action, e.proposal, e.revision))
        .collect();
    assert_eq!(
        log,
        vec![
            ("cli".to_owned(), Action::Proposed, number, None),
            ("liquid".to_owned(), Action::Approved, number, Some(1)),
        ]
    );
    assert_eq!(fixture.reopen().current().unwrap(), current);
}

#[test]
fn a_decided_or_unknown_proposal_cannot_be_decided_again() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    let approved = propose(store, vec![instruction("a", "x")]);
    let rejected = propose(store, vec![instruction("b", "y")]);
    store.approve("cli", approved, |_| Ok(())).unwrap();
    store.reject("cli", rejected).unwrap();
    for number in [approved, rejected, 99] {
        assert!(
            store.approve("cli", number, |_| Ok(())).is_err(),
            "{number}"
        );
        assert!(store.reject("cli", number).is_err(), "{number}");
    }
    assert_eq!(store.current().unwrap().revision, 1);
    assert!(
        !store
            .current()
            .unwrap()
            .blocks
            .instructions
            .contains_key("b")
    );
}

#[test]
fn a_restore_brings_back_an_earlier_revision_exactly() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    let first = propose(store, vec![instruction("rules", "one"), server("docs")]);
    let kept = store.approve("cli", first, |_| Ok(())).unwrap();
    let second = propose(
        store,
        vec![
            instruction("rules", "two"),
            skill("review"),
            Change::McpServer {
                name: "docs".to_owned(),
                server: None,
            },
        ],
    );
    store.approve("cli", second, |_| Ok(())).unwrap();

    let restore = store
        .propose("cli", "back", Proposed::Restore(kept.revision))
        .unwrap();
    let third = propose(store, vec![instruction("late", "x")]);
    store.approve("cli", third, |_| Ok(())).unwrap();
    let restored = store.approve("cli", restore, |_| Ok(())).unwrap();
    assert_eq!(restored.revision, 4);
    assert_eq!(restored.blocks, kept.blocks);
    assert!(
        store
            .propose("cli", "nowhere", Proposed::Restore(99))
            .is_err()
    );
}

#[test]
fn the_log_and_revisions_refuse_changes_in_place() {
    let fixture = Fixture::new();
    let number = propose(&fixture.store, vec![instruction("a", "x")]);
    fixture.store.approve("cli", number, |_| Ok(())).unwrap();
    drop(fixture.store.log());
    let raw = fixture.raw();
    for statement in [
        "DELETE FROM log",
        "UPDATE log SET client = 'agent'",
        "DELETE FROM revision",
        "UPDATE revision SET blocks = '{}'",
        "UPDATE proposal SET summary = 'other'",
        "DELETE FROM proposal",
    ] {
        assert!(raw.execute(statement, []).is_err(), "{statement}");
    }
}

#[test]
fn an_invalid_change_is_refused_with_its_reason() {
    let fixture = Fixture::new();
    let big = "x".repeat(300 * 1024);
    let cases = [
        (instruction("", "x"), "name"),
        (instruction("a b", "x"), "name"),
        (instruction(&"n".repeat(65), "x"), "name"),
        (instruction("a", &big), "too large"),
        (skill_files(&[("README.md", "x")]), "SKILL.md"),
        (
            skill_files(&[("SKILL.md", "x"), ("../escape", "x")]),
            "path",
        ),
        (skill_files(&[("SKILL.md", "x"), ("/etc/x", "x")]), "path"),
        (
            skill_files(&[("SKILL.md", "x"), ("x", "x"), ("x/y", "x")]),
            "clash",
        ),
        (
            skill_files(&[("SKILL.md", "x"), ("A/b", "x"), ("a", "x")]),
            "clash",
        ),
        (command_server("", &[]), "command"),
        (command_server("x", &[("BAD KEY", "v")]), "variable"),
        (
            Change::McpServer {
                name: "m".to_owned(),
                server: Some(McpServer::Url {
                    url: "file:///etc/passwd".to_owned(),
                }),
            },
            "http",
        ),
    ];
    for (change, reason) in cases {
        let error = fixture
            .store
            .propose("cli", "bad", Proposed::Changes(vec![change.clone()]))
            .expect_err(&format!("{change:?}"));
        assert!(error.contains(reason), "{error} lacks {reason}");
    }
    assert!(fixture.store.pending().unwrap().is_empty());

    let both = propose(&fixture.store, vec![skill("review"), skill("Review")]);
    let error = fixture.store.approve("cli", both, |_| Ok(())).unwrap_err();
    assert!(error.contains("only in case"), "{error}");
}

#[test]
fn a_revision_the_homes_cannot_take_is_not_approved() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    let number = propose(store, vec![instruction("a", "x")]);
    let written = std::cell::RefCell::new(Vec::new());
    let failed = store.approve("cli", number, |blocks| {
        written
            .borrow_mut()
            .push(blocks.instructions.contains_key("a"));
        if blocks.instructions.is_empty() {
            Ok(())
        } else {
            Err("The disk is full.".to_owned())
        }
    });
    assert_eq!(written.into_inner(), [true, false]);
    assert_eq!(failed, Err("The disk is full.".to_owned()));
    assert_eq!(store.current().unwrap().revision, 0);
    assert_eq!(
        store.pending().unwrap().len(),
        1,
        "it can be approved again"
    );
    assert_eq!(store.log().unwrap().len(), 1, "only the proposal is logged");
}

#[test]
fn a_failure_after_the_homes_took_the_revision_puts_the_current_one_back() {
    for fault in ["revision", "log", "commit"] {
        let fixture = Fixture::new();
        let store = &fixture.store;
        let number = propose(store, vec![instruction("rules", "x")]);
        let raw = fixture.raw();
        match fault {
            "commit" => raw
                .execute_batch("BEGIN; SELECT count(*) FROM revision;")
                .unwrap(),
            table => raw
                .execute_batch(&format!(
                    "CREATE TRIGGER refuse BEFORE INSERT ON {table} \
                     BEGIN SELECT RAISE(ABORT, 'refused'); END;"
                ))
                .unwrap(),
        }
        let before = store.current().unwrap();
        let written = std::cell::RefCell::new(Vec::new());
        let result = store.approve("cli", number, |blocks| {
            written.borrow_mut().push(blocks.clone());
            Ok(())
        });
        assert!(result.is_err(), "{fault}");
        let written = written.into_inner();
        assert_eq!(written.len(), 2, "{fault}");
        assert_eq!(written.last(), Some(&before.blocks), "{fault}");
        assert_eq!(store.current().unwrap(), before, "{fault}");
        assert_eq!(store.pending().unwrap().len(), 1, "{fault}");
    }
}

#[test]
fn a_failed_rollback_leaves_the_homes_unreconciled_until_an_approval_writes_them() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    let first = propose(store, vec![instruction("rules", "x")]);
    let second = propose(store, vec![instruction("more", "y")]);
    let raw = fixture.raw();
    raw.execute_batch(
        "CREATE TRIGGER refuse BEFORE INSERT ON revision BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    let calls = std::cell::Cell::new(0);
    let failed = store.approve("cli", first, |_| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Ok(())
        } else {
            Err("the homes refuse writes".to_owned())
        }
    });
    let error = failed.unwrap_err();
    assert!(
        error.contains("refused") && error.contains("the homes refuse writes"),
        "{error}"
    );
    assert!(!store.reconciled());

    raw.execute_batch("DROP TRIGGER refuse").unwrap();
    let refused = store
        .approve("cli", second, |_| Err("the homes refuse writes".to_owned()))
        .unwrap_err();
    assert!(refused.contains("the homes refuse writes"), "{refused}");
    assert!(!store.reconciled());

    let written = std::cell::RefCell::new(Vec::new());
    store
        .approve("cli", second, |blocks| {
            written.borrow_mut().push(blocks.clone());
            Ok(())
        })
        .unwrap();
    let written = written.into_inner();
    assert_eq!(written.len(), 1);
    assert!(written[0].instructions.contains_key("more"));
    assert!(store.reconciled());
    assert_eq!(store.current().unwrap().revision, 1);
}

#[test]
fn a_revert_is_approved_while_the_homes_refuse_the_current_revision() {
    let fixture = Fixture::new();
    let store = &fixture.store;
    let rules = propose(store, vec![instruction("rules", "x")]);
    store.approve("cli", rules, |_| Ok(())).unwrap();
    store.set_reconciled(false);
    let revert = store
        .propose("cli", "back to the start", Proposed::Restore(0))
        .unwrap();
    let written = std::cell::RefCell::new(Vec::new());
    let approved = store
        .approve("cli", revert, |blocks| {
            written.borrow_mut().push(blocks.clone());
            if blocks.instructions.is_empty() {
                Ok(())
            } else {
                Err("the homes refuse revision 1".to_owned())
            }
        })
        .unwrap();
    assert_eq!(approved.revision, 2);
    assert!(store.reconciled());
    assert_eq!(written.into_inner(), vec![Blocks::default()]);
}

#[test]
fn a_diff_names_each_change_with_its_direction_and_identical_blocks_need_none() {
    let files = BTreeMap::from([("SKILL.md".to_owned(), "review".to_owned())]);
    let docs = McpServer::Url {
        url: "https://docs".to_owned(),
    };
    let mut from = Blocks::default();
    from.instructions.insert("AGENTS.md".into(), "rules".into());
    from.skills.insert("review".into(), files.clone());
    from.mcp_servers.insert("docs".into(), docs.clone());
    let mut to = Blocks::default();
    to.instructions
        .insert("AGENTS.md".into(), "old rules".into());
    to.instructions
        .insert("notes.md".into(), "kept elsewhere".into());
    to.mcp_servers.insert("docs".into(), docs.clone());
    to.mcp_servers.insert(
        "old".into(),
        McpServer::Url {
            url: "https://old".to_owned(),
        },
    );

    assert!(diff_blocks(&from, &from.clone()).is_empty());
    assert_eq!(
        diff_blocks(&from, &to),
        vec![
            Difference {
                direction: Direction::Replace,
                change: Change::Instruction {
                    name: "AGENTS.md".into(),
                    text: Some("rules".into()),
                },
            },
            Difference {
                direction: Direction::Remove,
                change: Change::Instruction {
                    name: "notes.md".into(),
                    text: None,
                },
            },
            Difference {
                direction: Direction::Add,
                change: Change::Skill {
                    name: "review".into(),
                    files: Some(files),
                },
            },
            Difference {
                direction: Direction::Remove,
                change: Change::McpServer {
                    name: "old".into(),
                    server: None,
                },
            },
        ]
    );
}

#[test]
fn a_change_with_an_unknown_field_is_refused_instead_of_read_as_a_removal() {
    for text in [
        r#"{"instruction":{"name":"style","txt":"Be verbose."}}"#,
        r#"{"instruction":{"name":"style","text":"Be verbose.","extra":1}}"#,
        r#"{"skill":{"name":"review","files":null,"extra":1}}"#,
        r#"{"mcp_server":{"name":"docs","server":{"url":"https://x","extra":1}}}"#,
        r#"{"extra":1}"#,
    ] {
        assert!(serde_json::from_str::<Change>(text).is_err(), "{text}");
    }
    let removal: Change =
        serde_json::from_str(r#"{"instruction":{"name":"style","text":null}}"#).unwrap();
    assert_eq!(
        removal,
        Change::Instruction {
            name: "style".to_owned(),
            text: None
        }
    );
}
