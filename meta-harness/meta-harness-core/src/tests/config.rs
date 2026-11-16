//! The central configuration's store: proposals take effect only once approved, every step is
//! logged append-only, and a revert restores an earlier revision (ADR-0298).

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::{Action, Blocks, Change, McpServer, Proposed, Store};

/// A store in a fresh folder, removed on drop.
struct Fixture {
    dir: PathBuf,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("mh-config-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("meta-harness.sqlite")).unwrap();
        Self { dir, store }
    }

    fn reopen(&self) -> Store {
        Store::open(&self.dir.join("meta-harness.sqlite")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
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
    // The state outlives the store.
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
    // A change approved after the restore was proposed is undone by it too.
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
    let raw = rusqlite::Connection::open(fixture.dir.join("meta-harness.sqlite")).unwrap();
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
    let files = |pairs: &[(&str, &str)]| {
        Some(
            pairs
                .iter()
                .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let big = "x".repeat(300 * 1024);
    let cases = [
        (instruction("", "x"), "name"),
        (instruction("a b", "x"), "name"),
        (instruction(&"n".repeat(65), "x"), "name"),
        (instruction("a", &big), "too large"),
        (
            Change::Skill {
                name: "s".to_owned(),
                files: files(&[("README.md", "x")]),
            },
            "SKILL.md",
        ),
        (
            Change::Skill {
                name: "s".to_owned(),
                files: files(&[("SKILL.md", "x"), ("../escape", "x")]),
            },
            "path",
        ),
        (
            Change::Skill {
                name: "s".to_owned(),
                files: files(&[("SKILL.md", "x"), ("/etc/x", "x")]),
            },
            "path",
        ),
        (
            Change::Skill {
                name: "s".to_owned(),
                files: files(&[("SKILL.md", "x"), ("x", "x"), ("x/y", "x")]),
            },
            "clash",
        ),
        // Case-insensitive file systems, such as macOS's default, make these one folder.
        (
            Change::Skill {
                name: "s".to_owned(),
                files: files(&[("SKILL.md", "x"), ("A/b", "x"), ("a", "x")]),
            },
            "clash",
        ),
        (
            Change::McpServer {
                name: "m".to_owned(),
                server: Some(McpServer::Command {
                    command: String::new(),
                    args: vec![],
                    env: BTreeMap::new(),
                }),
            },
            "command",
        ),
        (
            Change::McpServer {
                name: "m".to_owned(),
                server: Some(McpServer::Command {
                    command: "x".to_owned(),
                    args: vec![],
                    env: BTreeMap::from([("BAD KEY".to_owned(), "v".to_owned())]),
                }),
            },
            "variable",
        ),
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

    // Skills that differ only in case would share a folder on macOS's default file system.
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
        // The new revision fails to write; the current one is put back.
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
