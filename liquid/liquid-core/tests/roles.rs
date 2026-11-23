#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Allows, Applied, AreaId, AutomationId, AutomationSetup, BlockId, Capability, Cell, Created,
    Error, FieldId, FieldKind, Grant, MAX_ROLE_NAME, Mutation, Permission, Place, RoleId,
    RoleSetup, Step, Target, Template, Trigger,
};

const READ: Allows = Allows {
    read: true,
    propose: false,
    apply: false,
    approve: false,
    manage: false,
};

const EDIT: Allows = Allows {
    apply: true,
    ..READ
};

struct Budget {
    workspace: Workspace,
    area: AreaId,
    expenses: BlockId,
    bills: BlockId,
    amount: FieldId,
}

fn table(workspace: &mut Workspace, area: &AreaId, name: &str) -> BlockId {
    let Created::Table(table) = created(
        workspace,
        Mutation::CreateTable {
            area: area.clone(),
            name: name.into(),
        },
    ) else {
        panic!("a Table")
    };
    table
}

impl Budget {
    fn new() -> Self {
        let mut workspace = workspace();
        let Created::Area(area) = created(
            &mut workspace,
            Mutation::CreateArea {
                name: "Private".into(),
            },
        ) else {
            panic!("an area")
        };
        let expenses = table(&mut workspace, &area, "Expenses");
        let bills = table(&mut workspace, &area, "Bills");
        let Created::Field(amount) = created(
            &mut workspace,
            Mutation::AddField {
                block: expenses.clone(),
                place: Place::Last,
                name: "Amount".into(),
                kind: FieldKind::Number,
            },
        ) else {
            panic!("a field")
        };
        Self {
            workspace,
            area,
            expenses,
            bills,
            amount,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn role(&mut self, name: &str, grants: Vec<Grant>) -> Result<RoleId, Error> {
        let setup = RoleSetup {
            name: name.into(),
            grants,
        };
        match self.edit(Mutation::CreateRole { setup })?.created {
            Some(Created::Role(role)) => Ok(role),
            other => panic!("a Role, got {other:?}"),
        }
    }

    fn remove_role(&mut self, role: RoleId) -> Result<Applied, Error> {
        self.edit(Mutation::RemoveRole { role })
    }

    fn set_role(&mut self, role: RoleId, name: &str, grants: Vec<Grant>) -> Result<Applied, Error> {
        let setup = RoleSetup {
            name: name.into(),
            grants,
        };
        self.edit(Mutation::SetRole { role, setup })
    }

    fn protect(&mut self, table: &BlockId, roles: Vec<RoleId>) -> Result<Applied, Error> {
        self.edit(Mutation::ProtectTable {
            table: table.clone(),
            roles,
        })
    }

    fn unprotect(&mut self, table: &BlockId) -> Result<Applied, Error> {
        self.edit(Mutation::UnprotectTable {
            table: table.clone(),
        })
    }

    fn protected(&mut self, table: &BlockId) -> bool {
        let tables = self.workspace.tables().unwrap();
        tables
            .iter()
            .any(|entry| entry.id == *table && entry.protected)
    }

    fn allows(&mut self, role: &RoleId, table: &BlockId) -> Allows {
        self.workspace.allows(role, table).unwrap()
    }

    fn setter(&mut self) -> AutomationId {
        let setup = AutomationSetup {
            name: "Round".into(),
            trigger: Some(Trigger::RowAdded(self.expenses.clone())),
            conditions: Vec::new(),
            steps: vec![Step::Set {
                field: self.amount.clone(),
                value: Cell::Number(1.0),
            }],
        };
        let Created::Automation(automation) = created(
            &mut self.workspace,
            Mutation::CreateAutomation {
                setup: Box::new(setup),
            },
        ) else {
            panic!("an Automation")
        };
        automation
    }

    fn assign(&mut self, automation: &AutomationId, role: &RoleId) -> Result<Applied, Error> {
        self.edit(Mutation::SetAutomationRole {
            automation: automation.clone(),
            role: role.clone(),
        })
    }

    fn added(&mut self) -> Applied {
        self.edit(Mutation::AddRow {
            block: self.expenses.clone(),
            place: Place::Last,
        })
        .unwrap()
    }

    fn amounts(&mut self) -> Vec<Cell> {
        let rows = self.workspace.rows(&self.expenses, &all()).unwrap().rows;
        rows.iter()
            .map(|row| row.cell(&self.amount).cloned().unwrap_or(Cell::Empty))
            .collect()
    }
}

#[test]
fn a_workspace_starts_with_three_templates_and_an_automation_runs_as_can_edit() {
    let mut budget = Budget::new();
    let roles = budget.workspace.roles().unwrap();
    let named: Vec<_> = roles
        .iter()
        .map(|role| (role.setup.name.as_str(), role.template))
        .collect();
    assert_eq!(
        named,
        [
            ("Can manage", Some(Template::CanManage)),
            ("Can edit", Some(Template::CanEdit)),
            ("Can view", Some(Template::CanView)),
        ]
    );
    let expenses = budget.expenses.clone();
    assert_eq!(budget.allows(&Template::CanView.id(), &expenses), READ);
    assert_eq!(budget.allows(&Template::CanEdit.id(), &expenses), EDIT);
    assert!(budget.allows(&Template::CanManage.id(), &expenses).manage);

    budget.setter();
    let automation = budget.workspace.automations().unwrap().remove(0);
    assert_eq!(automation.role, Template::CanEdit.id());
}

#[test]
fn a_role_allows_only_what_it_lists_and_a_run_needs_apply_where_it_writes() {
    let mut budget = Budget::new();
    let grant = Grant {
        target: Target::Table(budget.expenses.clone()),
        allows: EDIT,
    };
    let editor = budget.role("Budget editor", vec![grant]).unwrap();
    let (expenses, bills) = (budget.expenses.clone(), budget.bills.clone());
    assert_eq!(budget.allows(&editor, &expenses), EDIT);
    assert_eq!(budget.allows(&editor, &bills), Allows::default());

    let round = budget.setter();
    budget.assign(&round, &editor).unwrap();
    let action = budget.added().ran.remove(0).outcome.unwrap();
    assert_eq!(budget.amounts(), [Cell::Number(1.0)]);
    let history = budget.workspace.history(None, 1).unwrap();
    assert_eq!(history[0].id, action);
    assert_eq!(history[0].permission, Permission::Role(editor));

    budget.assign(&round, &Template::CanView.id()).unwrap();
    let refused = budget.added().ran.remove(0).outcome;
    assert_eq!(
        refused,
        Err(Error::Denied {
            capability: Capability::Apply,
            table: "Expenses".into(),
        })
    );
    assert_eq!(budget.amounts(), [Cell::Number(1.0), Cell::Empty]);
}

#[test]
fn a_protected_table_keeps_only_its_chosen_roles_reads_follow_its_area_and_people_add_rows() {
    let mut budget = Budget::new();
    let editor = budget.role("Budget editor", Vec::new()).unwrap();
    let expenses = budget.expenses.clone();
    budget.setter();
    let roles = vec![editor.clone(), Template::CanEdit.id()];
    budget.protect(&expenses, roles).unwrap();
    assert!(budget.protected(&expenses));
    assert_eq!(budget.allows(&Template::CanEdit.id(), &expenses), EDIT);
    assert_eq!(budget.allows(&editor, &expenses), Allows::default());
    assert!(budget.allows(&Template::CanManage.id(), &expenses).manage);

    budget.protect(&expenses, Vec::new()).unwrap();
    assert_eq!(budget.allows(&Template::CanEdit.id(), &expenses), READ);
    let added = budget.added();
    assert!(matches!(added.created, Some(Created::Row(_))));
    assert!(added.ran[0].outcome.is_err());

    budget.unprotect(&expenses).unwrap();
    assert_eq!(budget.allows(&Template::CanEdit.id(), &expenses), EDIT);
    assert!(budget.added().ran[0].outcome.is_ok());
}

#[test]
fn templates_and_assigned_roles_hold_and_setups_name_what_exists() {
    let mut budget = Budget::new();
    let everything = budget.set_role(Template::CanManage.id(), "Everything", Vec::new());
    assert_eq!(everything.unwrap_err(), Error::NotAllowed);
    let grant = Grant {
        target: Target::Area(budget.area.clone()),
        allows: READ,
    };
    let readers = budget.set_role(Template::CanView.id(), "Readers", vec![grant]);
    readers.unwrap();
    assert_eq!(budget.workspace.roles().unwrap()[2].setup.name, "Readers");
    let refused = budget.remove_role(Template::CanView.id()).unwrap_err();
    assert_eq!(refused, Error::NotAllowed);

    let editor = budget.role("Budget editor", Vec::new()).unwrap();
    let round = budget.setter();
    budget.assign(&round, &editor).unwrap();
    let refused = budget.remove_role(editor.clone()).unwrap_err();
    assert_eq!(refused, Error::NotAllowed);
    budget.assign(&round, &Template::CanEdit.id()).unwrap();
    budget.remove_role(editor.clone()).unwrap();
    assert_eq!(budget.workspace.roles().unwrap().len(), 3);
    assert_eq!(budget.assign(&round, &editor).unwrap_err(), Error::NotFound);

    let ghost = Grant {
        target: Target::Table(BlockId::parse(&"e".repeat(32)).unwrap()),
        allows: READ,
    };
    let refused = budget.role("Ghost", vec![ghost]).unwrap_err();
    assert_eq!(refused, Error::NotFound);
    let long_name = "x".repeat(MAX_ROLE_NAME + 1);
    let refused = budget.role(&long_name, Vec::new()).unwrap_err();
    assert_eq!(refused, Error::TooLong);
}
