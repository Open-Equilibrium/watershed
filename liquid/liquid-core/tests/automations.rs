#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, Attention, AutomationId, AutomationSetup, BlockId, Cell, Changed, Created,
    Error, FieldId, FieldKind, Filter, MAX_AUTOMATION_TEXT, MAX_CONDITIONS, Match, MemoryStore,
    Mutation, Operation, Origin, PersonId, Place, Request, RowId, RowQuery, Step, Trigger,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

struct Budget {
    workspace: Workspace,
    expenses: BlockId,
    months: BlockId,
    item: FieldId,
    amount: FieldId,
    category: FieldId,
    review: liquid_core::OptionId,
    summarize: FieldId,
}

impl Budget {
    fn new() -> Self {
        let mut workspace =
            Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let mut apply = |mutation| {
            workspace.apply(Request {
                actor: owner(),
                origin: Origin::Ui,
                mutation,
            })
        };
        let Some(Created::Area(area)) = apply(Mutation::CreateArea {
            name: "Private".into(),
        })
        .unwrap()
        .created
        else {
            panic!("an area")
        };
        let mut table = |name: &str| match apply(Mutation::CreateTable {
            area: area.clone(),
            name: name.into(),
        })
        .unwrap()
        .created
        {
            Some(Created::Table(table)) => table,
            other => panic!("a Table, got {other:?}"),
        };
        let expenses = table("Expenses");
        let months = table("Months");
        let mut budget = Self {
            workspace,
            item: FieldId::parse(&"0".repeat(32)).unwrap(),
            amount: FieldId::parse(&"0".repeat(32)).unwrap(),
            category: FieldId::parse(&"0".repeat(32)).unwrap(),
            summarize: FieldId::parse(&"0".repeat(32)).unwrap(),
            review: liquid_core::OptionId::parse(&"0".repeat(32)).unwrap(),
            expenses,
            months,
        };
        budget.item = budget
            .workspace
            .database(&budget.expenses)
            .unwrap()
            .unwrap()
            .fields[0]
            .id
            .clone();
        budget.amount = budget.field(&budget.expenses.clone(), "Amount", FieldKind::Number);
        budget.category = budget.field(&budget.expenses.clone(), "Category", FieldKind::Select);
        budget.summarize = budget.field(&budget.months.clone(), "Summarize", FieldKind::Button);
        let add = Mutation::AddOption {
            block: budget.expenses.clone(),
            field: budget.category.clone(),
            name: "Review".into(),
            color: 4,
        };
        budget.review = match budget.edit(add).unwrap().created {
            Some(Created::Option(option)) => option,
            other => panic!("an option, got {other:?}"),
        };
        budget
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        self.workspace.apply(Request {
            actor: owner(),
            origin: Origin::Ui,
            mutation,
        })
    }

    fn field(&mut self, table: &BlockId, name: &str, kind: FieldKind) -> FieldId {
        let add = Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: name.into(),
            kind,
        };
        match self.edit(add).unwrap().created {
            Some(Created::Field(field)) => field,
            other => panic!("a field, got {other:?}"),
        }
    }

    fn row(&mut self, table: &BlockId) -> Applied {
        self.edit(Mutation::AddRow {
            block: table.clone(),
            place: Place::Last,
        })
        .unwrap()
    }

    fn set(&mut self, row: &RowId, field: &FieldId, value: Cell) -> Applied {
        self.edit(Mutation::SetCell {
            block: self.expenses.clone(),
            row: row.clone(),
            field: field.clone(),
            value,
        })
        .unwrap()
    }

    fn create(&mut self, setup: AutomationSetup) -> Result<AutomationId, Error> {
        match self
            .edit(Mutation::CreateAutomation {
                setup: Box::new(setup),
            })?
            .created
        {
            Some(Created::Automation(automation)) => Ok(automation),
            other => panic!("an Automation, got {other:?}"),
        }
    }

    fn flag_large(&self) -> AutomationSetup {
        AutomationSetup {
            name: "Flag large expenses".into(),
            trigger: Some(Trigger::FieldChanged(
                self.expenses.clone(),
                self.amount.clone(),
            )),
            conditions: vec![Filter {
                field: self.amount.clone(),
                matches: Match::Above(Cell::Number(500.0)),
            }],
            steps: vec![
                Step::Set {
                    field: self.category.clone(),
                    value: Cell::Select(self.review.clone()),
                },
                Step::Notify {
                    message: format!("Large expense: {{{}}}", self.item),
                },
            ],
        }
    }

    fn cell(&mut self, table: &BlockId, row: &RowId, field: &FieldId) -> Cell {
        let rows = self.workspace.rows(table, &all()).unwrap();
        rows.rows
            .into_iter()
            .find(|found| &found.id == row)
            .unwrap()
            .cell(field)
            .cloned()
            .unwrap_or(Cell::Empty)
    }

    fn count(&mut self, table: &BlockId) -> usize {
        self.workspace.rows(table, &all()).unwrap().count
    }
}

fn all() -> RowQuery {
    RowQuery {
        sort: None,
        filter: None,
        within: None,
        start: 0,
        limit: usize::MAX,
        search: None,
        totals: Vec::new(),
    }
}

fn created_row(applied: &Applied) -> RowId {
    match &applied.created {
        Some(Created::Row(row)) => row.clone(),
        other => panic!("a row, got {other:?}"),
    }
}

#[test]
fn a_change_that_meets_the_conditions_runs_the_steps_as_the_automation() {
    let mut budget = Budget::new();
    let flag = budget.create(budget.flag_large()).unwrap();
    let rent = created_row(&budget.row(&budget.expenses.clone()));
    budget.set(&rent, &budget.item.clone(), Cell::Text("Rent".into()));

    let small = budget.set(&rent, &budget.amount.clone(), Cell::Number(120.0));
    assert!(small.ran.is_empty());
    assert_eq!(
        budget.cell(&budget.expenses.clone(), &rent, &budget.category.clone()),
        Cell::Empty
    );

    let large = budget.set(&rent, &budget.amount.clone(), Cell::Number(950.0));
    assert_eq!(large.ran.len(), 1);
    let ran = &large.ran[0];
    assert_eq!(ran.automation, flag);
    assert_eq!(ran.notices, ["Large expense: Rent"]);
    assert!(
        ran.changed
            .contains(&Changed::Table(budget.expenses.clone()))
    );
    let action = ran.outcome.clone().unwrap();
    assert_eq!(
        budget.cell(&budget.expenses.clone(), &rent, &budget.category.clone()),
        Cell::Select(budget.review.clone())
    );

    let history = budget.workspace.history(None, 1).unwrap();
    assert_eq!(history[0].id, action);
    assert_eq!(history[0].actor, Actor::Automation(flag.clone()));
    assert_eq!(history[0].origin, Origin::Automation);
    assert_eq!(
        history[0].operation,
        Operation::Run {
            automation: flag.clone(),
            cause: Some(large.action.clone()),
        }
    );
    let automation = budget.workspace.automations().unwrap().remove(0);
    let last = automation.last.unwrap();
    assert_eq!((last.set, last.added, last.notified), (1, 0, 1));
    assert_eq!(automation.owner, Some(budget.expenses.clone()));
}

#[test]
fn a_change_an_automation_makes_starts_no_other_automation() {
    let mut budget = Budget::new();
    budget.create(budget.flag_large()).unwrap();
    let echo = AutomationSetup {
        name: "Echo".into(),
        trigger: Some(Trigger::FieldChanged(
            budget.expenses.clone(),
            budget.category.clone(),
        )),
        conditions: Vec::new(),
        steps: vec![Step::Notify {
            message: "Category changed".into(),
        }],
    };
    budget.create(echo).unwrap();
    let rent = created_row(&budget.row(&budget.expenses.clone()));

    let applied = budget.set(&rent, &budget.amount.clone(), Cell::Number(950.0));
    assert_eq!(applied.ran.len(), 1);
    assert_eq!(applied.ran[0].name, "Flag large expenses");
}

#[test]
fn a_new_row_runs_the_automations_of_its_table_and_a_step_adds_rows_elsewhere() {
    let mut budget = Budget::new();
    let note = AutomationSetup {
        name: "Log new expense".into(),
        trigger: Some(Trigger::RowAdded(budget.expenses.clone())),
        conditions: Vec::new(),
        steps: vec![Step::AddRow {
            table: budget.months.clone(),
            cells: vec![(
                budget
                    .workspace
                    .database(&budget.months)
                    .unwrap()
                    .unwrap()
                    .fields[0]
                    .id
                    .clone(),
                Cell::Text("New expense".into()),
            )],
        }],
    };
    let note = budget.create(note).unwrap();
    assert_eq!(budget.workspace.automations().unwrap()[0].owner, None);

    let applied = budget.row(&budget.expenses.clone());
    assert_eq!(applied.ran.len(), 1);
    assert!(
        applied.ran[0]
            .changed
            .contains(&Changed::Table(budget.months.clone()))
    );
    assert_eq!(budget.count(&budget.months.clone()), 1);
    assert!(budget.row(&budget.months.clone()).ran.is_empty());

    budget
        .edit(Mutation::SetAutomationOn {
            automation: note.clone(),
            on: false,
        })
        .unwrap();
    assert!(budget.row(&budget.expenses.clone()).ran.is_empty());
    budget
        .edit(Mutation::RemoveAutomation { automation: note })
        .unwrap();
    assert!(budget.workspace.automations().unwrap().is_empty());
}

#[test]
fn a_button_field_runs_its_automations_and_holds_no_value() {
    let mut budget = Budget::new();
    let summary = AutomationSetup {
        name: "Monthly summary".into(),
        trigger: Some(Trigger::Clicked(
            budget.months.clone(),
            budget.summarize.clone(),
        )),
        conditions: Vec::new(),
        steps: vec![Step::Notify {
            message: "Summarized".into(),
        }],
    };
    budget.create(summary).unwrap();
    let october = created_row(&budget.row(&budget.months.clone()));

    let ran = budget
        .workspace
        .press(&owner(), &budget.months, &october, &budget.summarize)
        .unwrap();
    assert_eq!(ran.len(), 1);
    assert_eq!(ran[0].notices, ["Summarized"]);
    assert!(ran[0].outcome.is_ok());

    assert_eq!(
        budget
            .workspace
            .press(&owner(), &budget.months, &october, &budget.amount)
            .unwrap_err(),
        Error::NotFound
    );
    let stranger = Actor::Person(PersonId::new("someone"));
    assert_eq!(
        budget
            .workspace
            .press(&stranger, &budget.months, &october, &budget.summarize)
            .unwrap_err(),
        Error::NotAllowed
    );
    let value = Mutation::SetCell {
        block: budget.months.clone(),
        row: october,
        field: budget.summarize.clone(),
        value: Cell::Text("x".into()),
    };
    assert_eq!(budget.edit(value).unwrap_err(), Error::WrongKind);
    let mut on_button = budget.flag_large();
    on_button.trigger = Some(Trigger::RowAdded(budget.months.clone()));
    on_button.conditions = vec![Filter {
        field: budget.summarize.clone(),
        matches: Match::NotEmpty,
    }];
    on_button.steps.clear();
    assert_eq!(budget.create(on_button).unwrap_err(), Error::WrongKind);
}

#[test]
fn test_runs_an_automation_once_on_the_last_row() {
    let mut budget = Budget::new();
    let flag = budget.create(budget.flag_large()).unwrap();
    assert_eq!(
        budget.workspace.test(&owner(), &flag).unwrap_err(),
        Error::NotFound
    );
    let first = created_row(&budget.row(&budget.expenses.clone()));
    let last = created_row(&budget.row(&budget.expenses.clone()));
    budget.set(&first, &budget.amount.clone(), Cell::Number(10.0));
    budget.set(&last, &budget.amount.clone(), Cell::Number(10.0));

    let ran = budget.workspace.test(&owner(), &flag).unwrap();
    assert!(ran.outcome.is_ok());
    assert_eq!(
        budget.cell(&budget.expenses.clone(), &last, &budget.category.clone()),
        Cell::Select(budget.review.clone())
    );
    assert_eq!(
        budget.cell(&budget.expenses.clone(), &first, &budget.category.clone()),
        Cell::Empty
    );
}

#[test]
fn a_setup_must_name_fields_of_the_right_kind_and_stay_within_bounds() {
    let mut budget = Budget::new();
    let formula = budget.field(&budget.expenses.clone(), "Left", FieldKind::Formula);
    let mut broken = Vec::new();
    let mut setup = budget.flag_large();
    setup.trigger = Some(Trigger::Clicked(
        budget.expenses.clone(),
        budget.amount.clone(),
    ));
    broken.push((setup, Error::WrongKind));
    let mut setup = budget.flag_large();
    setup.trigger = Some(Trigger::FieldChanged(
        budget.expenses.clone(),
        formula.clone(),
    ));
    broken.push((setup, Error::WrongKind));
    let mut setup = budget.flag_large();
    setup.steps = vec![Step::Set {
        field: formula,
        value: Cell::Number(1.0),
    }];
    broken.push((setup, Error::WrongKind));
    let mut setup = budget.flag_large();
    setup.steps = vec![Step::Set {
        field: budget.amount.clone(),
        value: Cell::Text("many".into()),
    }];
    broken.push((setup, Error::WrongKind));
    let mut setup = budget.flag_large();
    setup.conditions = vec![Filter {
        field: budget.amount.clone(),
        matches: Match::Above(Cell::Text("x".into())),
    }];
    broken.push((setup, Error::WrongKind));
    let mut setup = budget.flag_large();
    setup.steps = vec![Step::Notify {
        message: format!("{{{}}}", budget.summarize),
    }];
    broken.push((setup, Error::NotFound));
    let mut setup = budget.flag_large();
    setup.conditions = vec![setup.conditions[0].clone(); MAX_CONDITIONS + 1];
    broken.push((setup, Error::TooMany));
    let mut setup = budget.flag_large();
    setup.trigger = None;
    broken.push((setup, Error::NotFound));
    let mut setup = budget.flag_large();
    setup.steps = vec![Step::Set {
        field: budget.item.clone(),
        value: Cell::Text("x".repeat(MAX_AUTOMATION_TEXT + 1)),
    }];
    broken.push((setup, Error::TooLong));
    for (setup, error) in broken {
        let name = format!("{:?}", setup.steps);
        assert_eq!(budget.create(setup).unwrap_err(), error, "{name}");
    }

    let draft = AutomationSetup {
        name: "Draft".into(),
        trigger: None,
        conditions: Vec::new(),
        steps: Vec::new(),
    };
    assert!(budget.create(draft).is_ok());
}

#[test]
fn deleting_a_table_takes_its_own_automations_and_turns_off_those_that_use_it() {
    let mut budget = Budget::new();
    let flag = budget.create(budget.flag_large()).unwrap();
    let summary = AutomationSetup {
        name: "Monthly summary".into(),
        trigger: Some(Trigger::Clicked(
            budget.months.clone(),
            budget.summarize.clone(),
        )),
        conditions: Vec::new(),
        steps: vec![Step::AddRow {
            table: budget.expenses.clone(),
            cells: Vec::new(),
        }],
    };
    let summary = budget.create(summary).unwrap();

    let deleted = budget
        .edit(Mutation::DeleteTable {
            block: budget.expenses.clone(),
        })
        .unwrap();
    assert!(deleted.changed.contains(&Changed::Automations));
    let left = budget.workspace.automations().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, summary);
    assert!(!left[0].on);
    assert_eq!(
        left[0].attention,
        Some(Attention::TableDeleted {
            name: "Expenses".into()
        })
    );
    assert!(!left.iter().any(|automation| automation.id == flag));
    let on = Mutation::SetAutomationOn {
        automation: summary.clone(),
        on: true,
    };
    assert_eq!(budget.edit(on.clone()).unwrap_err(), Error::NotAllowed);

    let mut repaired = left[0].setup.clone();
    repaired.steps = vec![Step::Notify {
        message: "Summarized".into(),
    }];
    budget
        .edit(Mutation::SetAutomation {
            automation: summary.clone(),
            setup: Box::new(repaired),
        })
        .unwrap();
    assert_eq!(budget.workspace.automations().unwrap()[0].attention, None);
    budget.edit(on).unwrap();
    assert!(budget.workspace.automations().unwrap()[0].on);
}

#[test]
fn removing_a_field_an_automation_uses_turns_it_off() {
    let mut budget = Budget::new();
    budget.create(budget.flag_large()).unwrap();
    budget
        .edit(Mutation::RemoveField {
            block: budget.expenses.clone(),
            field: budget.category.clone(),
        })
        .unwrap();
    let automation = budget.workspace.automations().unwrap().remove(0);
    assert!(!automation.on);
    assert_eq!(
        automation.attention,
        Some(Attention::FieldRemoved {
            table: "Expenses".into(),
            field: "Category".into(),
        })
    );
    let rent = created_row(&budget.row(&budget.expenses.clone()));
    assert!(
        budget
            .set(&rent, &budget.amount.clone(), Cell::Number(950.0))
            .ran
            .is_empty()
    );
}

#[test]
fn no_one_writes_as_an_automation_from_outside() {
    let mut budget = Budget::new();
    let flag = budget.create(budget.flag_large()).unwrap();
    let forged = budget.workspace.apply(Request {
        actor: Actor::Automation(flag),
        origin: Origin::Automation,
        mutation: Mutation::AddRow {
            block: budget.expenses.clone(),
            place: Place::Last,
        },
    });
    assert_eq!(forged.unwrap_err(), Error::NotAllowed);
}

#[test]
fn conditions_compare_numbers_and_dates_and_rule_out_values() {
    let mut budget = Budget::new();
    let due = budget.field(&budget.expenses.clone(), "Due", FieldKind::Date);
    let setup = AutomationSetup {
        name: "Early and small".into(),
        trigger: Some(Trigger::FieldChanged(budget.expenses.clone(), due.clone())),
        conditions: vec![
            Filter {
                field: due.clone(),
                matches: Match::Below(Cell::Date("2026-11-01".into())),
            },
            Filter {
                field: budget.amount.clone(),
                matches: Match::Below(Cell::Number(100.0)),
            },
            Filter {
                field: budget.category.clone(),
                matches: Match::IsNot(Cell::Select(budget.review.clone())),
            },
        ],
        steps: vec![Step::Notify {
            message: format!("{{{}}} is due on {{{due}}}", budget.amount),
        }],
    };
    budget.create(setup).unwrap();
    let row = created_row(&budget.row(&budget.expenses.clone()));
    let due_on = |budget: &mut Budget, date: &str| {
        budget
            .edit(Mutation::SetCell {
                block: budget.expenses.clone(),
                row: row.clone(),
                field: due.clone(),
                value: Cell::Date(date.into()),
            })
            .unwrap()
            .ran
    };
    assert!(due_on(&mut budget, "2026-10-01").is_empty());
    budget.set(&row, &budget.amount.clone(), Cell::Number(42.5));
    assert!(due_on(&mut budget, "2026-12-01").is_empty());
    assert_eq!(
        due_on(&mut budget, "2026-10-02")[0].notices,
        ["42.5 is due on 2026-10-02"]
    );
    budget.set(
        &row,
        &budget.category.clone(),
        Cell::Select(budget.review.clone()),
    );
    assert!(due_on(&mut budget, "2026-10-03").is_empty());
}
