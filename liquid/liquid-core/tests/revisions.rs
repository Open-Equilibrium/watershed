#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Action, ActionId, Actor, Applied, AutomationSetup, BlockId, BlockKind, Cell, Changed, Created,
    Entry, EntryQuery, Error, FieldId, FieldKind, Mutation, Operation, PageId, PersonId, Place,
    Purge, RevertKind, RowId, RowQuery, Step, Trigger,
};

struct Budget {
    workspace: Workspace,
    page: PageId,
    text: BlockId,
    table: BlockId,
    amount: FieldId,
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
        let Created::Page(page) = created(
            &mut workspace,
            Mutation::CreatePage {
                area,
                parent: None,
                title: "Household budget".into(),
            },
        ) else {
            panic!("a Page")
        };
        let Created::Block { block: text, .. } = created(
            &mut workspace,
            Mutation::AddTextBlock {
                page: page.clone(),
                after: None,
                text: String::new(),
            },
        ) else {
            panic!("a Block")
        };
        let Created::Block { block: table, .. } = created(
            &mut workspace,
            Mutation::AddDatabaseBlock {
                page: page.clone(),
                after: None,
                name: "Expenses".into(),
                layout: liquid_core::Layout::Table,
            },
        ) else {
            panic!("a Table")
        };
        let Created::Field(amount) = created(
            &mut workspace,
            Mutation::AddField {
                block: table.clone(),
                place: Place::Last,
                name: "Amount".into(),
                kind: FieldKind::Number,
            },
        ) else {
            panic!("a field")
        };
        Self {
            workspace,
            page,
            text,
            table,
            amount,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Applied {
        apply(&mut self.workspace, mutation).unwrap()
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
        let Created::Row(row) = created(
            &mut self.workspace,
            Mutation::AddRow {
                block: self.table.clone(),
                place: Place::Last,
            },
        ) else {
            panic!("a row")
        };
        row
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
        self.entries_up_to(100)
    }

    fn entries_up_to(&mut self, limit: usize) -> Vec<Entry> {
        self.workspace
            .entries(&EntryQuery {
                page: None,
                actor: None,
                origin: None,
                before: None,
                limit,
            })
            .unwrap()
            .entries
    }

    fn entry_of(&mut self, action: &ActionId) -> Entry {
        self.entries()
            .into_iter()
            .find(|entry| entry.actions.contains(action))
            .unwrap()
    }

    fn newest(&mut self) -> Action {
        self.workspace.history(None, 1).unwrap().remove(0)
    }

    fn automation(&self, name: &str, step: Step) -> Mutation {
        Mutation::CreateAutomation {
            setup: Box::new(AutomationSetup {
                name: name.into(),
                trigger: Some(Trigger::RowAdded(self.table.clone())),
                conditions: Vec::new(),
                steps: vec![step],
            }),
        }
    }

    fn zero_on_new_row(&self) -> Mutation {
        self.automation(
            "Zero",
            Step::Set {
                field: self.amount.clone(),
                value: Cell::Number(0.0),
            },
        )
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
            .contains(&Changed::Table(budget.table.clone()))
    );
    assert_eq!(budget.amounts(), [Cell::Empty, Cell::Number(84.2)]);
    let newest = budget.newest();
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
    let food = budget.row();
    let first = budget.set(&rent, 950.0);
    let second = budget.set(&food, 300.0);
    budget.set(&rent, 900.0);
    assert_eq!(
        budget
            .workspace
            .revert(&owner(), std::slice::from_ref(&first))
            .unwrap_err(),
        Error::Conflict
    );
    assert_eq!(
        budget
            .workspace
            .revert(&owner(), &[second, first])
            .unwrap_err(),
        Error::Conflict,
        "several actions are refused together"
    );
    assert_eq!(budget.amounts(), [Cell::Number(900.0), Cell::Number(300.0)]);
}

#[test]
fn reverting_a_deleted_table_brings_back_its_rows_views_and_automations() {
    let mut budget = Budget::new();
    let rent = budget.row();
    budget.set(&rent, 950.0);
    let setup = budget.automation(
        "Note",
        Step::Notify {
            message: "Added".into(),
        },
    );
    budget.edit(setup);
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
    let newest = budget.newest();
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
    let zero = budget.zero_on_new_row();
    budget.edit(zero);
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
    assert_eq!(budget.workspace.automations().unwrap().len(), 1);
    assert_eq!(
        budget.newest().operation,
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
    budget.edit(Mutation::RenamePage {
        page: budget.page.clone(),
        title: "Severance for Ana".into(),
    });
    budget.edit(Mutation::RenamePage {
        page: budget.page.clone(),
        title: "Monthly costs".into(),
    });
    let purged = budget
        .workspace
        .purge(&owner(), &Purge::Page(budget.page.clone()))
        .unwrap();
    assert_eq!(budget.text(), "Rent");
    let records = serde_json::to_string(&budget.workspace.history(None, 100).unwrap()).unwrap();
    assert!(!records.contains("hunter2"));
    assert!(!records.contains("Severance") && !records.contains("Household"));
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
        "Deleted the earlier versions of “Monthly costs”"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.actions.contains(&secret) && entry.purged)
    );
    assert!(budget.workspace.versions(&budget.page).unwrap().len() <= 1);
    let redone = budget.typed(4, "!");
    budget.workspace.undo(&owner()).unwrap().unwrap();
    budget.workspace.redo(&owner()).unwrap().unwrap();
    budget
        .workspace
        .purge(&owner(), &Purge::Page(budget.page.clone()))
        .unwrap();
    assert_eq!(
        budget.workspace.revert(&owner(), &[redone]).unwrap_err(),
        Error::Purged
    );

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
    let rows = RowQuery {
        sort: None,
        filter: None,
        within: None,
        start: 0,
        limit: 1,
        search: None,
        totals: Vec::new(),
    };
    assert!(budget.workspace.rows(&budget.table, &rows).is_err());
    let entries = budget.entries();
    assert_eq!(entries[0].title, "Deleted a Table for good");
    assert!(entries.iter().all(|entry| entry.deleted_table.is_none()));
    assert!(
        entries
            .iter()
            .all(|entry| !entry.title.contains("Expenses"))
    );
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
    let first = budget.typed(0, "Re");
    let typed = budget.typed(2, "nt");
    budget.workspace.undo(&owner()).unwrap().unwrap();
    budget.workspace.redo(&owner()).unwrap().unwrap();
    assert_eq!(budget.text(), "Rent");
    budget
        .workspace
        .revert(&owner(), &[typed.clone(), first])
        .unwrap();
    assert_eq!(budget.text(), "");
    assert!(budget.entry_of(&typed).undone);
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
fn deleting_a_table_for_good_leaves_other_workspace_changes_revertable() {
    let mut budget = Budget::new();
    let other = budget
        .edit(Mutation::CreateArea {
            name: "Work".into(),
        })
        .action;
    budget.edit(Mutation::DeleteTable {
        block: budget.table.clone(),
    });
    budget
        .workspace
        .purge(&owner(), &Purge::Table(budget.table.clone()))
        .unwrap();
    assert!(budget.entry_of(&other).revertable);
    budget
        .workspace
        .revert(&owner(), std::slice::from_ref(&other))
        .unwrap();
    assert_eq!(budget.workspace.areas().unwrap().len(), 1);
}

#[test]
fn an_action_older_than_ten_thousand_others_still_reverts() {
    let mut budget = Budget::new();
    let rent = budget.row();
    let early = budget.set(&rent, 950.0);
    for at in 0..10_001 {
        budget.typed(at, "x");
    }
    assert!(!budget.entries_up_to(10).is_empty());
    budget
        .workspace
        .revert(&owner(), std::slice::from_ref(&early))
        .unwrap();
    assert_eq!(budget.amounts(), [Cell::Empty]);
}

#[test]
fn after_deleting_earlier_versions_undo_passes_over_a_restore_it_cannot_take_back() {
    let mut budget = Budget::new();
    let rent = budget.typed(0, "Rent");
    budget.typed(4, " due");
    let page = budget.page.clone();
    let restored = budget.workspace.restore(&owner(), &page, &rent).unwrap();
    assert_eq!(budget.text(), "Rent");
    budget
        .workspace
        .purge(&owner(), &Purge::Page(page.clone()))
        .unwrap();
    budget.workspace.undo(&owner()).unwrap();
    assert_eq!(budget.text(), "Rent");
    assert!(!budget.entry_of(&restored.action).revertable);
}

#[test]
fn deleting_a_table_s_earlier_versions_keeps_the_page_s_own() {
    let mut budget = Budget::new();
    let zero = budget.zero_on_new_row();
    budget.edit(zero);
    budget.typed(0, "October");
    budget.row();
    budget.typed(7, " is done");
    let page = budget.page.clone();
    let before = budget.workspace.versions(&page).unwrap();
    budget
        .workspace
        .purge(&owner(), &Purge::Table(budget.table.clone()))
        .unwrap();
    let after = budget.workspace.versions(&page).unwrap();
    assert_eq!(after.len(), before.len());
    let older = after.last().unwrap().after.clone();
    budget.workspace.restore(&owner(), &page, &older).unwrap();
    assert_eq!(budget.text(), "October");
}

#[test]
fn deleting_earlier_versions_drops_the_files_only_they_still_showed() {
    let mut budget = Budget::new();
    let page = budget.page.clone();
    let media = |budget: &mut Budget, bytes: &[u8]| {
        let mut upload = budget
            .workspace
            .begin_upload(&owner(), bytes.len() as u64)
            .unwrap();
        budget.workspace.upload(&mut upload, bytes).unwrap();
        let attachment = budget.workspace.finish_upload(upload).unwrap().id;
        let Applied {
            created: Some(Created::Block { view, .. }),
            ..
        } = budget.edit(Mutation::AddMediaBlock {
            page: page.clone(),
            after: None,
            name: "Receipt.pdf".into(),
            media_type: "application/pdf".into(),
            attachment: attachment.clone(),
        })
        else {
            panic!("a Media Block")
        };
        (attachment, view)
    };
    let (gone, view) = media(&mut budget, b"the old receipt");
    let (kept, _) = media(&mut budget, b"the new receipt");
    budget.edit(Mutation::RemoveView {
        page: page.clone(),
        view,
    });
    budget
        .workspace
        .purge(&owner(), &Purge::Page(page))
        .unwrap();
    assert_eq!(budget.workspace.attachment_size(&gone).unwrap(), None);
    assert!(budget.workspace.attachment_size(&kept).unwrap().is_some());
}

#[test]
fn restore_version_leaves_a_table_deleted_since_to_its_own_revert() {
    let mut budget = Budget::new();
    let rent = budget.row();
    let point = budget.set(&rent, 950.0);
    budget.typed(0, "October");
    let deleted = budget
        .edit(Mutation::DeleteTable {
            block: budget.table.clone(),
        })
        .action;
    let page = budget.page.clone();
    let tables = |workspace: &mut Workspace| {
        workspace
            .views(&page, 0, usize::MAX)
            .unwrap()
            .into_iter()
            .filter(|view| view.kind == BlockKind::Database)
            .count()
    };
    let mut preview = budget.workspace.preview(&page, &point).unwrap();
    assert_eq!(tables(&mut preview), 0);
    budget.workspace.restore(&owner(), &page, &point).unwrap();
    assert_eq!(budget.text(), "");
    assert_eq!(tables(&mut budget.workspace), 0);
    budget.workspace.revert(&owner(), &[deleted]).unwrap();
    assert_eq!(tables(&mut budget.workspace), 1);
}
#[test]
fn a_version_brings_back_a_table_the_page_showed_then_as_it_was() {
    let mut budget = Budget::new();
    let rent = budget.row();
    let point = budget.set(&rent, 950.0);
    let page = budget.page.clone();
    let shown = budget
        .workspace
        .views(&page, 0, usize::MAX)
        .unwrap()
        .into_iter()
        .find(|view| view.block == budget.table)
        .unwrap();
    budget.edit(Mutation::RemoveView {
        page: page.clone(),
        view: shown.id,
    });
    budget.set(&rent, 900.0);

    let mut preview = budget.workspace.preview(&page, &point).unwrap();
    std::mem::swap(&mut budget.workspace, &mut preview);
    assert_eq!(budget.amounts(), [Cell::Number(950.0)]);
    std::mem::swap(&mut budget.workspace, &mut preview);
    budget.workspace.restore(&owner(), &page, &point).unwrap();
    assert_eq!(budget.amounts(), [Cell::Number(950.0)]);
}

#[test]
fn history_reaches_past_a_long_stretch_of_typing() {
    let mut budget = Budget::new();
    for at in 0..5_000 {
        budget.typed(at, "x");
    }
    let listed = budget
        .workspace
        .entries(&EntryQuery {
            page: None,
            actor: None,
            origin: None,
            before: None,
            limit: 60,
        })
        .unwrap()
        .entries;
    assert_eq!(listed[0].actions.len(), 5_000);
    assert!(
        listed
            .iter()
            .any(|entry| entry.title == "Created the Page “Household budget”")
    );
}
