use super::Dir;
use crate::agents::{self, AgentChangeKind, NewAgent};
use crate::config::Store;
use crate::{ModelChoice, Permissions};

fn new_agent(name: &str) -> NewAgent {
    NewAgent {
        name: Some(name.to_owned()),
        root: "/repo".to_owned(),
        model: ModelChoice {
            instance: "codex".to_owned(),
            model: "gpt-6.1".to_owned(),
            options: Vec::new(),
        },
        permissions: Permissions::FullAccess,
        instructions: "Keep the news page current.".to_owned(),
        request: None,
    }
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    std::os::unix::fs::PermissionsExt::mode(&path.metadata().unwrap().permissions()) & 0o777
}

#[test]
fn a_profile_keeps_private_instructions_and_memory_and_outlives_a_restart() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();

    let created = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();

    assert_eq!(
        created.memory,
        home.join("agents").join(&created.id).join("memory")
    );
    assert_eq!(
        agents::instructions(&created).unwrap(),
        "# news-bot\n\nKeep the news page current.\n"
    );
    #[cfg(unix)]
    {
        assert_eq!(mode(&created.memory), 0o700);
        assert_eq!(mode(&created.memory.join("instructions.md")), 0o600);
    }
    assert_eq!(
        (created.thread.as_str(), created.generation),
        ("thread-1", 0)
    );
    assert!(
        agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1")
            .unwrap_err()
            .contains("already")
    );
    for bad in ["", "News", "1st", "a/b", &"x".repeat(41)] {
        assert!(
            agents::create(&store.db(), &home, &new_agent(bad), "thread-1").is_err(),
            "{bad:?} is refused"
        );
    }

    agents::advance(&store.db(), &created.id, "thread-2").unwrap();
    agents::rename(&store.db(), &home, &created.id, "morning-news").unwrap();
    drop(store);
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let listed = agents::list(&store.db(), &home).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "morning-news");
    assert_eq!(
        listed[0].memory, created.memory,
        "a rename keeps the memory folder"
    );
    assert_eq!(listed[0].model, new_agent("x").model);
    assert_eq!(
        (
            listed[0].thread.as_str(),
            listed[0].previous.as_deref(),
            listed[0].generation
        ),
        ("thread-2", Some("thread-1"), 1)
    );
    assert_eq!(
        agents::threads(&store.db(), &created.id).unwrap(),
        ["thread-2", "thread-1"]
    );
    agents::settle(&store.db(), &created.id).unwrap();
    assert_eq!(
        agents::get(&store.db(), &home, "morning-news")
            .unwrap()
            .unwrap()
            .previous,
        None
    );

    agents::remove(&store.db(), &created.id).unwrap();
    assert!(agents::list(&store.db(), &home).unwrap().is_empty());
    assert!(
        agents::threads(&store.db(), &created.id)
            .unwrap()
            .is_empty()
    );
    assert!(agents::remove(&store.db(), &created.id).is_err());
}

#[test]
fn routine_changes_are_found_by_what_the_person_sees() {
    let task = |id: &str, title: &str, model: &str| {
        agents::routine_copy(&serde_json::json!({
            "id": id, "title": title, "prompt": "Check.", "enabled": true,
            "schedule": {"type": "interval", "everyMs": 60_000},
            "modelSelection": {"instanceId": "codex", "model": model}, "nextRunAt": "later",
        }))
    };
    let seen = agents::Snapshot {
        instructions: "# a\n".to_owned(),
        routines: vec![
            task("kept", "Kept", "m1"),
            task("renamed", "Old", "m1"),
            task("gone", "Gone", "m1"),
        ],
        fresh: false,
    };
    let now = agents::Snapshot {
        instructions: "# a\n".to_owned(),
        routines: vec![
            task("kept", "Kept", "m2"),
            task("renamed", "New", "m1"),
            task("new", "New", "m1"),
        ],
        fresh: false,
    };

    let found: Vec<_> = agents::differences(&seen, &now)
        .into_iter()
        .map(|(kind, ..)| kind)
        .collect();

    assert_eq!(
        found,
        [
            AgentChangeKind::RoutineChanged,
            AgentChangeKind::RoutineAdded,
            AgentChangeKind::RoutineRemoved
        ]
    );
    assert!(task("x", "X", "m").get("nextRunAt").is_none());
}

#[test]
fn changes_and_mentions_are_kept_per_agent() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();
    let routine = serde_json::json!({"id": "task-1", "title": "Morning news", "enabled": true,
        "schedule": {"type": "fixed_time", "timeOfDay": "07:30", "weekdays": [1, 2, 3, 4, 5]}});

    agents::record(
        &store.db(),
        &agent.id,
        "thread-1",
        Some(2),
        false,
        (
            AgentChangeKind::RoutineAdded,
            None,
            Some(routine.to_string()),
        ),
    )
    .unwrap();
    let changes = agents::changes(&store.db(), &agent.id).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].turn, Some(2));
    assert_eq!(changes[0].routine.as_ref().unwrap().title, "Morning news");
    assert!(!changes[0].undone);
    agents::mark_undone(&store.db(), changes[0].id).unwrap();
    assert!(
        agents::change(&store.db(), &agent.id, changes[0].id)
            .unwrap()
            .undone
    );
    assert!(agents::change(&store.db(), "other", changes[0].id).is_err());

    agents::add_mention(&store.db(), &agent, "chat-7", "msg-1", "Is main green?").unwrap();
    let mentions = agents::mentions(&store.db(), "chat-7").unwrap();
    assert_eq!(
        (
            mentions[0].name.as_str(),
            mentions[0].thread.as_str(),
            mentions[0].message.as_str(),
            mentions[0].text.as_str()
        ),
        ("news-bot", "thread-1", "msg-1", "Is main green?")
    );
    assert!(agents::mentions(&store.db(), "chat-8").unwrap().is_empty());
}

#[test]
fn a_moving_agent_carries_its_memory_files_and_refuses_unsafe_paths() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();
    std::fs::create_dir_all(agent.memory.join("days")).unwrap();
    let bytes: Vec<u8> = (0..=255).collect();
    std::fs::write(agent.memory.join("days/raw.bin"), &bytes).unwrap();

    let carry = agents::pack(&agent, Vec::new()).unwrap();
    assert_eq!(
        carry
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["days/raw.bin", "instructions.md"]
    );
    let target = home.join("arrived");
    agents::unpack(&target, &carry.files).unwrap();
    assert_eq!(std::fs::read(target.join("days/raw.bin")).unwrap(), bytes);
    #[cfg(unix)]
    assert_eq!(mode(&target.join("days/raw.bin")), 0o600);

    for unsafe_path in ["../escape.md", "/etc/passwd", ""] {
        let file = agents::CarriedFile {
            path: unsafe_path.to_owned(),
            bytes: String::new(),
        };
        assert!(
            agents::unpack(&target, &[file]).is_err(),
            "{unsafe_path:?} is refused"
        );
    }
    let twice = agents::CarriedFile {
        path: "days/raw.bin".to_owned(),
        bytes: String::new(),
    };
    assert!(
        agents::unpack(&home.join("twice"), &[twice.clone(), twice]).is_err(),
        "a path listed twice is refused"
    );
    assert!(!home.join("escape.md").exists());
    assert!(agents::too_large(agents::MAX_MEMORY_BYTES).is_none());
    assert!(
        agents::too_large(agents::MAX_MEMORY_BYTES + 1)
            .unwrap()
            .contains("at most 100 MiB")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_memory_name_that_is_not_utf8_is_refused_instead_of_carried_under_a_changed_name() {
    use std::os::unix::ffi::OsStrExt;
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();
    for name in [&b"caf\xe9.md"[..], &b"caf\xe8.md"[..]] {
        std::fs::write(agent.memory.join(std::ffi::OsStr::from_bytes(name)), "text").unwrap();
    }
    assert!(agents::pack(&agent, Vec::new()).is_err());
}

#[test]
fn packing_reads_a_file_only_within_the_budget_its_predecessors_left() {
    let home = Dir::new();
    let file = home.join("grown.md");
    std::fs::write(&file, "0123456789").unwrap();
    assert_eq!(agents::read_within(&file, 0).unwrap(), b"0123456789");
    assert!(agents::read_within(&file, agents::MAX_MEMORY_BYTES - 10).is_ok());
    assert!(agents::read_within(&file, agents::MAX_MEMORY_BYTES - 9).is_err());
}

#[test]
fn saving_instructions_leaves_a_memory_file_named_like_its_temporary_file_alone() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();
    let kept = agent.memory.join("instructions.tmp");
    std::fs::write(&kept, "kept").unwrap();
    agents::write_instructions(&agent, "# news-bot\nNew.").unwrap();
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "kept");
    assert_eq!(agents::instructions(&agent).unwrap(), "# news-bot\nNew.");
}

#[test]
fn a_profile_moves_on_once_its_context_is_four_fifths_full() {
    let context = |tokens, window| {
        Some(crate::Context {
            tokens,
            window,
            compacts_at: None,
        })
    };
    assert!(!agents::full(context(Some(799), 1000)));
    assert!(agents::full(context(Some(800), 1000)));
    assert!(!agents::full(context(None, 1000)));
    assert!(!agents::full(context(Some(5), 0)));
    assert!(!agents::full(None));
}
