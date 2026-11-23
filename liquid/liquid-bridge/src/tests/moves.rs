use meta_harness_core::{Arrival, Change, Difference, Direction, Harness, McpServer, MoveCheck};

use crate::api::moves::{
    BridgeArrivalKind, BridgeConfigKind, BridgeDirection, BridgeMovePhase, BridgeMoving,
    arrival_of, config_change, move_check_of, move_in_failure, unchanged,
};

fn instruction(direction: Direction) -> Difference {
    Difference {
        direction,
        change: Change::Instruction {
            name: "AGENTS.md".into(),
            text: Some("rules".into()),
        },
    }
}

#[test]
fn each_change_carries_its_kind_name_and_direction() {
    let server = Difference {
        direction: Direction::Remove,
        change: Change::McpServer {
            name: "old".into(),
            server: None::<McpServer>,
        },
    };

    for (difference, kind, direction) in [
        (
            instruction(Direction::Add),
            BridgeConfigKind::Instruction,
            BridgeDirection::Add,
        ),
        (
            instruction(Direction::Replace),
            BridgeConfigKind::Instruction,
            BridgeDirection::Replace,
        ),
        (server, BridgeConfigKind::McpServer, BridgeDirection::Remove),
    ] {
        let change = config_change(&difference);
        assert_eq!(change.kind, kind);
        assert_eq!(change.direction, direction);
    }
    assert_eq!(
        config_change(&instruction(Direction::Add)).name,
        "AGENTS.md"
    );
}

#[test]
fn a_change_set_that_moved_since_it_was_shown_is_refused_and_an_unchanged_one_passes() {
    let shown = vec![instruction(Direction::Add)];

    assert_eq!(unchanged(&shown, &shown.clone()), Ok(()));
    assert_eq!(unchanged(&[], &[]), Ok(()));
    assert!(
        unchanged(&shown, &[]).is_err(),
        "a change that vanished is refused"
    );
    assert!(
        unchanged(&shown, &[instruction(Direction::Replace)]).is_err(),
        "a change whose direction moved is refused"
    );
}

#[test]
fn a_move_check_says_whether_its_session_has_moved() {
    let check = |moved| MoveCheck {
        title: "Fix".into(),
        model: None,
        harness: Some(Harness::Codex),
        running: false,
        uncommitted_files: 0,
        branch: Some("fix".into()),
        remote_url: None,
        blocked: Some("This session has already moved to another host.".into()),
        moved,
        moving: None,
        agent: None,
    };

    assert!(move_check_of(check(true)).moved);
    assert!(!move_check_of(check(false)).moved);
}

#[test]
fn a_lost_connection_during_move_in_is_not_a_refusal() {
    let lost = move_in_failure("Meta-Harness closed the connection before it answered.".to_owned());
    let refused = move_in_failure(
        "This project has no Git remote that T3 can resolve, so the session cannot move here."
            .to_owned(),
    );

    assert!(lost.lost && lost.thread.is_none());
    assert!(!refused.lost && refused.thread.is_none());
}

#[test]
fn a_move_check_names_the_move_it_is_making() {
    let check: MoveCheck = serde_json::from_value(serde_json::json!({
        "title": "Fix",
        "model": null,
        "harness": "codex",
        "running": false,
        "uncommitted_files": 0,
        "branch": "fix",
        "remote_url": null,
        "blocked": null,
        "moved": false,
        "moving": {
            "transfer": "abc",
            "phase": "handed",
            "target_address": "ws://studio",
            "target_name": "studio",
            "target_environment": "env-studio",
            "since_ms": 1700
        },
    }))
    .expect("a move check");

    assert_eq!(
        move_check_of(check).moving,
        Some(BridgeMoving {
            transfer: "abc".to_owned(),
            phase: BridgeMovePhase::Handed,
            target_address: "ws://studio".to_owned(),
            target_name: "studio".to_owned(),
            target_environment: "env-studio".to_owned(),
            since_ms: 1700,
        })
    );
}

#[test]
fn a_move_status_reports_landed_starting_or_unknown() {
    let landed = arrival_of(Arrival::Landed {
        thread: "thread-1".to_owned(),
    });

    assert_eq!(landed.kind, BridgeArrivalKind::Landed);
    assert_eq!(landed.thread.as_deref(), Some("thread-1"));
    assert_eq!(
        arrival_of(Arrival::Starting).kind,
        BridgeArrivalKind::Starting
    );
    assert_eq!(
        arrival_of(Arrival::Unknown).kind,
        BridgeArrivalKind::Unknown
    );
    assert!(arrival_of(Arrival::Refused).thread.is_none());
}
