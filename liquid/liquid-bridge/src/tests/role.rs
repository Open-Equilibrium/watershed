use super::workspace::{mutation, open, query};
use crate::api::automation::{
    BridgeAutomationSetup, BridgeStep, BridgeStepKind, BridgeTrigger, BridgeTriggerKind,
};
use crate::api::database::{BridgeCell, BridgeCellKind, BridgeFieldKind};
use crate::api::replica::LiquidWorkspace;
use crate::api::role::{
    BridgeAllows, BridgeGrant, BridgeRoleSetup, BridgeTargetKind, BridgeTemplate,
};
use crate::api::workspace::{BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind};

const EDIT: BridgeAllows = BridgeAllows {
    read: true,
    propose: false,
    apply: true,
    approve: false,
    manage: false,
};

#[test]
fn roles_protection_and_refused_runs_cross_the_bridge() {
    let (_folder, workspace) = open("roles");
    let area = workspace
        .apply(BridgeMutation {
            text: Some("Private".into()),
            ..mutation(BridgeMutationKind::CreateArea)
        })
        .unwrap()
        .area
        .unwrap();
    let table = workspace
        .apply(BridgeMutation {
            area: Some(area.clone()),
            text: Some("Expenses".into()),
            ..mutation(BridgeMutationKind::CreateTable)
        })
        .unwrap()
        .block
        .unwrap();
    let amount = workspace
        .apply(BridgeMutation {
            block: Some(table.clone()),
            last: true,
            text: Some("Amount".into()),
            field_kind: Some(BridgeFieldKind::Number),
            ..mutation(BridgeMutationKind::AddField)
        })
        .unwrap()
        .field
        .unwrap();

    let roles = workspace
        .query(query(BridgeQueryKind::Roles))
        .unwrap()
        .roles;
    assert_eq!(
        roles.iter().map(|role| role.template).collect::<Vec<_>>(),
        [
            Some(BridgeTemplate::CanManage),
            Some(BridgeTemplate::CanEdit),
            Some(BridgeTemplate::CanView),
        ]
    );
    let setup = BridgeRoleSetup {
        name: "Budget editor".into(),
        grants: vec![BridgeGrant {
            kind: BridgeTargetKind::Area,
            id: Some(area.clone()),
            allows: EDIT,
        }],
    };
    let editor = workspace
        .apply(BridgeMutation {
            role_setup: Some(setup.clone()),
            ..mutation(BridgeMutationKind::CreateRole)
        })
        .unwrap()
        .role
        .unwrap();
    let roles = workspace
        .query(query(BridgeQueryKind::Roles))
        .unwrap()
        .roles;
    assert_eq!(roles[3].id, editor);
    assert_eq!(roles[3].setup, setup);
    let allows = |workspace: &LiquidWorkspace| {
        workspace
            .query(BridgeQuery {
                block: Some(table.clone()),
                ..query(BridgeQueryKind::Allows)
            })
            .unwrap()
            .allows
    };
    let on_table = allows(&workspace);
    assert_eq!(on_table.len(), 4);
    assert_eq!(on_table[3].role, editor);
    assert_eq!(on_table[3].allows, EDIT);

    let flag = BridgeAutomationSetup {
        name: "Round".into(),
        trigger: Some(BridgeTrigger {
            kind: BridgeTriggerKind::RowAdded,
            table: table.clone(),
            field: None,
        }),
        conditions: Vec::new(),
        steps: vec![BridgeStep {
            kind: BridgeStepKind::Set,
            field: Some(amount),
            value: Some(BridgeCell {
                kind: BridgeCellKind::Number,
                text: None,
                number: 1.0,
                checked: false,
                files: Vec::new(),
            }),
            table: None,
            cells: Vec::new(),
            message: None,
        }],
    };
    let automation = workspace
        .apply(BridgeMutation {
            automation_setup: Some(flag),
            ..mutation(BridgeMutationKind::CreateAutomation)
        })
        .unwrap()
        .automation
        .unwrap();
    workspace
        .apply(BridgeMutation {
            automation: Some(automation),
            role: Some(editor.clone()),
            ..mutation(BridgeMutationKind::SetAutomationRole)
        })
        .unwrap();
    let listed = workspace
        .query(query(BridgeQueryKind::Automations))
        .unwrap()
        .automations;
    assert_eq!(listed[0].role, editor);

    workspace
        .apply(BridgeMutation {
            block: Some(table.clone()),
            roles: Vec::new(),
            ..mutation(BridgeMutationKind::ProtectTable)
        })
        .unwrap();
    let tables = workspace
        .query(query(BridgeQueryKind::Tables))
        .unwrap()
        .tables;
    assert!(tables[0].protected);
    assert!(!allows(&workspace)[3].allows.apply);
    let added = workspace
        .apply(BridgeMutation {
            block: Some(table.clone()),
            last: true,
            ..mutation(BridgeMutationKind::AddRow)
        })
        .unwrap();
    assert_eq!(
        added.ran[0].failure.as_deref(),
        Some("its Role allows no Apply on “Expenses”")
    );

    workspace
        .apply(BridgeMutation {
            block: Some(table.clone()),
            ..mutation(BridgeMutationKind::UnprotectTable)
        })
        .unwrap();
    assert!(allows(&workspace)[3].allows.apply);
    assert!(
        workspace
            .apply(BridgeMutation {
                role: Some(editor),
                ..mutation(BridgeMutationKind::RemoveRole)
            })
            .is_err()
    );
}
