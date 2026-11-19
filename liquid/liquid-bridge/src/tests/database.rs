use std::sync::{Arc, Mutex};

use super::workspace::{Folder, Memory, lisbon, mutation, query, resource};
use crate::api::database::{
    BridgeCell, BridgeCellKind, BridgeFieldKind, BridgeFilter, BridgeMatch, BridgeSort,
};
use crate::api::replica::LiquidWorkspace;
use crate::api::workspace::{
    BridgeAnswer, BridgeBlockKind, BridgeMutation, BridgeMutationKind, BridgeQuery,
    BridgeQueryKind, BridgeResource, BridgeResourceKind, BridgeView,
};

fn cell(kind: BridgeCellKind) -> BridgeCell {
    BridgeCell {
        kind,
        text: None,
        number: 0.0,
        checked: false,
        files: Vec::new(),
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
fn a_table_write_reaches_its_subscribers_until_it_closes() {
    let expenses = Expenses::new("database-notify");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let block = BridgeResource {
        block: Some(expenses.block.clone()),
        ..resource(BridgeResourceKind::Table)
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
    expenses.set(&rent, &expenses.name, text("Flat")).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [0, 1, 2],
        "a closed subscription hears nothing more"
    );
    let refused = expenses.set(&rent, &expenses.name, number(1.0));
    assert_eq!(
        refused.unwrap_err(),
        "the value is not of its kind or not valid"
    );
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
fn field_row_and_code_changes_map_to_their_core_mutations() {
    let expenses = Expenses::new("database-mapping");
    let rows = |e: &Expenses| e.rows(query(BridgeQueryKind::Rows)).unwrap().rows;
    let fields = |e: &Expenses| e.database().database.unwrap().fields;
    let views = |e: &Expenses| {
        e.workspace
            .query(BridgeQuery {
                page: Some(e.page.clone()),
                ..query(BridgeQueryKind::Views)
            })
            .unwrap()
            .views
    };
    let cost = expenses.field(&expenses.name, "Cost", BridgeFieldKind::Number);
    let row = expenses.row(None);
    assert_eq!(rows(&expenses)[0].id, row);

    expenses
        .apply(BridgeMutation {
            field: Some(cost.clone()),
            text: Some("Price".into()),
            ..mutation(BridgeMutationKind::RenameField)
        })
        .unwrap();
    assert_eq!(
        fields(&expenses)
            .iter()
            .find(|field| field.id == cost)
            .unwrap()
            .name,
        "Price"
    );

    expenses
        .apply(BridgeMutation {
            field: Some(cost.clone()),
            ..mutation(BridgeMutationKind::RemoveField)
        })
        .unwrap();
    assert!(fields(&expenses).iter().all(|field| field.id != cost));

    expenses
        .apply(BridgeMutation {
            row: Some(row),
            ..mutation(BridgeMutationKind::RemoveRow)
        })
        .unwrap();
    assert!(rows(&expenses).is_empty());

    let paragraph = views(&expenses)[0].block.clone();
    let on_paragraph = |kind| BridgeMutation {
        page: Some(expenses.page.clone()),
        block: Some(paragraph.clone()),
        ..mutation(kind)
    };
    expenses
        .workspace
        .apply(on_paragraph(BridgeMutationKind::TurnIntoCode))
        .unwrap();
    expenses
        .workspace
        .apply(BridgeMutation {
            text: Some("rust".into()),
            ..on_paragraph(BridgeMutationKind::SetCodeLanguage)
        })
        .unwrap();
    let code = expenses
        .workspace
        .query(BridgeQuery {
            page: Some(expenses.page.clone()),
            block: Some(paragraph),
            ..query(BridgeQueryKind::Code)
        })
        .unwrap();
    assert_eq!(code.language.as_deref(), Some("rust"));
    assert_eq!(
        views(&expenses)
            .iter()
            .map(|view| view.kind)
            .collect::<Vec<_>>(),
        [BridgeBlockKind::Code, BridgeBlockKind::Database],
    );
}

#[test]
fn a_view_s_sort_and_filter_cross_the_bridge() {
    let expenses = Expenses::new("database-view-query");
    let view = |answer: BridgeAnswer| {
        let table = |view: &BridgeView| view.kind == BridgeBlockKind::Database;
        answer.views.into_iter().find(table).unwrap()
    };
    let read = || {
        expenses
            .workspace
            .query(BridgeQuery {
                page: Some(expenses.page.clone()),
                ..query(BridgeQueryKind::Views)
            })
            .unwrap()
    };
    let id = view(read()).id;
    let sort = BridgeSort {
        field: expenses.name.clone(),
        descending: true,
    };
    let filter = BridgeFilter {
        field: expenses.name.clone(),
        matches: BridgeMatch::Contains,
        value: Some(text("rent")),
    };
    expenses
        .apply(BridgeMutation {
            view: Some(id.clone()),
            sort: Some(sort.clone()),
            ..mutation(BridgeMutationKind::SetViewSort)
        })
        .unwrap();
    expenses
        .apply(BridgeMutation {
            view: Some(id),
            filter: Some(filter.clone()),
            ..mutation(BridgeMutationKind::SetViewFilter)
        })
        .unwrap();
    let shown = view(read());
    assert_eq!(shown.sort, Some(sort));
    assert_eq!(shown.filter, Some(filter));
}

#[test]
fn a_table_shows_on_two_pages_keeps_its_name_and_goes_with_all_its_views() {
    let expenses = Expenses::new("table-views");
    let tables = || {
        expenses
            .workspace
            .query(query(BridgeQueryKind::Tables))
            .unwrap()
            .tables
    };
    let listed = tables();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].id.as_str(), listed[0].name.as_str()),
        (expenses.block.as_str(), "Expenses")
    );

    let review = expenses
        .workspace
        .apply(BridgeMutation {
            area: Some(listed[0].area.clone()),
            text: Some("Monthly review".into()),
            ..mutation(BridgeMutationKind::CreatePage)
        })
        .unwrap()
        .page
        .unwrap();
    let view = expenses
        .workspace
        .apply(BridgeMutation {
            page: Some(review.clone()),
            block: Some(expenses.block.clone()),
            text: Some("All costs".into()),
            sort: Some(BridgeSort {
                field: expenses.name.clone(),
                descending: true,
            }),
            ..mutation(BridgeMutationKind::AddTableView)
        })
        .unwrap()
        .view
        .unwrap();
    expenses
        .workspace
        .apply(BridgeMutation {
            page: Some(review.clone()),
            view: Some(view.clone()),
            text: Some("Costs".into()),
            ..mutation(BridgeMutationKind::RenameView)
        })
        .unwrap();
    let shown = expenses
        .workspace
        .query(BridgeQuery {
            page: Some(review.clone()),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap()
        .views
        .remove(0);
    assert_eq!(
        (
            shown.block.as_str(),
            shown.title.as_str(),
            shown.sort.is_some()
        ),
        (expenses.block.as_str(), "Costs", true)
    );
    let mut pages: Vec<_> = expenses
        .workspace
        .query(BridgeQuery {
            block: Some(expenses.block.clone()),
            ..query(BridgeQueryKind::TableViews)
        })
        .unwrap()
        .shown
        .into_iter()
        .map(|shown| shown.page)
        .collect();
    pages.sort();
    let mut expected = vec![expenses.page.clone(), review.clone()];
    expected.sort();
    assert_eq!(pages, expected);

    expenses
        .workspace
        .apply(BridgeMutation {
            block: Some(expenses.block.clone()),
            ..mutation(BridgeMutationKind::DeleteTable)
        })
        .unwrap();
    assert!(tables().is_empty());
    let left = expenses
        .workspace
        .query(BridgeQuery {
            page: Some(review),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap();
    assert_eq!(left.count, 0);
    assert!(
        !expenses
            .workspace
            .is_loaded(&liquid_core::PageId::parse(&expenses.page).unwrap())
    );
}
