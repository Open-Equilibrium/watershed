//! Database Blocks as Dart builds and reads them: fields, options, rows and cells cross as
//! flat messages, rows come in filtered and sorted windows, and writes reach the Block's
//! subscribers.

use std::sync::{Arc, Mutex};

use super::workspace::{Folder, Memory, lisbon, mutation, query, resource};
use crate::api::database::{
    BridgeCell, BridgeCellKind, BridgeFieldKind, BridgeFilter, BridgeMatch, BridgeSort,
};
use crate::api::workspace::{
    BridgeAnswer, BridgeBlockKind, BridgeMutation, BridgeMutationKind, BridgeQuery,
    BridgeQueryKind, BridgeResource, BridgeResourceKind, LiquidWorkspace, MAX_WINDOW,
};

fn cell(kind: BridgeCellKind) -> BridgeCell {
    BridgeCell {
        kind,
        text: None,
        number: 0.0,
        checked: false,
    }
}

fn text(value: &str) -> BridgeCell {
    BridgeCell {
        text: Some(value.into()),
        ..cell(BridgeCellKind::Text)
    }
}

fn number(value: f64) -> BridgeCell {
    BridgeCell {
        number: value,
        ..cell(BridgeCellKind::Number)
    }
}

/// The Database "Expenses" on the Page "Lisbon", after its Text Block.
struct Expenses {
    workspace: LiquidWorkspace,
    page: String,
    block: String,
    name: String,
    _folder: Folder,
}

impl Expenses {
    fn new(folder: &str) -> Self {
        let folder = Folder::new(folder);
        let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
        let (_, page, _, view) = lisbon(&workspace);
        let block = workspace
            .apply(BridgeMutation {
                page: Some(page.clone()),
                after: Some(view),
                text: Some("Expenses".into()),
                ..mutation(BridgeMutationKind::AddDatabaseBlock)
            })
            .unwrap()
            .block
            .unwrap();
        let mut expenses = Self {
            workspace,
            page,
            block,
            name: String::new(),
            _folder: folder,
        };
        expenses.name = expenses.database().database.unwrap().fields[0].id.clone();
        expenses
    }

    /// Applies `write` to the Database and answers what it created, or its Action.
    fn apply(&self, write: BridgeMutation) -> Result<String, String> {
        let applied = self.workspace.apply(BridgeMutation {
            page: Some(self.page.clone()),
            block: Some(self.block.clone()),
            ..write
        })?;
        Ok(applied
            .field
            .or(applied.row)
            .or(applied.option)
            .unwrap_or(applied.action))
    }

    fn field(&self, after: &str, name: &str, kind: BridgeFieldKind) -> String {
        self.apply(BridgeMutation {
            after: Some(after.into()),
            text: Some(name.into()),
            field_kind: Some(kind),
            ..mutation(BridgeMutationKind::AddField)
        })
        .unwrap()
    }

    fn row(&self, after: Option<&str>) -> String {
        self.apply(BridgeMutation {
            after: after.map(Into::into),
            ..mutation(BridgeMutationKind::AddRow)
        })
        .unwrap()
    }

    fn set(&self, row: &str, field: &str, value: BridgeCell) -> Result<String, String> {
        self.apply(BridgeMutation {
            row: Some(row.into()),
            field: Some(field.into()),
            cell: Some(value),
            ..mutation(BridgeMutationKind::SetCell)
        })
    }

    fn database(&self) -> BridgeAnswer {
        self.workspace
            .query(BridgeQuery {
                page: Some(self.page.clone()),
                block: Some(self.block.clone()),
                ..query(BridgeQueryKind::Database)
            })
            .unwrap()
    }

    fn rows(&self, read: BridgeQuery) -> Result<BridgeAnswer, String> {
        self.workspace.query(BridgeQuery {
            kind: BridgeQueryKind::Rows,
            page: Some(self.page.clone()),
            block: Some(self.block.clone()),
            ..read
        })
    }

    /// The window's rows as their "Name" cells.
    fn names(&self, read: BridgeQuery) -> Vec<String> {
        self.rows(read)
            .unwrap()
            .rows
            .iter()
            .map(|row| {
                let name = row.cells.iter().find(|cell| cell.field == self.name);
                name.and_then(|cell| cell.value.text.clone())
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[test]
fn a_database_s_fields_options_and_cells_cross_the_bridge() {
    let expenses = Expenses::new("database-fields");
    let amount = expenses.field(&expenses.name, "Amount", BridgeFieldKind::Number);
    let category = expenses.field(&amount, "Category", BridgeFieldKind::Select);
    let date = expenses.field(&category, "Date", BridgeFieldKind::Date);
    let paid = expenses.field(&date, "Paid", BridgeFieldKind::Checkbox);
    let home = expenses
        .apply(BridgeMutation {
            field: Some(category.clone()),
            text: Some("Home".into()),
            color: 2,
            ..mutation(BridgeMutationKind::AddOption)
        })
        .unwrap();
    let rent = expenses.row(None);
    expenses.set(&rent, &expenses.name, text("Rent")).unwrap();
    expenses.set(&rent, &amount, number(950.5)).unwrap();
    let select = BridgeCell {
        text: Some(home.clone()),
        ..cell(BridgeCellKind::Select)
    };
    expenses.set(&rent, &category, select.clone()).unwrap();
    let day = BridgeCell {
        text: Some("2026-10-28".into()),
        ..cell(BridgeCellKind::Date)
    };
    expenses.set(&rent, &date, day.clone()).unwrap();
    let ticked = BridgeCell {
        checked: true,
        ..cell(BridgeCellKind::Checkbox)
    };
    expenses.set(&rent, &paid, ticked.clone()).unwrap();

    let database = expenses.database().database.unwrap();
    assert_eq!(database.name, "Expenses");
    let fields: Vec<_> = database
        .fields
        .iter()
        .map(|field| (field.name.as_str(), field.kind))
        .collect();
    assert_eq!(
        fields,
        [
            ("Name", BridgeFieldKind::Text),
            ("Amount", BridgeFieldKind::Number),
            ("Category", BridgeFieldKind::Select),
            ("Date", BridgeFieldKind::Date),
            ("Paid", BridgeFieldKind::Checkbox),
        ]
    );
    let options = &database.fields[2].options;
    assert_eq!(
        (
            options[0].id.as_str(),
            options[0].name.as_str(),
            options[0].color
        ),
        (home.as_str(), "Home", 2)
    );

    let window = expenses.rows(query(BridgeQueryKind::Rows)).unwrap();
    assert_eq!((window.count, window.rows.len()), (1, 1));
    let row = &window.rows[0];
    assert_eq!(row.id, rent);
    let value = |field: &str| {
        row.cells
            .iter()
            .find(|cell| cell.field == field)
            .map(|cell| cell.value.clone())
    };
    assert_eq!(value(&expenses.name), Some(text("Rent")));
    assert_eq!(value(&amount), Some(number(950.5)));
    assert_eq!(value(&category), Some(select));
    assert_eq!(value(&date), Some(day));
    assert_eq!(value(&paid), Some(ticked));

    // Clearing a cell leaves it out of its row.
    expenses
        .set(&rent, &amount, cell(BridgeCellKind::Empty))
        .unwrap();
    let window = expenses.rows(query(BridgeQueryKind::Rows)).unwrap();
    assert!(window.rows[0].cells.iter().all(|cell| cell.field != amount));
}

#[test]
fn rows_come_in_filtered_and_sorted_windows() {
    let expenses = Expenses::new("database-rows");
    let amount = expenses.field(&expenses.name, "Amount", BridgeFieldKind::Number);
    let mut last = None;
    for (item, euros) in [
        ("Rent", Some(950.0)),
        ("groceries", Some(84.2)),
        ("Gift", None),
        ("Train", Some(42.5)),
    ] {
        let row = expenses.row(last.as_deref());
        expenses.set(&row, &expenses.name, text(item)).unwrap();
        if let Some(euros) = euros {
            expenses.set(&row, &amount, number(euros)).unwrap();
        }
        last = Some(row);
    }

    assert_eq!(
        expenses.names(query(BridgeQueryKind::Rows)),
        ["Rent", "groceries", "Gift", "Train"]
    );
    let by_amount = BridgeQuery {
        sort: Some(BridgeSort {
            field: amount.clone(),
            descending: false,
        }),
        ..query(BridgeQueryKind::Rows)
    };
    assert_eq!(
        expenses.names(by_amount.clone()),
        ["Train", "groceries", "Rent", "Gift"]
    );
    assert_eq!(
        expenses.names(BridgeQuery {
            start: 1,
            limit: 2,
            ..by_amount.clone()
        }),
        ["groceries", "Rent"]
    );
    let filtered = |matches: BridgeMatch, field: &str, value: Option<BridgeCell>| BridgeQuery {
        filter: Some(BridgeFilter {
            field: field.into(),
            matches,
            value,
        }),
        ..by_amount.clone()
    };
    assert_eq!(
        expenses.names(filtered(BridgeMatch::Empty, &amount, None)),
        ["Gift"]
    );
    assert_eq!(
        expenses.names(filtered(BridgeMatch::NotEmpty, &amount, None)),
        ["Train", "groceries", "Rent"]
    );
    assert_eq!(
        expenses.names(filtered(BridgeMatch::Is, &amount, Some(number(84.2)))),
        ["groceries"]
    );
    let no_value = filtered(BridgeMatch::Is, &amount, Some(cell(BridgeCellKind::Empty)));
    assert_eq!(expenses.names(no_value.clone()), ["Gift"]);
    assert_eq!(expenses.rows(no_value).unwrap().count, 1);
    let containing = filtered(BridgeMatch::Contains, &expenses.name, Some(text("E")));
    assert_eq!(expenses.names(containing.clone()), ["groceries", "Rent"]);
    assert_eq!(expenses.rows(containing).unwrap().count, 2);
}

#[test]
fn renames_and_removals_show_in_the_next_read() {
    let expenses = Expenses::new("database-remove");
    let amount = expenses.field(&expenses.name, "Amount", BridgeFieldKind::Number);
    let rent = expenses.row(None);
    let train = expenses.row(Some(&rent));
    for write in [
        BridgeMutation {
            text: Some("Trip costs".into()),
            ..mutation(BridgeMutationKind::RenameDatabase)
        },
        BridgeMutation {
            field: Some(amount.clone()),
            text: Some("Euros".into()),
            ..mutation(BridgeMutationKind::RenameField)
        },
        BridgeMutation {
            field: Some(expenses.name.clone()),
            ..mutation(BridgeMutationKind::RemoveField)
        },
        BridgeMutation {
            row: Some(rent.clone()),
            ..mutation(BridgeMutationKind::RemoveRow)
        },
    ] {
        expenses.apply(write).unwrap();
    }
    let database = expenses.database().database.unwrap();
    assert_eq!(database.name, "Trip costs");
    let names: Vec<_> = database.fields.iter().map(|field| &field.name).collect();
    assert_eq!(names, ["Euros"]);
    let window = expenses.rows(query(BridgeQueryKind::Rows)).unwrap();
    let rows: Vec<_> = window.rows.iter().map(|row| &row.id).collect();
    assert_eq!(rows, [&train]);
}

#[test]
fn a_database_write_reaches_its_block_s_subscribers_and_then_leaves_memory() {
    let expenses = Expenses::new("database-notify");
    let page = liquid_core::PageId::parse(&expenses.page).unwrap();
    assert!(!expenses.workspace.is_loaded(&page), "after adding it");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let block = BridgeResource {
        page: Some(expenses.page.clone()),
        block: Some(expenses.block.clone()),
        ..resource(BridgeResourceKind::Block)
    };
    let sink = seen.clone();
    expenses
        .workspace
        .watch(1, block, move |change| {
            sink.lock().unwrap().push(change.seq);
            true
        })
        .unwrap();
    let rent = expenses.row(None);
    expenses.set(&rent, &expenses.name, text("Rent")).unwrap();
    assert_eq!(*seen.lock().unwrap(), [0, 1, 2]);

    expenses.workspace.close(1);
    assert!(!expenses.workspace.is_loaded(&page));
    expenses.set(&rent, &expenses.name, text("Flat")).unwrap();
    assert!(!expenses.workspace.is_loaded(&page), "after a write");
    let refused = expenses.set(&rent, &expenses.name, number(1.0));
    assert_eq!(refused.unwrap_err(), "the value is not of its field's kind");
    assert!(
        !expenses.workspace.is_loaded(&page),
        "after a refused write"
    );
    expenses.rows(query(BridgeQueryKind::Rows)).unwrap();
    assert!(!expenses.workspace.is_loaded(&page), "after a read");
}

#[test]
fn malformed_database_requests_are_refused() {
    let expenses = Expenses::new("database-refuse");
    let rent = expenses.row(None);
    let kindless = expenses.apply(BridgeMutation {
        text: Some("Amount".into()),
        ..mutation(BridgeMutationKind::AddField)
    });
    assert_eq!(kindless.unwrap_err(), "field kind is missing");
    let valueless = expenses.apply(BridgeMutation {
        row: Some(rent.clone()),
        field: Some(expenses.name.clone()),
        ..mutation(BridgeMutationKind::SetCell)
    });
    assert_eq!(valueless.unwrap_err(), "cell is missing");
    let textless = expenses.set(&rent, &expenses.name, cell(BridgeCellKind::Text));
    assert_eq!(textless.unwrap_err(), "the cell's text is missing");
    let unknown = expenses.set(
        &rent,
        &expenses.name,
        BridgeCell {
            text: Some("home".into()),
            ..cell(BridgeCellKind::Select)
        },
    );
    assert_eq!(unknown.unwrap_err(), "option is missing or no identity");
    for color in [-1, 256] {
        let colored = expenses.apply(BridgeMutation {
            field: Some(expenses.name.clone()),
            text: Some("Home".into()),
            color,
            ..mutation(BridgeMutationKind::AddOption)
        });
        assert_eq!(colored.unwrap_err(), "color is out of range", "{color}");
    }
    let unmatched = expenses.rows(BridgeQuery {
        filter: Some(BridgeFilter {
            field: expenses.name.clone(),
            matches: BridgeMatch::Contains,
            value: None,
        }),
        ..query(BridgeQueryKind::Rows)
    });
    assert_eq!(unmatched.unwrap_err(), "the filter's value is missing");
    let both = expenses.apply(BridgeMutation {
        after: Some(rent),
        last: true,
        ..mutation(BridgeMutationKind::AddRow)
    });
    assert_eq!(both.unwrap_err(), "after and last cannot both be set");
}

#[test]
fn rows_and_fields_added_last_go_after_all_others() {
    let expenses = Expenses::new("database-last");
    let last = |kind| BridgeMutation {
        last: true,
        ..mutation(kind)
    };
    let a = expenses.apply(last(BridgeMutationKind::AddRow)).unwrap();
    let b = expenses.apply(last(BridgeMutationKind::AddRow)).unwrap();
    let first = expenses.row(None);
    let rows = expenses.rows(query(BridgeQueryKind::Rows)).unwrap().rows;
    let ids: Vec<_> = rows.into_iter().map(|row| row.id).collect();
    assert_eq!(ids, [first, a, b]);

    let amount = expenses
        .apply(BridgeMutation {
            text: Some("Amount".into()),
            field_kind: Some(BridgeFieldKind::Number),
            ..last(BridgeMutationKind::AddField)
        })
        .unwrap();
    let fields = expenses.database().database.unwrap().fields;
    assert_eq!(fields.last().unwrap().id, amount);
}

#[test]
fn a_view_names_its_block_s_kind() {
    let expenses = Expenses::new("database-kind");
    let views = expenses
        .workspace
        .query(BridgeQuery {
            page: Some(expenses.page.clone()),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap()
        .views;
    let kinds: Vec<_> = views.iter().map(|view| view.kind).collect();
    // Lisbon's Text Block, then the Table after it.
    assert_eq!(kinds, [BridgeBlockKind::Text, BridgeBlockKind::Database]);
}

#[test]
fn a_rows_window_never_exceeds_its_cap() {
    let expenses = Expenses::new("database-rows-cap");
    let mut last = None;
    for _ in 0..MAX_WINDOW + 11 {
        last = Some(expenses.row(last.as_deref()));
    }
    let window = expenses
        .rows(BridgeQuery {
            limit: i64::MAX,
            ..query(BridgeQueryKind::Rows)
        })
        .unwrap();
    assert_eq!(window.rows.len() as i64, MAX_WINDOW);
    assert_eq!(window.count, MAX_WINDOW + 11);
}
