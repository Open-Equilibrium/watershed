use super::*;

async fn change_at(host: &remote::Host, index: usize) -> AgentChange {
    settle(host, "the agent's change is noticed", |host| async move {
        host.agent_changes("news-bot")
            .await
            .unwrap()
            .get(index)
            .cloned()
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_persistent_agent_moves_to_a_fresh_thread_with_its_routines_when_its_context_fills() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let created = host
        .agent_create(NewAgent {
            name: Some("news-bot".to_owned()),
            root: repo.to_string_lossy().into_owned(),
            model: start("", Workspace::Checkout, Permissions::FullAccess).model,
            permissions: Permissions::FullAccess,
            instructions: "Keep the news page current.".to_owned(),
            request: None,
        })
        .await
        .unwrap();
    let project = host.project(&repo).await.unwrap();
    let first = project.open(&created.thread).await.unwrap();
    wait(&first, completed(1)).await;
    let routine = host
        .agent_routine_add(
            "news-bot",
            NewRoutine {
                title: "Morning".to_owned(),
                prompt: "Check the feeds.".to_owned(),
                schedule: Schedule {
                    kind: ScheduleKind::FixedTime,
                    every_ms: None,
                    time_of_day: Some("07:30".to_owned()),
                    weekdays: Vec::new(),
                    run_at: None,
                },
            },
        )
        .await
        .unwrap();

    first.send("fill", false).await.unwrap();
    let moved = settle(&host, "the agent moves on", |host| async move {
        let agents = host.agents().await.unwrap();
        (agents[0].generation == 1 && agents[0].previous.is_none()).then(|| agents[0].clone())
    })
    .await;
    let next = project.open(&moved.thread).await.unwrap();
    let opened = wait(&next, completed(1)).await;

    assert!(created.memory.is_dir());
    assert_ne!(moved.thread, created.thread);
    let item = &opened.transcript[0];
    assert_eq!(item.detail, ItemDetail::Opening { handoff: true });
    assert_eq!(item.text, "", "Liquid shows no opening text");
    let opening = &opened.turns[0].prompt;
    assert!(opening.contains("Keep the news page current."), "{opening}");
    assert!(opening.contains(&created.memory.display().to_string()));
    assert!(opening.contains("Imported conversation history"));
    let routines = host.agent_routines("news-bot").await.unwrap();
    let ids: Vec<&str> = routines.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, [routine.id.as_str()]);
    assert!(first.automations().await.unwrap().current().is_empty());

    service.stop().await;
    let (service, host) = start_service(&home).await;
    assert_eq!(host.agents().await.unwrap()[0].thread, moved.thread);
    host.agent_remove("news-bot").await.unwrap();
    assert!(host.agents().await.unwrap().is_empty());
    let project = host.project(&repo).await.unwrap();
    let next = project.open(&moved.thread).await.unwrap();
    assert!(next.automations().await.unwrap().current().is_empty());
    assert!(created.memory.is_dir(), "the memory stays");
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_made_by_chat_names_itself_logs_its_changes_and_answers_a_mention() {
    let (_dir, repo, home) = fixture();
    let (service, host) = start_service(&home).await;
    let model = start("", Workspace::Checkout, Permissions::FullAccess).model;
    let created = host
        .agent_create(NewAgent {
            name: None,
            root: repo.to_string_lossy().into_owned(),
            model,
            permissions: Permissions::FullAccess,
            instructions: String::new(),
            request: Some("instruct".to_owned()),
        })
        .await
        .unwrap();
    let project = host.project(&repo).await.unwrap();
    let thread = project.open(&created.thread).await.unwrap();
    let opened = wait(&thread, completed(1)).await;
    assert_eq!(
        opened.transcript[0].text, "instruct",
        "Liquid shows the person's words"
    );
    let named = settle(
        &host,
        "the agent names itself after its first turn",
        |host| async move {
            let agents = host.agents().await.unwrap();
            agents[0].named.then(|| agents[0].clone())
        },
    )
    .await;
    assert_eq!(named.name, "news-bot");
    assert_eq!(named.memory, created.memory);
    let kinds = |changes: Vec<AgentChange>| changes.into_iter().map(|c| c.kind).collect::<Vec<_>>();
    let changes = host.agent_changes("news-bot").await.unwrap();
    assert_eq!(
        kinds(changes.clone()),
        [AgentChangeKind::Name, AgentChangeKind::Instructions]
    );
    assert_eq!(changes[1].turn, Some(1));

    let routine = host
        .agent_routine_add("news-bot", every_hour("Morning", "reinstruct"))
        .await
        .unwrap();
    host.agent_undo("news-bot", changes[1].id).await.unwrap();
    assert_eq!(host.agent_instructions("news-bot").await.unwrap(), "");
    assert!(host.agent_undo("news-bot", changes[1].id).await.is_err());
    let after = host.agent_changes("news-bot").await.unwrap();
    assert_eq!(after.len(), 2, "the person's changes are not the agent's");
    assert!(after[1].undone);

    thread.run_automation_now(&routine.id).await.unwrap();
    wait(&thread, completed(2)).await;
    let held = change_at(&host, 2).await;
    assert_eq!(held.kind, AgentChangeKind::Instructions);
    assert!(held.held, "a change from a routine waits for approval");
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "",
        "and is undone until then"
    );
    host.agent_apply("news-bot", held.id).await.unwrap();
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "# news-bot\n\nRead the feeds twice.\n"
    );
    assert!(!host.agent_changes("news-bot").await.unwrap()[2].held);
    assert!(host.agent_apply("news-bot", held.id).await.is_err());

    let stopped = host
        .agent_routine_add("news-bot", every_hour("Stopped", "rewrite-wait"))
        .await
        .unwrap();
    thread.run_automation_now(&stopped.id).await.unwrap();
    until_running(&thread).await;
    settle(
        &host,
        "the stopped routine run rewrites the instructions first",
        |host| async move {
            (host.agent_instructions("news-bot").await.unwrap()
                == "# news-bot\n\nRead the feeds three times.\n")
                .then_some(())
        },
    )
    .await;
    thread.stop().await.unwrap();
    let aborted = change_at(&host, 3).await;
    assert!(aborted.held, "a routine run that was stopped is held too");
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "# news-bot\n\nRead the feeds twice.\n"
    );
    host.agent_set_instructions("news-bot", "# news-bot\n\nRead.\n")
        .await
        .unwrap();
    assert!(
        host.agent_apply("news-bot", aborted.id).await.is_err(),
        "a held change does not overwrite what changed since"
    );
    host.agent_undo("news-bot", aborted.id).await.unwrap();
    assert_eq!(
        host.agent_instructions("news-bot").await.unwrap(),
        "# news-bot\n\nRead.\n",
        "discarding a held change keeps the current instructions"
    );
    host.agent_routine_remove("news-bot", &stopped.id)
        .await
        .unwrap();

    let chat = project
        .start(start(
            "Fix the move dialog",
            Workspace::Checkout,
            Permissions::FullAccess,
        ))
        .await
        .unwrap();
    wait(&chat, completed(1)).await;
    let message = host
        .agent_mention(
            "news-bot",
            &repo.to_string_lossy(),
            chat.id(),
            "Is main green?",
        )
        .await
        .unwrap();
    let answered = wait(&thread, completed(3)).await;
    let asked = answered
        .turns
        .iter()
        .find(|turn| turn.prompt.contains("Is main green?"))
        .expect("the mention is a turn of the agent");
    assert!(asked.prompt.contains("t3-context://v1/thread/"));
    let mentions = host.agent_mentions(chat.id()).await.unwrap();
    assert_eq!(
        (
            mentions[0].name.as_str(),
            mentions[0].message.as_str(),
            mentions[0].answered,
            mentions[0].answer.as_str()
        ),
        ("news-bot", message.as_str(), true, "Done.")
    );

    host.agent_remove("news-bot").await.unwrap();
    assert!(host.agent_mentions(chat.id()).await.unwrap().is_empty());
    service.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_moves_to_another_host_with_its_memory_and_routines() {
    let pair = Pair::codex().await;
    let created = pair
        .host_a
        .agent_create(NewAgent {
            name: Some("news-bot".to_owned()),
            root: pair.repo_a.to_string_lossy().into_owned(),
            model: codex_start("").model,
            permissions: Permissions::FullAccess,
            instructions: "Keep the news page current.".to_owned(),
            request: None,
        })
        .await
        .unwrap();
    let project_a = pair.host_a.project(&pair.repo_a).await.unwrap();
    let project_b = pair.host_b.project(&pair.repo_b).await.unwrap();
    let thread = project_a.open(&created.thread).await.unwrap();
    wait(&thread, completed(1)).await;
    std::fs::create_dir_all(created.memory.join("days")).unwrap();
    std::fs::write(created.memory.join("days/monday.md"), "Saw 4 items.\n").unwrap();
    let large = vec![7u8; 20 * 1024 * 1024];
    std::fs::write(created.memory.join("feeds.bin"), &large).unwrap();
    pair.host_a
        .agent_routine_add("news-bot", every_hour("Morning", "Check the feeds."))
        .await
        .unwrap();

    let check = thread.move_check().await.unwrap();
    let facts = check.agent.expect("the check names the agent");
    assert_eq!(
        (facts.name.as_str(), facts.routines, facts.earlier),
        ("news-bot", 1, 0)
    );
    assert!(facts.memory_bytes > 0);
    assert_eq!(check.blocked, None);
    let package = move_out(&thread, &transfer(), None).await.unwrap();
    assert!(package.agent.is_some(), "the package carries the agent");
    let moved = project_b.move_in(package.clone()).await.unwrap();

    let arrived = pair.host_b.agents().await.unwrap();
    assert_eq!(arrived.len(), 1);
    assert_eq!(
        (arrived[0].name.as_str(), arrived[0].thread.as_str()),
        ("news-bot", moved.id())
    );
    assert_eq!(
        std::fs::read_to_string(arrived[0].memory.join("days/monday.md")).unwrap(),
        "Saw 4 items.\n"
    );
    assert_eq!(
        std::fs::read(arrived[0].memory.join("feeds.bin")).unwrap(),
        large,
        "a memory past one socket frame arrives whole"
    );
    let instructions = pair.host_b.agent_instructions("news-bot").await.unwrap();
    assert!(instructions.contains("Keep the news page current."));
    let memory = arrived[0].memory.display().to_string();
    let told = wait(&moved, |s| {
        s.turns.iter().any(|t| t.prompt.contains(&memory))
    })
    .await;
    assert!(
        told.turns
            .iter()
            .any(|t| t.prompt.contains("You are news-bot")),
        "the moved agent learns its new memory folder"
    );
    let routines = pair.host_b.agent_routines("news-bot").await.unwrap();
    let titles: Vec<&str> = routines.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["Morning"]);
    let changes = pair.host_b.agent_changes("news-bot").await.unwrap();
    assert!(changes.is_empty());

    let record = moved.move_record().await.unwrap().unwrap();
    thread
        .move_finish(
            &package.transfer,
            MoveFinish::Seal {
                to_host: record.to_host,
            },
        )
        .await
        .unwrap();
    assert!(
        pair.host_a.agents().await.unwrap().is_empty(),
        "the agent left"
    );
    assert!(thread.automations().await.unwrap().current().is_empty());
    assert!(
        created.memory.is_dir(),
        "the old host keeps its copy of the memory"
    );
    pair.stop().await;
}
