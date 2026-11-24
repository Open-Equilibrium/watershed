use serde_json::{Value, json};

use super::fake_t3::{chunk, ok};
use super::thread::{follow_answering, projection};
use super::within;
use crate::thread::{Automation, LocalServer, Pace, RunStatus, ScheduleKind};

const SERVERS: &str = "subscribeDiscoveredLocalServers";
const TASKS: &str = "scheduledTasks.subscribe";

fn discovered() -> Value {
    json!({
        "servers": [
            {"host": "localhost", "port": 5173, "url": "http://localhost:5173/", "processName": "vite",
             "pid": 4242, "terminal": {"threadId": "thread-1", "terminalId": "term-1"}},
            {"host": "localhost", "port": 3000, "url": "http://localhost:3000/", "processName": null,
             "pid": 4343, "terminal": null, "agent": {"threadId": "thread-1"}},
            {"host": "localhost", "port": 8080, "url": "http://localhost:8080/", "processName": "go",
             "pid": 4444, "terminal": {"threadId": "thread-2", "terminalId": "term-2"}},
        ],
        "scannedAt": "2026-11-15T10:00:00.000Z",
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_servers_of_a_thread_are_watched_and_one_can_be_stopped() {
    let (fake, thread) = follow_answering(projection(json!({})), 1, |tag, _, id| match tag {
        SERVERS => vec![chunk(id, vec![discovered()])],
        _ => vec![ok(id, json!({}))],
    })
    .await;
    let mut servers = thread.servers().await.unwrap();
    assert_eq!(
        servers.current(),
        vec![
            LocalServer {
                host: "localhost".to_owned(),
                port: 5173,
                url: "http://localhost:5173/".to_owned(),
                process: Some("vite".to_owned()),
                pid: Some(4242),
                terminal: true,
            },
            LocalServer {
                host: "localhost".to_owned(),
                port: 3000,
                url: "http://localhost:3000/".to_owned(),
                process: None,
                pid: Some(4343),
                terminal: false,
            },
        ]
    );
    fake.received(SERVERS, 1).await;
    fake.push(
        SERVERS,
        vec![json!({"servers": [], "scannedAt": "2026-11-15T10:01:00.000Z"})],
    );
    assert_eq!(
        within("the servers change", servers.changed()).await,
        Some(Vec::new())
    );

    thread.stop_server(4242, 5173).await.unwrap();
    assert_eq!(
        fake.payloads("preview.stopLocalServer"),
        vec![json!({"pid": 4242, "port": 5173})]
    );
}

fn tasks() -> Value {
    json!({"tasks": [
        {"id": "task-1", "title": "Nightly\u{202e}check", "prompt": "check", "enabled": true,
         "schedule": {"type": "fixed_time", "timeOfDay": "09:30", "weekdays": [1, 3]},
         "projectId": "project-1", "threadId": "thread-1", "threadMode": "continue",
         "nextRunAt": "2099-01-01T09:30:00.000Z", "lastRunAt": null, "lastRunStatus": "never",
         "lastRunError": null, "runCount": 0, "blockedByPolicy": null},
        {"id": "task-2", "title": "Poll", "prompt": "poll", "enabled": false,
         "schedule": {"type": "interval", "everyMs": 300000}, "projectId": "project-1",
         "threadId": "thread-1", "threadMode": "continue", "nextRunAt": null,
         "lastRunAt": "2026-11-15T10:00:00.000Z", "lastRunStatus": "failed", "lastRunError": "boom",
         "runCount": 2, "blockedByPolicy": "Webhooks are disabled by policy."},
        {"id": "task-3", "title": "Elsewhere", "prompt": "x", "enabled": true,
         "schedule": {"type": "once", "runAt": "2099-02-02T10:00:00+01:00"}, "projectId": "project-1",
         "threadId": "thread-2", "threadMode": "continue", "nextRunAt": null, "lastRunAt": null,
         "lastRunStatus": "succeeded", "lastRunError": null, "runCount": 1},
    ]})
}

#[tokio::test(flavor = "multi_thread")]
async fn the_automations_of_a_thread_show_their_schedule_and_state_and_can_be_run_or_paused() {
    let (fake, thread) = follow_answering(projection(json!({})), 1, |tag, _, id| match tag {
        TASKS => vec![chunk(id, vec![tasks()])],
        _ => vec![ok(id, json!({"task": {}}))],
    })
    .await;
    let automations = thread.automations().await.unwrap().current();
    assert_eq!(
        automations,
        vec![
            Automation {
                id: "task-1".to_owned(),
                title: "Nightly\u{202e}check".to_owned(),
                schedule: crate::thread::Schedule {
                    kind: ScheduleKind::FixedTime,
                    every_ms: None,
                    time_of_day: Some("09:30".to_owned()),
                    weekdays: vec![1, 3],
                    run_at: None,
                },
                enabled: true,
                next_run_ms: Some(4_070_943_000_000),
                last_run: RunStatus::Never,
                blocked_by_policy: None,
            },
            Automation {
                id: "task-2".to_owned(),
                title: "Poll".to_owned(),
                schedule: crate::thread::Schedule {
                    kind: ScheduleKind::Interval,
                    every_ms: Some(300_000),
                    time_of_day: None,
                    weekdays: Vec::new(),
                    run_at: None,
                },
                enabled: false,
                next_run_ms: None,
                last_run: RunStatus::Failed,
                blocked_by_policy: Some("Webhooks are disabled by policy.".to_owned()),
            },
        ]
    );
    thread.set_automation_enabled("task-2", true).await.unwrap();
    thread.run_automation_now("task-1").await.unwrap();
    assert_eq!(
        fake.payloads("scheduledTasks.setEnabled"),
        vec![json!({"id": "task-2", "enabled": true})]
    );
    assert_eq!(
        fake.payloads("scheduledTasks.runNow"),
        vec![json!({"id": "task-1"})]
    );
}

fn usage_config() -> Value {
    json!({
        "environment": {"label": "Test Machine"},
        "providers": [
            {"instanceId": "scripted", "driver": "acpRegistry", "enabled": true,
             "usageLimits": {"checkedAt": "2026-11-15T10:00:00.000Z", "windows": [
                {"id": "session", "kind": "session", "label": "5h", "usedPercent": 40,
                 "resetsAt": "2099-01-01T00:00:00.000Z", "windowDurationMins": 300},
                {"id": "weekly", "kind": "weekly", "label": "Week", "usedPercent": 10,
                 "resetsAt": "2000-01-01T00:00:00.000Z", "windowDurationMins": 300},
             ]}},
            {"instanceId": "limited", "driver": "acpRegistry", "enabled": true,
             "usageLimits": {"checkedAt": "2026-11-15T10:00:00.000Z", "windows": [],
                             "unavailable": {"reason": "probeFailed", "message": "Token expired"}}},
            {"instanceId": "plain", "driver": "acpRegistry", "enabled": true,
             "usageLimits": {"checkedAt": "2026-11-15T10:00:00.000Z", "windows": [],
                             "unavailable": {"reason": "unsupported"}}},
        ],
    })
}

async fn usage_of(instance: &str) -> Option<crate::thread::UsageLimits> {
    let mut records = projection(json!({}));
    records["thread"]["modelSelection"]["instanceId"] = json!(instance);
    let (_fake, thread) = follow_answering(records, 1, |tag, _, id| match tag {
        "server.getConfig" => vec![ok(id, usage_config())],
        _ => vec![ok(id, json!({}))],
    })
    .await;
    thread.usage_limits().await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_usage_limits_of_the_thread_provider_show_remaining_room_pace_and_notices() {
    let limits = usage_of("scripted")
        .await
        .expect("the provider reports limits");
    assert_eq!(limits.notice, None);
    let windows: Vec<_> = limits
        .windows
        .iter()
        .map(|w| {
            (
                w.label.as_str(),
                w.remaining_percent,
                w.resets_at_ms.is_some(),
                w.pace,
            )
        })
        .collect();
    assert_eq!(
        windows,
        vec![
            ("5h", 60, true, Some(Pace::Ahead)),
            ("Week", 90, false, Some(Pace::Under)),
        ]
    );
    let probe = usage_of("limited")
        .await
        .expect("the notice is a report too");
    assert_eq!(probe.notice.as_deref(), Some("Token expired"));
    assert!(probe.windows.is_empty());
    let plain = usage_of("plain")
        .await
        .expect("unsupported accounts report too");
    assert_eq!(
        plain.notice.as_deref(),
        Some("This account has no subscription limits.")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moved_session_pauses_its_enabled_scheduled_tasks_and_no_others() {
    let (fake, thread) = follow_answering(projection(json!({})), 1, |tag, _, id| match tag {
        TASKS => vec![chunk(
            id,
            vec![json!({"tasks": [
                {"id": "task-1", "title": "Nightly", "threadId": "thread-1", "enabled": true,
                 "schedule": {"type": "interval", "everyMs": 3_600_000}},
                {"id": "task-2", "title": "Paused", "threadId": "thread-1", "enabled": false,
                 "schedule": {"type": "interval", "everyMs": 3_600_000}},
                {"id": "task-3", "title": "Other", "threadId": "thread-2", "enabled": true,
                 "schedule": {"type": "interval", "everyMs": 3_600_000}},
            ]})],
        )],
        _ => vec![ok(id, json!({}))],
    })
    .await;
    thread.pause_automations().await.unwrap();
    assert_eq!(
        fake.payloads("scheduledTasks.setEnabled"),
        vec![json!({"id": "task-1", "enabled": false})]
    );
}
