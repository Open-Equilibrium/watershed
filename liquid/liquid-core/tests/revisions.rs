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
            .entries
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
    let entry = budget
        .entries()
        .into_iter()
        .find(|entry| entry.actions.contains(&other))
        .unwrap();
    assert!(entry.revertable);
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
    let entry = budget
        .workspace
        .entries(&EntryQuery {
            page: None,
            actor: None,
            origin: None,
            before: None,
            limit: 10,
        })
        .unwrap()
        .entries;
    assert!(!entry.is_empty());
    budget
        .workspace
        .revert(&owner(), std::slice::from_ref(&early))
        .unwrap();
    assert_eq!(budget.amounts(), [Cell::Empty]);
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
    let entry = budget
        .entries()
        .into_iter()
        .find(|entry| entry.actions.contains(&restored.action))
        .unwrap();
    assert!(!entry.revertable);
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
fn reverting_several_actions_together_is_refused_when_a_later_change_would_hide_one() {
    let mut budget = Budget::new();
    let rent = budget.row();
    let food = budget.row();
    let first = budget.set(&rent, 950.0);
    let second = budget.set(&food, 300.0);
    budget.set(&rent, 900.0);
    assert_eq!(
        budget
            .workspace
            .revert(&owner(), &[second, first])
            .unwrap_err(),
        Error::Conflict
    );
    assert_eq!(budget.amounts(), [Cell::Number(900.0), Cell::Number(300.0)]);
}

#[test]
fn deleting_a_table_s_earlier_versions_keeps_the_page_s_own() {
    let mut budget = Budget::new();
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
            .filter(|view| view.kind == liquid_core::BlockKind::Database)
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
