#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Actor, Applied, Attention, AutomationId, AutomationSetup, BlockId, Cell, Changed, Created,
    Error, FieldId, FieldKind, Filter, MAX_AUTOMATION_TEXT, MAX_CONDITIONS, Match, Mutation,
    Operation, OptionId, Origin, PersonId, Place, Request, RowId, Step, Trigger,
};

fn field(workspace: &mut Workspace, table: &BlockId, name: &str, kind: FieldKind) -> FieldId {
    let Created::Field(field) = created(
        workspace,
        Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: name.into(),
            kind,
        },
    ) else {
        panic!("a field")
    };
    field
}

fn setup(name: &str, trigger: Trigger, steps: Vec<Step>) -> AutomationSetup {
    AutomationSetup {
        name: name.into(),
        trigger: Some(trigger),
        conditions: Vec::new(),
        steps,
    }
}

fn notify(message: &str) -> Step {
    Step::Notify {
        message: message.into(),
    }
}

fn created_row(applied: &Applied) -> RowId {
    match &applied.created {
        Some(Created::Row(row)) => row.clone(),
        other => panic!("a row, got {other:?}"),
    }
}

struct Budget {
    workspace: Workspace,
    expenses: BlockId,
    months: BlockId,
    item: FieldId,
    amount: FieldId,
    category: FieldId,
    review: OptionId,
    month_name: FieldId,
    summarize: FieldId,
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
        let table = |workspace: &mut Workspace, name: &str| {
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
        };
        let expenses = table(&mut workspace, "Expenses");
        let months = table(&mut workspace, "Months");
        let item = workspace.database(&expenses).unwrap().unwrap().fields[0]
            .id
            .clone();
        let month_name = workspace.database(&months).unwrap().unwrap().fields[0]
            .id
            .clone();
        let amount = field(&mut workspace, &expenses, "Amount", FieldKind::Number);
        let category = field(&mut workspace, &expenses, "Category", FieldKind::Select);
        let summarize = field(&mut workspace, &months, "Summarize", FieldKind::Button);
        let Created::Option(review) = created(
            &mut workspace,
            Mutation::AddOption {
                block: expenses.clone(),
                field: category.clone(),
                name: "Review".into(),
                color: 4,
            },
        ) else {
            panic!("an option")
        };
        Self {
            workspace,
            expenses,
            months,
            item,
            amount,
            category,
            review,
            month_name,
            summarize,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn add_row(&mut self, table: &BlockId) -> Applied {
        self.edit(Mutation::AddRow {
            block: table.clone(),
            place: Place::Last,
        })
        .unwrap()
    }

    fn expense(&mut self) -> RowId {
        let expenses = self.expenses.clone();
        created_row(&self.add_row(&expenses))
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
            conditions: vec![Filter {
                field: self.amount.clone(),
                matches: Match::Above(Cell::Number(500.0)),
            }],
            ..setup(
                "Flag large expenses",
                Trigger::FieldChanged(self.expenses.clone(), self.amount.clone()),
                vec![
                    Step::Set {
                        field: self.category.clone(),
                        value: Cell::Select(self.review.clone()),
                    },
                    notify(&format!("Large expense: {{{}}}", self.item)),
                ],
            )
        }
    }

    fn cell(&mut self, row: &RowId, field: &FieldId) -> Cell {
        let expenses = self.expenses.clone();
        self.workspace
            .rows(&expenses, &all())
            .unwrap()
            .rows
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

#[test]
fn a_change_that_meets_the_conditions_runs_the_steps_as_the_automation() {
    let mut budget = Budget::new();
    let flag = budget.create(budget.flag_large()).unwrap();
    let echo = setup(
        "Echo",
        Trigger::FieldChanged(budget.expenses.clone(), budget.category.clone()),
        vec![notify("Category changed")],
    );
    budget.create(echo).unwrap();
    let (item, amount, category) = (
        budget.item.clone(),
        budget.amount.clone(),
        budget.category.clone(),
    );
    let rent = budget.expense();
    budget.set(&rent, &item, Cell::Text("Rent".into()));

    let small = budget.set(&rent, &amount, Cell::Number(120.0));
    assert!(small.ran.is_empty());
    assert_eq!(budget.cell(&rent, &category), Cell::Empty);

    let large = budget.set(&rent, &amount, Cell::Number(950.0));
    assert_eq!(
        large.ran.len(),
        1,
        "the change an automation makes starts no other"
    );
    let ran = &large.ran[0];
    assert_eq!(ran.automation, flag);
    assert_eq!(ran.notices, ["Large expense: Rent"]);
    assert!(
        ran.changed
            .contains(&Changed::Table(budget.expenses.clone()))
    );
    let action = ran.outcome.clone().unwrap();
    assert_eq!(
        budget.cell(&rent, &category),
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
fn a_new_row_runs_the_automations_of_its_table_and_a_step_adds_rows_elsewhere() {
    let mut budget = Budget::new();
    let note = setup(
        "Log new expense",
        Trigger::RowAdded(budget.expenses.clone()),
        vec![Step::AddRow {
            table: budget.months.clone(),
            cells: vec![(budget.month_name.clone(), Cell::Text("New expense".into()))],
        }],
    );
    let note = budget.create(note).unwrap();
    assert_eq!(budget.workspace.automations().unwrap()[0].owner, None);

    let expenses = budget.expenses.clone();
    let months = budget.months.clone();
    let applied = budget.add_row(&expenses);
    assert_eq!(applied.ran.len(), 1);
    assert!(
        applied.ran[0]
            .changed
            .contains(&Changed::Table(months.clone()))
    );
    assert_eq!(budget.count(&months), 1);
    assert!(budget.add_row(&months).ran.is_empty());

    budget
        .edit(Mutation::SetAutomationOn {
            automation: note.clone(),
            on: false,
        })
        .unwrap();
    assert!(budget.add_row(&expenses).ran.is_empty());
    budget
        .edit(Mutation::RemoveAutomation { automation: note })
        .unwrap();
    assert!(budget.workspace.automations().unwrap().is_empty());
}

#[test]
fn a_button_field_runs_its_automations_and_holds_no_value() {
    let mut budget = Budget::new();
    let summary = setup(
        "Monthly summary",
        Trigger::Clicked(budget.months.clone(), budget.summarize.clone()),
        vec![notify("Summarized")],
    );
    budget.create(summary).unwrap();
    let months = budget.months.clone();
    let october = created_row(&budget.add_row(&months));

    let ran = budget
        .workspace
        .press(&owner(), &months, &october, &budget.summarize)
        .unwrap();
    assert_eq!(ran.len(), 1);
    assert_eq!(ran[0].notices, ["Summarized"]);
    assert!(ran[0].outcome.is_ok());

    assert_eq!(
        budget
            .workspace
            .press(&owner(), &months, &october, &budget.amount)
            .unwrap_err(),
        Error::NotFound
    );
    let stranger = Actor::Person(PersonId::new("someone"));
    assert_eq!(
        budget
            .workspace
            .press(&stranger, &months, &october, &budget.summarize)
            .unwrap_err(),
        Error::NotAllowed
    );
    let value = Mutation::SetCell {
        block: months.clone(),
        row: october,
        field: budget.summarize.clone(),
        value: Cell::Text("x".into()),
    };
    assert_eq!(budget.edit(value).unwrap_err(), Error::WrongKind);
    let mut on_button = budget.flag_large();
    on_button.trigger = Some(Trigger::RowAdded(months));
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
    let amount = budget.amount.clone();
    let first = budget.expense();
    let last = budget.expense();
    budget.set(&first, &amount, Cell::Number(10.0));
    budget.set(&last, &amount, Cell::Number(10.0));

    let ran = budget.workspace.test(&owner(), &flag).unwrap();
    assert!(ran.outcome.is_ok());
    let category = budget.category.clone();
    assert_eq!(
        budget.cell(&last, &category),
        Cell::Select(budget.review.clone())
    );
    assert_eq!(budget.cell(&first, &category), Cell::Empty);
}

#[test]
fn a_setup_must_name_fields_of_the_right_kind_and_stay_within_bounds() {
    let mut budget = Budget::new();
    let expenses = budget.expenses.clone();
    let formula = field(&mut budget.workspace, &expenses, "Left", FieldKind::Formula);
    let flag = |budget: &Budget| budget.flag_large();
    let mut broken = Vec::new();
    let mut setup = flag(&budget);
    setup.trigger = Some(Trigger::Clicked(expenses.clone(), budget.amount.clone()));
    broken.push((setup, Error::WrongKind));
    let mut setup = flag(&budget);
    setup.trigger = Some(Trigger::FieldChanged(expenses.clone(), formula.clone()));
    broken.push((setup, Error::WrongKind));
    let mut setup = flag(&budget);
    setup.steps = vec![Step::Set {
        field: formula,
        value: Cell::Number(1.0),
    }];
    broken.push((setup, Error::WrongKind));
    let mut setup = flag(&budget);
    setup.steps = vec![Step::Set {
        field: budget.amount.clone(),
        value: Cell::Text("many".into()),
    }];
    broken.push((setup, Error::WrongKind));
    let mut setup = flag(&budget);
    setup.conditions = vec![Filter {
        field: budget.amount.clone(),
        matches: Match::Above(Cell::Text("x".into())),
    }];
    broken.push((setup, Error::WrongKind));
    let mut setup = flag(&budget);
    setup.steps = vec![notify(&format!("{{{}}}", budget.summarize))];
    broken.push((setup, Error::NotFound));
    let mut setup = flag(&budget);
    setup.conditions = vec![setup.conditions[0].clone(); MAX_CONDITIONS + 1];
    broken.push((setup, Error::TooMany));
    let mut setup = flag(&budget);
    setup.trigger = None;
    broken.push((setup, Error::NotFound));
    let mut setup = flag(&budget);
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
    let summary = setup(
        "Monthly summary",
        Trigger::Clicked(budget.months.clone(), budget.summarize.clone()),
        vec![Step::AddRow {
            table: budget.expenses.clone(),
            cells: Vec::new(),
        }],
    );
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
    assert!(left[0].setup.steps.is_empty());
    budget
        .edit(Mutation::SetAutomationOn {
            automation: summary,
            on: true,
        })
        .unwrap();
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
    assert_eq!(
        automation.setup.steps,
        [notify(&format!("Large expense: {{{}}}", budget.item))]
    );
    let rent = budget.expense();
    assert!(
        budget
            .set(&rent, &budget.amount.clone(), Cell::Number(950.0))
            .ran
            .is_empty()
    );
    budget
        .edit(Mutation::SetAutomationOn {
            automation: automation.id.clone(),
            on: true,
        })
        .unwrap();
    let repaired = budget.workspace.automations().unwrap().remove(0);
    assert!(repaired.on);
    assert_eq!(repaired.attention, None);
}

#[test]
fn an_automation_whose_trigger_field_went_is_repaired_by_choosing_another() {
    let mut budget = Budget::new();
    let mut setup = budget.flag_large();
    setup
        .steps
        .push(notify(&format!("{{{}}} spent", budget.amount)));
    let flag = budget.create(setup).unwrap();
    budget
        .edit(Mutation::RemoveField {
            block: budget.expenses.clone(),
            field: budget.amount.clone(),
        })
        .unwrap();
    let broken = budget.workspace.automations().unwrap().remove(0);
    assert!(broken.setup.conditions.is_empty());
    assert_eq!(broken.setup.steps.last(), Some(&notify("{Amount} spent")));
    let on = Mutation::SetAutomationOn {
        automation: flag.clone(),
        on: true,
    };
    assert_eq!(budget.edit(on.clone()).unwrap_err(), Error::NotAllowed);
    let mut repaired = broken.setup;
    repaired.trigger = Some(Trigger::FieldChanged(
        budget.expenses.clone(),
        budget.item.clone(),
    ));
    budget
        .edit(Mutation::SetAutomation {
            automation: flag,
            setup: Box::new(repaired),
        })
        .unwrap();
    budget.edit(on).unwrap();
    assert!(budget.workspace.automations().unwrap()[0].on);
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
    let expenses = budget.expenses.clone();
    let due = field(&mut budget.workspace, &expenses, "Due", FieldKind::Date);
    let (amount, category) = (budget.amount.clone(), budget.category.clone());
    let setup = AutomationSetup {
        conditions: vec![
            Filter {
                field: due.clone(),
                matches: Match::Below(Cell::Date("2026-11-01".into())),
            },
            Filter {
                field: amount.clone(),
                matches: Match::Below(Cell::Number(100.0)),
            },
            Filter {
                field: category.clone(),
                matches: Match::IsNot(Cell::Select(budget.review.clone())),
            },
        ],
        ..setup(
            "Early and small",
            Trigger::FieldChanged(expenses.clone(), due.clone()),
            vec![notify(&format!("{{{amount}}} is due on {{{due}}}"))],
        )
    };
    budget.create(setup).unwrap();
    let row = budget.expense();
    let due_on = |budget: &mut Budget, date: &str| {
        budget
            .edit(Mutation::SetCell {
                block: expenses.clone(),
                row: row.clone(),
                field: due.clone(),
                value: Cell::Date(date.into()),
            })
            .unwrap()
            .ran
    };
    assert!(due_on(&mut budget, "2026-10-01").is_empty());
    budget.set(&row, &amount, Cell::Number(42.5));
    assert!(due_on(&mut budget, "2026-12-01").is_empty());
    assert_eq!(
        due_on(&mut budget, "2026-10-02")[0].notices,
        ["42.5 is due on 2026-10-02"]
    );
    budget.set(&row, &category, Cell::Select(budget.review.clone()));
    assert!(due_on(&mut budget, "2026-10-03").is_empty());
}

#[test]
fn a_deleted_trigger_table_takes_its_uses_along_so_choosing_another_repairs() {
    let mut budget = Budget::new();
    let (expenses, months) = (budget.expenses.clone(), budget.months.clone());
    let mirror = AutomationSetup {
        conditions: vec![Filter {
            field: budget.amount.clone(),
            matches: Match::NotEmpty,
        }],
        ..setup(
            "Mirror",
            Trigger::RowAdded(expenses.clone()),
            vec![
                Step::AddRow {
                    table: expenses.clone(),
                    cells: Vec::new(),
                },
                Step::AddRow {
                    table: months.clone(),
                    cells: Vec::new(),
                },
                Step::Set {
                    field: budget.category.clone(),
                    value: Cell::Empty,
                },
                notify(&format!("{{{}}} added", budget.item)),
            ],
        )
    };
    let mirror = budget.create(mirror).unwrap();
    budget
        .edit(Mutation::DeleteTable {
            block: expenses.clone(),
        })
        .unwrap();
    let broken = budget.workspace.automations().unwrap().remove(0);
    assert!(broken.setup.conditions.is_empty());
    assert_eq!(
        broken.setup.steps,
        [
            Step::AddRow {
                table: months.clone(),
                cells: Vec::new(),
            },
            notify("{Name} added"),
        ]
    );
    let mut repaired = broken.setup;
    repaired.trigger = Some(Trigger::RowAdded(months));
    budget
        .edit(Mutation::SetAutomation {
            automation: mirror.clone(),
            setup: Box::new(repaired),
        })
        .unwrap();
    budget
        .edit(Mutation::SetAutomationOn {
            automation: mirror,
            on: true,
        })
        .unwrap();
}
