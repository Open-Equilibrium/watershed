use std::sync::{Arc, Mutex};

use super::workspace::{Folder, Memory, lisbon, mutation, query, resource};
use crate::api::automation::{
    BridgeAttentionKind, BridgeAutomationSetup, BridgeStep, BridgeStepKind, BridgeTrigger,
    BridgeTriggerKind,
};
use crate::api::database::{
    BridgeCell, BridgeCellKind, BridgeFieldKind, BridgeFilter, BridgeMatch,
};
use crate::api::replica::LiquidWorkspace;
use crate::api::workspace::{
    BridgeMutation, BridgeMutationKind, BridgeQueryKind, BridgeResourceKind,
};

fn number(value: f64) -> BridgeCell {
    BridgeCell {
        kind: BridgeCellKind::Number,
        text: None,
        number: value,
        checked: false,
        files: Vec::new(),
    }
}

fn text(value: &str) -> BridgeCell {
    BridgeCell {
        kind: BridgeCellKind::Text,
        text: Some(value.into()),
        ..number(0.0)
    }
}

fn step(kind: BridgeStepKind) -> BridgeStep {
    BridgeStep {
        kind,
        field: None,
        value: None,
        table: None,
        cells: Vec::new(),
        message: None,
    }
}

#[test]
fn automations_cross_the_bridge_and_report_their_runs() {
    let folder = Folder::new("automation");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, _, view) = lisbon(&workspace);
    let table = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(view),
            text: Some("Expenses".into()),
            ..mutation(BridgeMutationKind::AddDatabaseBlock)
        })
        .unwrap()
        .block
        .unwrap();
    let edit = |write: BridgeMutation| {
        workspace.apply(BridgeMutation {
            block: Some(table.clone()),
            ..write
        })
    };
    let add_field = |name: &str, kind| {
        edit(BridgeMutation {
            last: true,
            text: Some(name.into()),
            field_kind: Some(kind),
            ..mutation(BridgeMutationKind::AddField)
        })
        .unwrap()
        .field
        .unwrap()
    };
    let amount = add_field("Amount", BridgeFieldKind::Number);
    let flag = add_field("Flag", BridgeFieldKind::Button);
    let read = || {
        workspace
            .query(query(BridgeQueryKind::Automations))
            .unwrap()
            .automations
    };
    let name = workspace
        .query(crate::api::workspace::BridgeQuery {
            block: Some(table.clone()),
            ..query(BridgeQueryKind::Database)
        })
        .unwrap()
        .database
        .unwrap()
        .fields[0]
        .id
        .clone();

    let changes = Arc::new(Mutex::new(0));
    let counted = changes.clone();
    workspace
        .watch(1, resource(BridgeResourceKind::Automations), move |_| {
            *counted.lock().unwrap() += 1;
            true
        })
        .unwrap();
    let before = *changes.lock().unwrap();
    let setup = BridgeAutomationSetup {
        name: "Flag large expenses".into(),
        trigger: Some(BridgeTrigger {
            kind: BridgeTriggerKind::FieldChanged,
            table: table.clone(),
            field: Some(amount.clone()),
        }),
        conditions: vec![BridgeFilter {
            field: amount.clone(),
            matches: BridgeMatch::Above,
            value: Some(number(500.0)),
            until: None,
        }],
        steps: vec![BridgeStep {
            message: Some(format!("Large expense: {{{name}}}")),
            ..step(BridgeStepKind::Notify)
        }],
    };
    let automation = workspace
        .apply(BridgeMutation {
            automation_setup: Some(setup.clone()),
            ..mutation(BridgeMutationKind::CreateAutomation)
        })
        .unwrap()
        .automation
        .unwrap();
    assert_eq!(*changes.lock().unwrap(), before + 1);
    let listed = read();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, automation);
    assert_eq!(listed[0].setup, setup);
    assert!(listed[0].on);
    assert_eq!(listed[0].owner, Some(table.clone()));

    let row = edit(BridgeMutation {
        last: true,
        ..mutation(BridgeMutationKind::AddRow)
    })
    .unwrap()
    .row
    .unwrap();
    let set = |field: &str, value| {
        edit(BridgeMutation {
            row: Some(row.clone()),
            field: Some(field.into()),
            cell: Some(value),
            ..mutation(BridgeMutationKind::SetCell)
        })
        .unwrap()
    };
    set(&name, text("Rent"));
    let applied = set(&amount, number(950.0));
    assert_eq!(applied.ran.len(), 1);
    assert_eq!(applied.ran[0].name, "Flag large expenses");
    assert_eq!(applied.ran[0].notices, ["Large expense: Rent"]);
    assert_eq!(applied.ran[0].failure, None);
    assert_eq!(read()[0].last.as_ref().unwrap().notified, 1);

    let pressed = BridgeAutomationSetup {
        name: "Pressed".into(),
        trigger: Some(BridgeTrigger {
            kind: BridgeTriggerKind::Clicked,
            table: table.clone(),
            field: Some(flag.clone()),
        }),
        conditions: Vec::new(),
        steps: vec![BridgeStep {
            field: Some(amount.clone()),
            value: Some(number(1.0)),
            ..step(BridgeStepKind::Set)
        }],
    };
    workspace
        .apply(BridgeMutation {
            automation: Some(automation.clone()),
            automation_setup: Some(pressed),
            ..mutation(BridgeMutationKind::SetAutomation)
        })
        .unwrap();
    let ran = workspace
        .press(table.clone(), row.clone(), flag.clone())
        .unwrap();
    assert_eq!(ran.len(), 1);
    assert!(ran[0].notices.is_empty());
    let ran = workspace.test(automation.clone()).unwrap();
    assert_eq!(ran.failure, None);
    assert!(workspace.press(table.clone(), row, amount).is_err());

    workspace
        .apply(BridgeMutation {
            automation: Some(automation.clone()),
            on: false,
            ..mutation(BridgeMutationKind::SetAutomationOn)
        })
        .unwrap();
    assert!(!read()[0].on);
    edit(BridgeMutation {
        field: Some(flag),
        ..mutation(BridgeMutationKind::RemoveField)
    })
    .unwrap();
    let attention = read()[0].attention.clone().unwrap();
    assert_eq!(attention.kind, BridgeAttentionKind::FieldRemoved);
    assert_eq!(
        (attention.table.as_str(), attention.field.as_deref()),
        ("Expenses", Some("Flag"))
    );
    workspace
        .apply(BridgeMutation {
            automation: Some(automation),
            ..mutation(BridgeMutationKind::RemoveAutomation)
        })
        .unwrap();
    assert!(read().is_empty());
}
