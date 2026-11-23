#![allow(clippy::unwrap_used)]

use liquid_core::{
    ActionId, Actor, Applied, AreaId, AutomationSetup, BlockId, Cell, Created, Entry, EntryQuery,
    Error, FieldId, FieldKind, MemoryStore, Mutation, Operation, Origin, PageId, PersonId, Place,
    Purge, Request, RevertKind, RowId, RowQuery, Step, Trigger,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

struct Budget {
    workspace: Workspace,
    area: AreaId,
    page: PageId,
    text: BlockId,
    table: BlockId,
    amount: FieldId,
}

impl Budget {
    fn new() -> Self {
        let workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let mut budget = Self {
            workspace,
            area: AreaId::parse(&"0".repeat(32)).unwrap(),
            page: PageId::parse(&"0".repeat(32)).unwrap(),
            text: BlockId::parse(&"0".repeat(32)).unwrap(),
            table: BlockId::parse(&"0".repeat(32)).unwrap(),
            amount: FieldId::parse(&"0".repeat(32)).unwrap(),
        };
        budget.area = match budget.edit(Mutation::CreateArea {
            name: "Private".into(),
        }) {
            Applied {
                created: Some(Created::Area(area)),
                ..
            } => area,
            other => panic!("an area, got {other:?}"),
        };
        budget.page = match budget.edit(Mutation::CreatePage {
            area: budget.area.clone(),
            parent: None,
            title: "Household budget".into(),
        }) {
            Applied {
                created: Some(Created::Page(page)),
                ..
            } => page,
            other => panic!("a Page, got {other:?}"),
        };
        budget.text = match budget.edit(Mutation::AddTextBlock {
            page: budget.page.clone(),
            after: None,
            text: String::new(),
        }) {
            Applied {
                created: Some(Created::Block { block, .. }),
                ..
            } => block,
            other => panic!("a Block, got {other:?}"),
        };
        budget.table = match budget.edit(Mutation::AddDatabaseBlock {
            page: budget.page.clone(),
            after: None,
            name: "Expenses".into(),
            layout: liquid_core::Layout::Table,
        }) {
            Applied {
                created: Some(Created::Block { block, .. }),
                ..
            } => block,
            other => panic!("a Table, got {other:?}"),
        };
        budget.amount = match budget.edit(Mutation::AddField {
            block: budget.table.clone(),
            place: Place::Last,
            name: "Amount".into(),
            kind: FieldKind::Number,
        }) {
            Applied {
                created: Some(Created::Field(field)),
                ..
            } => field,
            other => panic!("a field, got {other:?}"),
        };
        budget
    }

    fn edit(&mut self, mutation: Mutation) -> Applied {
        self.workspace
            .apply(Request {
                actor: owner(),
                origin: Origin::Ui,
                mutation,
            })
            .unwrap()
    }

    fn typed(&mut self, at: usize, insert: &str) -> ActionId {
        self.edit(Mutation::EditText {
            page: self.page.clone(),
            block: self.text.clone(),
            at,
            delete: 0,
            insert: insert.into(),
        })
        .action
    }

    fn text(&mut self) -> String {
        self.workspace
            .text(&self.page, &self.text)
            .unwrap()
            .unwrap_or_default()
    }

    fn row(&mut self) -> RowId {
        match self.edit(Mutation::AddRow {
            block: self.table.clone(),
            place: Place::Last,
        }) {
            Applied {
                created: Some(Created::Row(row)),
                ..
            } => row,
            other => panic!("a row, got {other:?}"),
        }
    }

    fn set(&mut self, row: &RowId, value: f64) -> ActionId {
        self.edit(Mutation::SetCell {
            block: self.table.clone(),
            row: row.clone(),
            field: self.amount.clone(),
            value: Cell::Number(value),
        })
        .action
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
            .rows(&self.table, &all)
            .unwrap()
            .rows
            .iter()
            .map(|row| row.cell(&self.amount).cloned().unwrap_or(Cell::Empty))
            .collect()
    }

    fn entries(&mut self) -> Vec<Entry> {
        self.workspace
            .entries(&EntryQuery {
                page: None,
                actor: None,
                origin: None,
                before: None,
                limit: 100,
            })
            .unwrap()
    }
}

#[test]
fn a_revert_records_a_new_action_that_undoes_only_the_one_it_reverts() {
    let mut budget = Budget::new();
    let (rent, food) = (budget.row(), budget.row());
    let first = budget.set(&rent, 950.0);
    budget.set(&food, 84.2);

    let reverted = budget
        .workspace
        .revert(&owner(), std::slice::from_ref(&first))
        .unwrap();
    assert!(
        reverted
            .changed
            .contains(&liquid_core::Changed::Table(budget.table.clone()))
    );
    assert_eq!(budget.amounts(), [Cell::Empty, Cell::Number(84.2)]);
    let newest = budget.workspace.history(None, 1).unwrap().remove(0);
    assert_eq!(newest.id, reverted.action);
    assert_eq!(
        newest.operation,
        Operation::Revert {
            reverted: vec![first.clone()],
            kind: RevertKind::Revert,
        }
    );
    let entry = budget
        .entries()
        .into_iter()
        .find(|entry| entry.actions == [first.clone()])
        .unwrap();
    assert!(entry.undone);
}

#[test]
fn a_revert_that_a_later_change_would_hide_is_refused() {
    let mut budget = Budget::new();
    let rent = budget.row();
    let first = budget.set(&rent, 950.0);
    budget.set(&rent, 900.0);
    assert_eq!(
        budget.workspace.revert(&owner(), &[first]).unwrap_err(),
        Error::Conflict
    );
    assert_eq!(budget.amounts(), [Cell::Number(900.0)]);
}

#[test]
fn reverting_a_deleted_table_brings_back_its_rows_views_and_automations() {
    let mut budget = Budget::new();
    let rent = budget.row();
    budget.set(&rent, 950.0);
    let setup = AutomationSetup {
        name: "Note".into(),
        trigger: Some(Trigger::RowAdded(budget.table.clone())),
        conditions: Vec::new(),
        steps: vec![Step::Notify {
            message: "Added".into(),
        }],
    };
    budget.edit(Mutation::CreateAutomation {
        setup: Box::new(setup),
    });
    let deleted = budget
        .edit(Mutation::DeleteTable {
            block: budget.table.clone(),
        })
        .action;
    assert!(budget.workspace.tables().unwrap().is_empty());
    assert!(budget.workspace.automations().unwrap().is_empty());

    budget.workspace.revert(&owner(), &[deleted]).unwrap();
    assert_eq!(budget.workspace.tables().unwrap()[0].name, "Expenses");
    assert_eq!(budget.amounts(), [Cell::Number(950.0)]);
    assert_eq!(budget.workspace.automations().unwrap().len(), 1);
    assert_eq!(budget.workspace.view_count(&budget.page).unwrap(), 2);
}

#[test]
fn undo_takes_back_the_last_pause_of_typing_and_redo_brings_it_again() {
    let mut budget = Budget::new();
    for (at, key) in ["R", "e", "n", "t"].iter().enumerate() {
        budget.typed(at, key);
    }
    assert_eq!(budget.text(), "Rent");
    let grouped = budget.entries().remove(0);
    assert_eq!(grouped.actions.len(), 4);
    assert_eq!(grouped.title, "Edited text on “Household budget”");

    let undone = budget.workspace.undo(&owner()).unwrap().unwrap();
    assert_eq!(budget.text(), "");
    let newest = budget.workspace.history(None, 1).unwrap().remove(0);
    assert_eq!(newest.id, undone.action);
    assert!(matches!(
        newest.operation,
        Operation::Revert {
            kind: RevertKind::Undo,
            ..
        }
    ));
    budget.workspace.redo(&owner()).unwrap().unwrap();
    assert_eq!(budget.text(), "Rent");
    budget.workspace.undo(&owner()).unwrap().unwrap();
    assert_eq!(budget.text(), "");

    budget.typed(0, "Food");
    assert!(budget.workspace.redo(&owner()).unwrap().is_none());
    budget.workspace.undo(&owner()).unwrap().unwrap();
    assert_eq!(budget.text(), "");
    budget.row();
    budget.workspace.undo(&owner()).unwrap().unwrap();
    assert!(budget.amounts().is_empty());
}

#[test]
fn restore_version_returns_the_page_and_its_tables_to_an_earlier_point() {
    let mut budget = Budget::new();
    budget.typed(0, "October");
    let rent = budget.row();
    let point = budget.set(&rent, 950.0);
    budget.typed(7, " is done");
    budget.set(&rent, 900.0);
    budget.row();

    let versions = budget.workspace.versions(&budget.page).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].actor, owner());
    let setup = AutomationSetup {
        name: "Zero".into(),
        trigger: Some(Trigger::RowAdded(budget.table.clone())),
        conditions: Vec::new(),
        steps: vec![Step::Set {
            field: budget.amount.clone(),
            value: Cell::Number(0.0),
        }],
    };
    budget.edit(Mutation::CreateAutomation {
        setup: Box::new(setup),
    });
    budget.row();
    let versions = budget.workspace.versions(&budget.page).unwrap();
    assert!(matches!(versions[0].actor, Actor::Automation(_)));
    assert_eq!(versions[1].actor, owner());
    assert!(!versions[1].save_point);
    budget
        .workspace
        .restore(&owner(), &budget.page, &point)
        .unwrap();
    assert_eq!(budget.text(), "October");
    assert_eq!(budget.amounts(), [Cell::Number(950.0)]);
    assert!(budget.workspace.automations().unwrap().len() == 1);
    let newest = budget.workspace.history(None, 1).unwrap().remove(0);
    assert_eq!(
        newest.operation,
        Operation::Restore {
            page: budget.page.clone(),
            to: point.clone(),
        }
    );

    let mut preview = budget.workspace.preview(&budget.page, &point).unwrap();
    assert_eq!(
        preview.text(&budget.page, &budget.text).unwrap().as_deref(),
        Some("October")
    );
}

#[test]
fn deleting_earlier_versions_keeps_the_page_and_drops_what_only_they_held() {
    let mut budget = Budget::new();
    let secret = budget.typed(0, "hunter2");
    budget.edit(Mutation::EditText {
        page: budget.page.clone(),
        block: budget.text.clone(),
        at: 0,
        delete: 7,
        insert: "Rent".into(),
    });
    let purged = budget
        .workspace
        .purge(&owner(), &Purge::Page(budget.page.clone()))
        .unwrap();
    assert_eq!(budget.text(), "Rent");
    let records = serde_json::to_string(&budget.workspace.history(None, 100).unwrap()).unwrap();
    assert!(!records.contains("hunter2"));
    assert_eq!(
        budget
            .workspace
            .revert(&owner(), std::slice::from_ref(&secret))
            .unwrap_err(),
        Error::Purged
    );
    let entries = budget.entries();
    assert_eq!(entries[0].actions, [purged.action]);
    assert_eq!(
        entries[0].title,
        "Deleted the earlier versions of “Household budget”"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.actions.contains(&secret) && entry.purged)
    );
    assert!(budget.workspace.versions(&budget.page).unwrap().len() <= 1);

    let stranger = Actor::Person(PersonId::new("someone"));
    assert_eq!(
        budget
            .workspace
            .purge(&stranger, &Purge::Page(budget.page.clone()))
            .unwrap_err(),
        Error::NotAllowed
    );
}

#[test]
fn a_deleted_table_deleted_for_good_leaves_only_that_it_was() {
    let mut budget = Budget::new();
    let rent = budget.row();
    budget.set(&rent, 950.0);
    let live = Purge::Table(budget.table.clone());
    budget.edit(Mutation::DeleteTable {
        block: budget.table.clone(),
    });
    assert_eq!(
        budget.entries()[0].deleted_table,
        Some(budget.table.clone())
    );
    budget.workspace.purge(&owner(), &live).unwrap();
    let records = serde_json::to_string(&budget.workspace.history(None, 100).unwrap()).unwrap();
    assert!(!records.contains("Expenses"));
    assert!(
        budget
            .workspace
            .rows(
                &budget.table,
                &RowQuery {
                    sort: None,
                    filter: None,
                    within: None,
                    start: 0,
                    limit: 1,
                    search: None,
                    totals: Vec::new(),
                }
            )
            .is_err()
    );
    let titles: Vec<String> = budget
        .entries()
        .into_iter()
        .map(|entry| entry.title)
        .collect();
    assert_eq!(titles[0], "Deleted a Table for good");
    assert!(
        budget
            .entries()
            .iter()
            .all(|entry| entry.deleted_table.is_none())
    );
    assert!(titles.iter().all(|title| !title.contains("Expenses")));
}

#[test]
fn an_undone_edit_stays_undone_once_its_page_loads_again() {
    let mut budget = Budget::new();
    budget.typed(0, "Rent");
    budget.workspace.close_page(&budget.page.clone());
    budget.workspace.undo(&owner()).unwrap().unwrap();
    budget.workspace.close_page(&budget.page.clone());
    assert_eq!(budget.text(), "");
}

#[test]
fn reverting_an_edit_that_was_undone_and_redone_reverts_what_now_carries_it() {
    let mut budget = Budget::new();
    let typed = budget.typed(0, "Rent");
    budget.workspace.undo(&owner()).unwrap().unwrap();
    budget.workspace.redo(&owner()).unwrap().unwrap();
    assert_eq!(budget.text(), "Rent");
    budget
        .workspace
        .revert(&owner(), std::slice::from_ref(&typed))
        .unwrap();
    assert_eq!(budget.text(), "");
    let entry = budget
        .entries()
        .into_iter()
        .find(|entry| entry.actions.contains(&typed))
        .unwrap();
    assert!(entry.undone);
}

#[test]
fn a_version_before_an_undo_and_a_revert_can_be_restored() {
    let mut budget = Budget::new();
    let typed = budget.typed(0, "Rent");
    budget.workspace.undo(&owner()).unwrap().unwrap();
    budget.workspace.redo(&owner()).unwrap().unwrap();
    budget
        .workspace
        .revert(&owner(), std::slice::from_ref(&typed))
        .unwrap();
    assert_eq!(budget.text(), "");
    let page = budget.page.clone();
    budget.workspace.close_page(&page);
    let versions = budget.workspace.versions(&page).unwrap();
    budget.workspace.preview(&page, &versions[0].after).unwrap();
    budget.workspace.close_page(&page);
    budget.workspace.restore(&owner(), &page, &typed).unwrap();
    assert_eq!(budget.text(), "Rent");
}

#[test]
fn an_undone_and_redone_edit_whose_page_was_purged_cannot_be_reverted() {
    let mut budget = Budget::new();
    let typed = budget.typed(0, "Rent");
    budget.workspace.undo(&owner()).unwrap().unwrap();
    budget.workspace.redo(&owner()).unwrap().unwrap();
    budget
        .workspace
        .purge(&owner(), &Purge::Page(budget.page.clone()))
        .unwrap();
    assert_eq!(
        budget
            .workspace
            .revert(&owner(), std::slice::from_ref(&typed))
            .unwrap_err(),
        Error::Purged
    );
}
