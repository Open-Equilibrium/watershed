#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Allows, Applied, AutomationId, AutomationSetup, BlockId, Capability, Cell, Created,
    Error, FieldId, FieldKind, Grant, MemoryStore, Mutation, Origin, Permission, PersonId, Place,
    Request, RoleId, RoleSetup, RowQuery, Step, Target, Template, Trigger,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

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
    area: liquid_core::AreaId,
    expenses: BlockId,
    bills: BlockId,
    amount: FieldId,
}

impl Budget {
    fn new() -> Self {
        let workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let mut budget = Self {
            workspace,
            area: liquid_core::AreaId::parse(&"0".repeat(32)).unwrap(),
            expenses: BlockId::parse(&"0".repeat(32)).unwrap(),
            bills: BlockId::parse(&"0".repeat(32)).unwrap(),
            amount: FieldId::parse(&"0".repeat(32)).unwrap(),
        };
        let Some(Created::Area(area)) = budget
            .edit(Mutation::CreateArea {
                name: "Private".into(),
            })
            .unwrap()
            .created
        else {
            panic!("an area")
        };
        budget.area = area;
        budget.expenses = budget.table("Expenses");
        budget.bills = budget.table("Bills");
        budget.amount = match budget
            .edit(Mutation::AddField {
                block: budget.expenses.clone(),
                place: Place::Last,
                name: "Amount".into(),
                kind: FieldKind::Number,
            })
            .unwrap()
            .created
        {
            Some(Created::Field(field)) => field,
            other => panic!("a field, got {other:?}"),
        };
        budget
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        self.workspace.apply(Request {
            actor: Actor::Person(PersonId::new("benni")),
            origin: Origin::Ui,
            mutation,
        })
    }

    fn table(&mut self, name: &str) -> BlockId {
        match self
            .edit(Mutation::CreateTable {
                area: self.area.clone(),
                name: name.into(),
            })
            .unwrap()
            .created
        {
            Some(Created::Table(table)) => table,
            other => panic!("a Table, got {other:?}"),
        }
    }

    fn role(&mut self, name: &str, grants: Vec<Grant>) -> Result<RoleId, Error> {
        match self
            .edit(Mutation::CreateRole {
                setup: RoleSetup {
                    name: name.into(),
                    grants,
                },
            })?
            .created
        {
            Some(Created::Role(role)) => Ok(role),
            other => panic!("a Role, got {other:?}"),
        }
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
        match self
            .edit(Mutation::CreateAutomation {
                setup: Box::new(setup),
            })
            .unwrap()
            .created
        {
            Some(Created::Automation(automation)) => automation,
            other => panic!("an Automation, got {other:?}"),
        }
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
        let all = RowQuery {
            sort: None,
            filter: None,
            within: None,
            start: 0,
            limit: usize::MAX,
            search: None,
            totals: Vec::new(),
        };
        self.workspace
            .rows(&self.expenses, &all)
            .unwrap()
            .rows
            .iter()
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
    let editor = budget
        .role(
            "Budget editor",
            vec![Grant {
                target: Target::Table(budget.expenses.clone()),
                allows: EDIT,
            }],
        )
        .unwrap();
    let (expenses, bills) = (budget.expenses.clone(), budget.bills.clone());
    assert_eq!(budget.allows(&editor, &expenses), EDIT);
    assert_eq!(budget.allows(&editor, &bills), Allows::default());

    let round = budget.setter();
    budget.assign(&round, &editor).unwrap();
    let ran = budget.added().ran.remove(0);
    let action = ran.outcome.unwrap();
    assert_eq!(budget.amounts(), [Cell::Number(1.0)]);
    let history = budget.workspace.history(None, 1).unwrap();
    assert_eq!(history[0].id, action);
    assert_eq!(history[0].permission, Permission::Role(editor));

    budget.assign(&round, &Template::CanView.id()).unwrap();
    let refused = budget.added().ran.remove(0);
    assert_eq!(
        refused.outcome,
        Err(Error::Denied {
            capability: Capability::Apply,
            table: "Expenses".into(),
        })
    );
    assert_eq!(budget.amounts(), [Cell::Number(1.0), Cell::Empty]);
}

#[test]
fn a_protected_table_keeps_only_the_roles_chosen_for_it_and_reading_follows_its_area() {
    let mut budget = Budget::new();
    let editor = budget.role("Budget editor", Vec::new()).unwrap();
    let expenses = budget.expenses.clone();
    budget.setter();
    budget
        .edit(Mutation::ProtectTable {
            table: expenses.clone(),
            roles: vec![editor.clone(), Template::CanEdit.id()],
        })
        .unwrap();
    assert!(
        budget
            .workspace
            .tables()
            .unwrap()
            .iter()
            .find(|table| table.id == expenses)
            .unwrap()
            .protected
    );
    assert_eq!(budget.allows(&Template::CanEdit.id(), &expenses), EDIT);
    assert_eq!(budget.allows(&editor, &expenses), Allows::default());
    assert!(budget.allows(&Template::CanManage.id(), &expenses).manage);

    budget
        .edit(Mutation::ProtectTable {
            table: expenses.clone(),
            roles: Vec::new(),
        })
        .unwrap();
    assert_eq!(budget.allows(&Template::CanEdit.id(), &expenses), READ);
    assert!(budget.added().ran[0].outcome.is_err());

    budget
        .edit(Mutation::UnprotectTable {
            table: expenses.clone(),
        })
        .unwrap();
    assert_eq!(budget.allows(&Template::CanEdit.id(), &expenses), EDIT);
    assert!(budget.added().ran[0].outcome.is_ok());
}

#[test]
fn templates_and_assigned_roles_hold_and_setups_name_what_exists() {
    let mut budget = Budget::new();
    let manage = Mutation::SetRole {
        role: Template::CanManage.id(),
        setup: RoleSetup {
            name: "Everything".into(),
            grants: Vec::new(),
        },
    };
    assert_eq!(budget.edit(manage).unwrap_err(), Error::NotAllowed);
    budget
        .edit(Mutation::SetRole {
            role: Template::CanView.id(),
            setup: RoleSetup {
                name: "Readers".into(),
                grants: vec![Grant {
                    target: Target::Area(budget.area.clone()),
                    allows: READ,
                }],
            },
        })
        .unwrap();
    assert_eq!(budget.workspace.roles().unwrap()[2].setup.name, "Readers");
    let remove = |role: RoleId| Mutation::RemoveRole { role };
    assert_eq!(
        budget.edit(remove(Template::CanView.id())).unwrap_err(),
        Error::NotAllowed
    );

    let editor = budget.role("Budget editor", Vec::new()).unwrap();
    let round = budget.setter();
    budget.assign(&round, &editor).unwrap();
    assert_eq!(
        budget.edit(remove(editor.clone())).unwrap_err(),
        Error::NotAllowed
    );
    budget.assign(&round, &Template::CanEdit.id()).unwrap();
    budget.edit(remove(editor.clone())).unwrap();
    assert_eq!(budget.workspace.roles().unwrap().len(), 3);
    assert_eq!(budget.assign(&round, &editor).unwrap_err(), Error::NotFound);

    let missing = BlockId::parse(&"e".repeat(32)).unwrap();
    let grant = Grant {
        target: Target::Table(missing),
        allows: READ,
    };
    assert_eq!(
        budget.role("Ghost", vec![grant]).unwrap_err(),
        Error::NotFound
    );
    assert_eq!(
        budget
            .role(&"x".repeat(liquid_core::MAX_ROLE_NAME + 1), Vec::new())
            .unwrap_err(),
        Error::TooLong
    );
}

#[test]
fn rows_added_by_people_need_no_role() {
    let mut budget = Budget::new();
    let expenses = budget.expenses.clone();
    budget
        .edit(Mutation::ProtectTable {
            table: expenses,
            roles: Vec::new(),
        })
        .unwrap();
    let added = budget.added();
    assert!(matches!(added.created, Some(Created::Row(_))));
}
