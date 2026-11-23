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
fn an_agent_made_by_chat_names_itself_from_its_instructions() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let request = "Every weekday at 7:30, read our feeds.";
    let agent = agents::create(
        &store.db(),
        &home,
        &NewAgent {
            name: None,
            instructions: String::new(),
            request: Some(request.to_owned()),
            ..new_agent("x")
        },
        "thread-1",
    )
    .unwrap();

    assert!(!agent.named);
    assert_eq!(agent.name, agent.id);
    let opening = agents::opening(&agent, "", None);
    assert!(opening.contains("without a name"));
    assert!(opening.contains(&agent.memory.join("instructions.md").display().to_string()));
    assert!(opening.contains("schedule_task"));
    assert_eq!(agents::spoken(&opening), Some(request));
    assert_eq!(
        agents::opening_generation(&agents::message_id(&agent.id, 3)),
        Some(3)
    );
    assert_eq!(agents::opening_generation("msg-1-0"), None);

    let seen = agents::Snapshot::default();
    let now = agents::Snapshot {
        instructions: "# news-bot\n\nRead the feeds.\n".to_owned(),
        routines: Vec::new(),
        fresh: false,
    };
    let found = agents::differences(&seen, &now);
    assert_eq!(
        found[0],
        (AgentChangeKind::Name, None, Some("news-bot".to_owned()))
    );
    assert_eq!(found[1].0, AgentChangeKind::Instructions);
    assert_eq!(found.len(), 2);
    assert_eq!(agents::heading("# Not A Name\n"), None);
    assert_eq!(
        agents::with_heading("# news-bot\n\nRead.\n", "morning-news"),
        "# morning-news\n\nRead.\n"
    );
    assert_eq!(
        agents::with_heading("Read.\n", "news-bot"),
        "# news-bot\nRead.\n"
    );
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
fn each_profile_thread_opens_with_the_agent_its_instructions_and_its_memory() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();
    let instructions = agents::instructions(&agent).unwrap();

    let first = agents::opening(&agent, &instructions, None);
    let next = agents::opening(
        &agent,
        &instructions,
        Some("Imported conversation history\nlatest"),
    );

    for text in [&first, &next] {
        assert!(text.contains("You are news-bot"));
        assert!(text.contains("Keep the news page current."));
        assert!(text.contains(&agent.memory.display().to_string()));
    }
    assert!(!first.contains("latest"));
    assert!(next.ends_with("Imported conversation history\nlatest"));
    assert_eq!(agents::spoken(&next), None);
}

fn bound_task(id: &str, thread: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "title": "Morning news", "prompt": "Check the feeds.", "enabled": true,
        "schedule": {"type": "fixed_time", "timeOfDay": "07:30", "weekdays": [1, 2]},
        "projectId": "project-1", "threadId": thread, "threadMode": "continue",
        "workspaceStrategy": {"type": "root"},
        "modelSelection": {"instanceId": "codex", "model": "gpt-6.1", "options": []},
        "runtimeMode": "full-access", "interactionMode": "default",
        "nextRunAt": null, "lastRunStatus": "never", "runCount": 0, "webhook": null,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_routine_wakes_the_profile_thread_and_moves_with_it_to_the_next_one() {
    use super::fake_t3::{chunk, ok};
    use super::thread::{follow_answering, projection};
    use crate::agents::NewRoutine;
    use crate::thread::{Schedule, ScheduleKind};
    use serde_json::json;

    let (fake, thread) = follow_answering(
        projection(json!({"thread": {"id": "thread-1", "projectId": "project-1", "title": "news-bot",
            "runtimeMode": "full-access", "branch": null, "worktreePath": null,
            "modelSelection": {"instanceId": "codex", "model": "gpt-6.1", "options": []}}})),
        1,
        |tag, payload, id| match tag {
            "scheduledTasks.upsert" => vec![ok(
                id,
                json!({"task": {"id": payload["id"].as_str().unwrap_or("task-9"),
                    "webhook": {"path": "/api/hooks/task-9/secret", "url": null, "hasSecret": false}}}),
            )],
            "scheduledTasks.subscribe" => vec![chunk(
                id,
                vec![json!({"tasks": [bound_task("task-1", "thread-1"), bound_task("task-2", "thread-other")]})],
            )],
            _ => vec![ok(id, json!({}))],
        },
    )
    .await;
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();

    let added = thread
        .add_routine(
            &agent,
            &NewRoutine {
                title: "Hook".to_owned(),
                prompt: "A delivery arrived: {{body}}".to_owned(),
                schedule: Schedule {
                    kind: ScheduleKind::Webhook,
                    every_ms: None,
                    time_of_day: None,
                    weekdays: Vec::new(),
                    run_at: None,
                },
            },
        )
        .await
        .unwrap();
    let tasks = thread.routine_tasks().await.unwrap();
    thread.save_routine(&tasks[0], true).await.unwrap();
    thread.save_routine(&tasks[0], false).await.unwrap();

    assert_eq!(added.id, "task-9");
    assert_eq!(added.webhook.as_deref(), Some("/api/hooks/task-9/secret"));
    let upserts = fake.payloads("scheduledTasks.upsert");
    assert_eq!(
        upserts[0],
        json!({
            "title": "Hook", "prompt": "A delivery arrived: {{body}}", "enabled": true,
            "schedule": {"type": "webhook"}, "projectId": "project-1", "threadId": "thread-1",
            "threadMode": "continue", "workspaceStrategy": {"type": "root"},
            "modelSelection": {"instanceId": "codex", "model": "gpt-6.1", "options": []},
            "runtimeMode": "full-access", "interactionMode": "default",
        })
    );
    assert_eq!(
        tasks.len(),
        1,
        "only the task bound to this thread is the agent's"
    );
    assert_eq!(upserts[1]["id"], "task-1");
    assert_eq!(upserts[1]["threadId"], "thread-1");
    assert_eq!(upserts[1]["requireExisting"], true);
    assert_eq!(upserts[1]["schedule"], bound_task("x", "y")["schedule"]);
    assert!(upserts[1].get("lastRunStatus").is_none());
    assert_eq!(
        upserts[2]["id"], "task-1",
        "a restored routine keeps its id"
    );
    assert!(upserts[2].get("requireExisting").is_none());
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
    assert!(!home.join("escape.md").exists());
    assert!(agents::too_large(agents::MAX_MEMORY_BYTES).is_none());
    assert!(
        agents::too_large(agents::MAX_MEMORY_BYTES + 1)
            .unwrap()
            .contains("at most 100 MiB")
    );
}
