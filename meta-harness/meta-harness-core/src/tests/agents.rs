use super::Dir;
use crate::agents::{self, NewAgent};
use crate::config::Store;
use crate::{ModelChoice, Permissions};

fn new_agent(name: &str) -> NewAgent {
    NewAgent {
        name: name.to_owned(),
        root: "/repo".to_owned(),
        model: ModelChoice {
            instance: "codex".to_owned(),
            model: "gpt-6.1".to_owned(),
            options: Vec::new(),
        },
        permissions: Permissions::FullAccess,
        instructions: "Keep the news page current.".to_owned(),
    }
}

#[test]
fn a_profile_keeps_a_private_memory_folder_and_outlives_a_restart() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();

    let created = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();

    assert_eq!(created.memory, home.join("agents/news-bot/memory"));
    assert!(created.memory.is_dir());
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&created.memory.metadata().unwrap().permissions())
            & 0o777,
        0o700
    );
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

    agents::advance(&store.db(), "news-bot", "thread-2").unwrap();
    drop(store);
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let listed = agents::list(&store.db(), &home).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "news-bot");
    assert_eq!(listed[0].model, new_agent("x").model);
    assert_eq!(
        (
            listed[0].thread.as_str(),
            listed[0].previous.as_deref(),
            listed[0].generation
        ),
        ("thread-2", Some("thread-1"), 1)
    );
    agents::settle(&store.db(), "news-bot").unwrap();
    assert_eq!(agents::list(&store.db(), &home).unwrap()[0].previous, None);

    agents::remove(&store.db(), "news-bot").unwrap();
    assert!(agents::list(&store.db(), &home).unwrap().is_empty());
    assert!(agents::remove(&store.db(), "news-bot").is_err());
}

#[test]
fn each_profile_thread_opens_with_the_agent_its_instructions_and_its_memory() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot"), "thread-1").unwrap();

    let first = agents::opening(&agent, None);
    let next = agents::opening(&agent, Some("Imported conversation history\nlatest"));

    for text in [&first, &next] {
        assert!(text.contains("news-bot"));
        assert!(text.contains("Keep the news page current."));
        assert!(text.contains(&agent.memory.display().to_string()));
    }
    assert!(!first.contains("latest"));
    assert!(next.ends_with("Imported conversation history\nlatest"));
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
    thread.rebind_routines("thread-2").await.unwrap();

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
    assert_eq!(upserts.len(), 2, "only the task bound to this thread moves");
    assert_eq!(upserts[1]["id"], "task-1");
    assert_eq!(upserts[1]["threadId"], "thread-2");
    assert_eq!(upserts[1]["requireExisting"], true);
    assert_eq!(upserts[1]["schedule"], bound_task("x", "y")["schedule"]);
    assert!(upserts[1].get("lastRunStatus").is_none());
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
