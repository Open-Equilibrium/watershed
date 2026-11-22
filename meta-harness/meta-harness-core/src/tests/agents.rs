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

    let created = agents::create(&store.db(), &home, &new_agent("news-bot")).unwrap();

    assert_eq!(created.memory, home.join("agents/news-bot/memory"));
    assert!(created.memory.is_dir());
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&created.memory.metadata().unwrap().permissions())
            & 0o777,
        0o700
    );
    assert_eq!((created.thread.as_deref(), created.generation), (None, 0));
    assert!(
        agents::create(&store.db(), &home, &new_agent("news-bot"))
            .unwrap_err()
            .contains("already")
    );
    for bad in ["", "News", "1st", "a/b", &"x".repeat(41)] {
        assert!(
            agents::create(&store.db(), &home, &new_agent(bad)).is_err(),
            "{bad:?} is refused"
        );
    }

    agents::set_thread(&store.db(), "news-bot", "thread-7", 1).unwrap();
    drop(store);
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let listed = agents::list(&store.db(), &home).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "news-bot");
    assert_eq!(listed[0].model, new_agent("x").model);
    assert_eq!(
        (listed[0].thread.as_deref(), listed[0].generation),
        (Some("thread-7"), 1)
    );

    agents::remove(&store.db(), "news-bot").unwrap();
    assert!(agents::list(&store.db(), &home).unwrap().is_empty());
    assert!(agents::remove(&store.db(), "news-bot").is_err());
}

#[test]
fn each_profile_thread_opens_with_the_agent_its_instructions_and_its_memory() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let agent = agents::create(&store.db(), &home, &new_agent("news-bot")).unwrap();

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
