use meta_harness_core::{
    Automation, Group, Lineage, LineageRow, LocalServer, Pace, Relation, RunStatus, Schedule,
    ScheduleKind, UsageLimits, UsageWindow,
};

use crate::api::details::{
    BridgeAutomation, BridgeLineageGroup, BridgeLineageRelation, BridgeLocalServer, BridgePace,
    BridgeRunStatus, BridgeSchedule, BridgeScheduleKind, BridgeUsageLimits, BridgeUsageWindow,
};
use crate::convert;

#[test]
fn servers_and_automations_write_out_hidden_characters_and_keep_their_schedule() {
    let servers = convert::local_servers(vec![LocalServer {
        host: "local\u{202e}host".to_owned(),
        port: 5173,
        url: "http://localhost:5173/".to_owned(),
        process: Some("vi\u{200b}te".to_owned()),
        pid: Some(4242),
        terminal: true,
    }]);
    assert_eq!(
        servers,
        vec![BridgeLocalServer {
            host: "local<U+202E>host".to_owned(),
            port: 5173,
            url: "http://localhost:5173/".to_owned(),
            process: Some("vi<U+200B>te".to_owned()),
            pid: Some(4242),
            terminal: true,
        }]
    );

    let tasks = convert::automations(vec![Automation {
        id: "task-1".to_owned(),
        title: "Nightly\u{202e}check".to_owned(),
        schedule: Schedule {
            kind: ScheduleKind::FixedTime,
            every_ms: None,
            time_of_day: Some("09:30".to_owned()),
            weekdays: vec![1, 3],
            run_at: None,
        },
        enabled: false,
        next_run_ms: Some(1_000),
        last_run: RunStatus::Failed,
        blocked_by_policy: Some("Off\u{202e} by policy".to_owned()),
    }]);
    assert_eq!(
        tasks,
        vec![BridgeAutomation {
            id: "task-1".to_owned(),
            title: "Nightly<U+202E>check".to_owned(),
            schedule: BridgeSchedule {
                kind: BridgeScheduleKind::FixedTime,
                every_ms: None,
                time_of_day: Some("09:30".to_owned()),
                weekdays: vec![1, 3],
                run_at: None,
            },
            enabled: false,
            next_run_ms: Some(1_000),
            last_run: BridgeRunStatus::Failed,
            blocked_by_policy: Some("Off<U+202E> by policy".to_owned()),
        }]
    );
}

#[test]
fn lineage_rows_cross_with_their_relation_group_timing_and_hidden_characters_written_out() {
    let lineage = convert::lineage(Lineage {
        rows: vec![LineageRow {
            thread: "t-sub".to_owned(),
            relation: Relation::Subagent,
            group: Group::Active,
            title: "Read\u{202e}Docs".to_owned(),
            status: Some("running".to_owned()),
            status_label: "Running".to_owned(),
            failed: false,
            started_ms: Some(1_000),
            ended_ms: None,
            merge_target: false,
            stop_subagent: Some("sub-1".to_owned()),
        }],
        running: 1,
        side_chats_running: 0,
        can_disconnect: true,
        merge_target_title: Some("Ro\u{202e}ot".to_owned()),
        merge_ready: true,
        can_merge: false,
    });
    let row = &lineage.rows[0];
    assert_eq!(row.thread_id, "t-sub");
    assert_eq!(row.title, "Read<U+202E>Docs");
    assert_eq!(row.relation, BridgeLineageRelation::Subagent);
    assert_eq!(row.group, BridgeLineageGroup::Active);
    assert_eq!((row.started_ms, row.ended_ms), (Some(1_000), None));
    assert_eq!(row.stop_subagent.as_deref(), Some("sub-1"));
    assert_eq!(lineage.merge_target_title.as_deref(), Some("Ro<U+202E>ot"));
    assert_eq!((lineage.running, lineage.can_disconnect), (1, true));
}

#[test]
fn usage_limits_cross_with_remaining_room_pace_and_their_notice() {
    let limits = convert::usage_limits(UsageLimits {
        notice: None,
        windows: vec![UsageWindow {
            label: "5h".to_owned(),
            remaining_percent: 60,
            resets_at_ms: Some(2_000),
            pace: Some(Pace::Ahead),
        }],
    });
    assert_eq!(
        limits,
        BridgeUsageLimits {
            notice: None,
            windows: vec![BridgeUsageWindow {
                label: "5h".to_owned(),
                remaining_percent: 60,
                resets_at_ms: Some(2_000),
                pace: Some(BridgePace::Ahead),
            }],
        }
    );
    let notice = convert::usage_limits(UsageLimits {
        notice: Some("Token\u{202e} expired".to_owned()),
        windows: Vec::new(),
    });
    assert_eq!(notice.notice.as_deref(), Some("Token<U+202E> expired"));
}
