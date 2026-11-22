#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, BlockId, BlockKind, Cell, Changed, Created, Database, Error, FieldId,
    FieldKind, Filter, MAX_FIELDS, MAX_FILTER_TEXT, MAX_OPTIONS, Match, MemoryStore, Mutation,
    Origin, PageId, PersonId, Place, Request, RowId, RowQuery, Rows, Sort, TabId, TextStyle,
    ViewId,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn apply(workspace: &mut Workspace, mutation: Mutation) -> Result<Applied, Error> {
    workspace.apply(Request {
        actor: Actor::Person(PersonId::new("benni")),
        origin: Origin::Ui,
        mutation,
    })
}

struct Sheet {
    workspace: Workspace,
    page: PageId,
    block: BlockId,
}

impl Sheet {
    fn new() -> Self {
        let mut workspace =
            Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let Some(Created::Area(area)) = apply(
            &mut workspace,
            Mutation::CreateArea {
                name: "Private".into(),
            },
        )
        .unwrap()
        .created
        else {
            panic!("an area")
        };
        let create = Mutation::CreatePage {
            area,
            parent: None,
            title: "Household budget".into(),
        };
        let Some(Created::Page(page)) = apply(&mut workspace, create).unwrap().created else {
            panic!("a Page")
        };
        let add = Mutation::AddDatabaseBlock {
            page: page.clone(),
            after: None,
            name: "Expenses".into(),
            layout: liquid_core::Layout::Table,
        };
        let Some(Created::Block { block, .. }) = apply(&mut workspace, add).unwrap().created else {
            panic!("a Database Block")
        };
        Self {
            workspace,
            page,
            block,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn database(&mut self) -> Database {
        self.workspace.database(&self.block).unwrap().unwrap()
    }

    fn name_field(&mut self) -> FieldId {
        self.database().fields[0].id.clone()
    }

    fn field(&mut self, name: &str, kind: FieldKind) -> FieldId {
        let add = Mutation::AddField {
            block: self.block.clone(),
            place: Place::Last,
            name: name.into(),
            kind,
        };
        match self.edit(add).unwrap().created {
            Some(Created::Field(field)) => field,
            other => panic!("expected a field, got {other:?}"),
        }
    }

    fn add_row(&mut self, place: Place<RowId>) -> RowId {
        let add = Mutation::AddRow {
            block: self.block.clone(),
            place,
        };
        match self.edit(add).unwrap().created {
            Some(Created::Row(row)) => row,
            other => panic!("expected a row, got {other:?}"),
        }
    }

    fn row(&mut self, after: Option<&RowId>) -> RowId {
        self.add_row(after.map_or(Place::First, |after| Place::After(after.clone())))
    }

    fn set(&mut self, row: &RowId, field: &FieldId, value: Cell) -> Result<Applied, Error> {
        self.edit(Mutation::SetCell {
            block: self.block.clone(),
            row: row.clone(),
            field: field.clone(),
            value,
        })
    }

    fn rows(&mut self, query: &RowQuery) -> Rows {
        self.workspace.rows(&self.block, query).unwrap()
    }

    fn ids(&mut self, query: &RowQuery) -> Vec<RowId> {
        self.rows(query)
            .rows
            .into_iter()
            .map(|row| row.id)
            .collect()
    }

    fn cells(&mut self, query: &RowQuery, field: &FieldId) -> Vec<Cell> {
        self.rows(query)
            .rows
            .into_iter()
            .map(|row| row.cell(field).cloned().unwrap_or(Cell::Empty))
            .collect()
    }

    fn kinds(&mut self) -> Vec<BlockKind> {
        self.workspace
            .views(&self.page, 0, 10)
            .unwrap()
            .into_iter()
            .map(|view| view.kind)
            .collect()
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
    }
}

#[test]
fn a_new_database_has_a_name_field_and_no_rows() {
    let mut sheet = Sheet::new();
    let database = sheet.database();
    assert_eq!(database.name, "Expenses");
    assert_eq!(
        database
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.kind))
            .collect::<Vec<_>>(),
        [("Name", FieldKind::Text)]
    );
    let rows = sheet.rows(&all());
    assert_eq!(rows.count, 0);
    assert!(rows.rows.is_empty());
    assert_eq!(sheet.workspace.view_count(&sheet.page).unwrap(), 1);
}

#[test]
fn fields_and_rows_keep_their_order_and_cells_their_values() {
    let mut sheet = Sheet::new();
    let name = sheet.name_field();
    let amount = sheet.field("Amount", FieldKind::Number);
    let paid = sheet.field("Paid", FieldKind::Checkbox);
    let date = sheet.field("Date", FieldKind::Date);
    let rent = sheet.row(None);
    let food = sheet.row(Some(&rent));
    let train = sheet.row(Some(&rent));
    for (row, item, euros) in [
        (&rent, "Rent", 950.0),
        (&food, "Groceries", 84.2),
        (&train, "Train", 42.5),
    ] {
        sheet.set(row, &name, Cell::Text(item.into())).unwrap();
        sheet.set(row, &amount, Cell::Number(euros)).unwrap();
    }
    sheet.set(&rent, &paid, Cell::Checkbox(true)).unwrap();
    sheet
        .set(&train, &date, Cell::Date("2026-10-28".into()))
        .unwrap();

    assert_eq!(
        sheet
            .database()
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["Name", "Amount", "Paid", "Date"]
    );
    assert_eq!(
        sheet.cells(&all(), &name),
        [
            Cell::Text("Rent".into()),
            Cell::Text("Train".into()),
            Cell::Text("Groceries".into())
        ]
    );
    let rows = sheet.rows(&all()).rows;
    assert_eq!(rows[0].cell(&paid), Some(&Cell::Checkbox(true)));
    assert_eq!(rows[1].cell(&date), Some(&Cell::Date("2026-10-28".into())));
    assert_eq!(rows[2].cell(&paid), None);

    sheet.set(&rent, &paid, Cell::Empty).unwrap();
    assert_eq!(sheet.set(&rent, &paid, Cell::Empty), Err(Error::NoChange));
    assert_eq!(sheet.rows(&all()).rows[0].cell(&paid), None);
}

#[test]
fn a_cell_takes_only_values_of_its_field_s_kind() {
    let mut sheet = Sheet::new();
    let amount = sheet.field("Amount", FieldKind::Number);
    let date = sheet.field("Date", FieldKind::Date);
    let category = sheet.field("Category", FieldKind::Select);
    let rent = sheet.row(None);
    let refused = [
        (&amount, Cell::Text("950".into())),
        (&amount, Cell::Number(f64::NAN)),
        (&amount, Cell::Number(f64::INFINITY)),
        (&date, Cell::Date("28.10.2026".into())),
        (&date, Cell::Date("2026-02-30".into())),
        (&date, Cell::Date("2026-10-28T09:30".into())),
    ];
    for (field, value) in refused {
        assert_eq!(
            sheet.set(&rent, field, value.clone()),
            Err(Error::WrongKind),
            "{value:?}"
        );
    }
    let Some(Created::Option(home)) = sheet
        .edit(Mutation::AddOption {
            block: sheet.block.clone(),
            field: category.clone(),
            name: "Home".into(),
            color: 2,
        })
        .unwrap()
        .created
    else {
        panic!("an option")
    };
    let unknown = liquid_core::OptionId::parse(&"0".repeat(32)).unwrap();
    assert_eq!(
        sheet.set(&rent, &category, Cell::Select(unknown)),
        Err(Error::NotFound)
    );
    sheet
        .set(&rent, &category, Cell::Select(home.clone()))
        .unwrap();
    sheet
        .set(&rent, &date, Cell::Date("2024-02-29".into()))
        .unwrap();
    let database = sheet.database();
    let options = &database
        .fields
        .iter()
        .find(|field| field.id == category)
        .unwrap()
        .options;
    assert_eq!(options.len(), 1);
    assert_eq!(
        (
            options[0].id.clone(),
            options[0].name.as_str(),
            options[0].color
        ),
        (home, "Home", 2)
    );
}

#[test]
fn rows_sort_and_filter_in_windows() {
    let mut sheet = Sheet::new();
    let name = sheet.name_field();
    let amount = sheet.field("Amount", FieldKind::Number);
    let mut last = None;
    for (item, euros) in [
        ("rent", Some(950.0)),
        ("Groceries", Some(84.2)),
        ("Cinema", Some(111.8)),
        ("Gift", None),
        ("Train", Some(42.5)),
    ] {
        let added = sheet.row(last.as_ref());
        sheet.set(&added, &name, Cell::Text(item.into())).unwrap();
        if let Some(euros) = euros {
            sheet.set(&added, &amount, Cell::Number(euros)).unwrap();
        }
        last = Some(added);
    }
    let by = |field: &FieldId, descending| RowQuery {
        sort: Some(Sort {
            field: field.clone(),
            descending,
        }),
        ..all()
    };
    let names = |cells: Vec<Cell>| -> Vec<String> {
        cells
            .into_iter()
            .map(|cell| match cell {
                Cell::Text(text) => text,
                other => panic!("{other:?}"),
            })
            .collect()
    };
    assert_eq!(
        names(sheet.cells(&by(&amount, false), &name)),
        ["Train", "Groceries", "Cinema", "rent", "Gift"]
    );
    assert_eq!(
        names(sheet.cells(&by(&amount, true), &name)),
        ["rent", "Cinema", "Groceries", "Train", "Gift"]
    );
    assert_eq!(
        names(sheet.cells(&by(&name, false), &name)),
        ["Cinema", "Gift", "Groceries", "rent", "Train"]
    );

    let window = RowQuery {
        start: 1,
        limit: 2,
        search: None,
        ..by(&name, false)
    };
    assert_eq!(sheet.rows(&window).count, 5);
    assert_eq!(names(sheet.cells(&window, &name)), ["Gift", "Groceries"]);

    let filtered = |matches| RowQuery {
        filter: Some(Filter {
            field: amount.clone(),
            matches,
        }),
        ..by(&name, false)
    };
    assert_eq!(names(sheet.cells(&filtered(Match::Empty), &name)), ["Gift"]);
    assert_eq!(sheet.rows(&filtered(Match::NotEmpty)).count, 4);
    assert_eq!(
        names(sheet.cells(&filtered(Match::Is(Cell::Number(42.5))), &name)),
        ["Train"]
    );
    let contains = RowQuery {
        filter: Some(Filter {
            field: name.clone(),
            matches: Match::Contains("R".into()),
        }),
        ..by(&name, false)
    };
    assert_eq!(
        names(sheet.cells(&contains, &name)),
        ["Groceries", "rent", "Train"]
    );
}

#[test]
fn removed_rows_and_fields_disappear_and_renames_show() {
    let mut sheet = Sheet::new();
    let amount = sheet.field("Amount", FieldKind::Number);
    let first = sheet.row(None);
    let second = sheet.row(Some(&first));
    sheet.set(&second, &amount, Cell::Number(3.0)).unwrap();
    for mutation in [
        Mutation::RemoveRow {
            block: sheet.block.clone(),
            row: first.clone(),
        },
        Mutation::RemoveField {
            block: sheet.block.clone(),
            field: amount.clone(),
        },
        Mutation::RenameDatabase {
            block: sheet.block.clone(),
            name: "Spending".into(),
        },
    ] {
        sheet.edit(mutation).unwrap();
    }
    assert_eq!(sheet.ids(&all()), std::slice::from_ref(&second));
    assert_eq!(sheet.rows(&all()).rows[0].cell(&amount), None);
    let database = sheet.database();
    assert_eq!(database.name, "Spending");
    assert_eq!(database.fields.len(), 1);
    assert_eq!(
        sheet.set(&second, &amount, Cell::Number(1.0)),
        Err(Error::NotFound)
    );
    let name = database.fields[0].id.clone();
    assert_eq!(
        sheet.set(&first, &name, Cell::Text("x".into())),
        Err(Error::NotFound)
    );
}

#[test]
fn a_database_takes_no_text_writes_or_style() {
    let mut sheet = Sheet::new();
    let applied = sheet
        .edit(Mutation::AddRow {
            block: sheet.block.clone(),
            place: Place::First,
        })
        .unwrap();
    assert_eq!(applied.changed, [Changed::Table(sheet.block.clone())]);
    assert_eq!(
        sheet.workspace.text(&sheet.page, &sheet.block).unwrap(),
        None
    );
    assert_eq!(
        sheet.workspace.style(&sheet.page, &sheet.block).unwrap(),
        None
    );
    assert_eq!(
        sheet.edit(Mutation::EditText {
            page: sheet.page.clone(),
            block: sheet.block.clone(),
            at: 0,
            delete: 0,
            insert: "x".into(),
        }),
        Err(Error::NotFound)
    );
    assert_eq!(
        sheet.edit(Mutation::SetTextStyle {
            page: sheet.page.clone(),
            block: sheet.block.clone(),
            style: TextStyle::Quote,
        }),
        Err(Error::NotFound)
    );
}

#[test]
fn rows_read_the_same_after_their_page_is_loaded_again() {
    let mut sheet = Sheet::new();
    let name = sheet.name_field();
    let first = sheet.row(None);
    let second = sheet.row(Some(&first));
    sheet.set(&first, &name, Cell::Text("Rent".into())).unwrap();
    sheet
        .set(&second, &name, Cell::Text("Food".into()))
        .unwrap();
    let before = sheet.rows(&all());
    sheet.workspace.close_page(&sheet.page);
    assert_eq!(sheet.rows(&all()), before);
    let third = sheet.row(Some(&first));
    assert_eq!(sheet.ids(&all()), [first, third, second]);
}

#[test]
fn zero_is_one_value_whatever_its_sign() {
    let mut sheet = Sheet::new();
    let amount = sheet.field("Amount", FieldKind::Number);
    let first = sheet.row(None);
    let second = sheet.row(Some(&first));
    sheet.set(&first, &amount, Cell::Number(0.0)).unwrap();
    sheet.set(&second, &amount, Cell::Number(-0.0)).unwrap();
    assert_eq!(
        sheet.set(&first, &amount, Cell::Number(-0.0)),
        Err(Error::NoChange)
    );
    let sorted = RowQuery {
        sort: Some(Sort {
            field: amount.clone(),
            descending: false,
        }),
        ..all()
    };
    let order = sheet.ids(&sorted);
    assert_eq!(order, [first.clone(), second.clone()]);
    let zero = RowQuery {
        filter: Some(Filter {
            field: amount.clone(),
            matches: Match::Is(Cell::Number(-0.0)),
        }),
        ..all()
    };
    assert_eq!(sheet.rows(&zero).count, 2);
    sheet.workspace.close_page(&sheet.page);
    assert_eq!(sheet.ids(&sorted), order);
}

#[test]
fn every_cell_equals_itself() {
    let nan = Cell::Number(f64::NAN);
    assert_eq!(nan, nan.clone());
    assert_ne!(Cell::Number(1.0), Cell::Text("1".into()));
}

#[test]
fn a_database_holds_a_bounded_number_of_fields_and_options() {
    let mut sheet = Sheet::new();
    let block = sheet.block.clone();
    for number in 1..MAX_FIELDS {
        sheet
            .edit(Mutation::AddField {
                block: block.clone(),
                place: Place::First,
                name: format!("Field {number}"),
                kind: FieldKind::Select,
            })
            .unwrap();
    }
    let one_more = Mutation::AddField {
        block: block.clone(),
        place: Place::First,
        name: "One more".into(),
        kind: FieldKind::Text,
    };
    assert_eq!(sheet.edit(one_more.clone()), Err(Error::TooMany));
    let fields = sheet.database().fields;
    assert_eq!(fields.len(), MAX_FIELDS);
    sheet
        .edit(Mutation::RemoveField {
            block: block.clone(),
            field: fields[0].id.clone(),
        })
        .unwrap();
    sheet.edit(one_more).unwrap();

    let select = fields[1].id.clone();
    let option = |number: usize| Mutation::AddOption {
        block: block.clone(),
        field: select.clone(),
        name: format!("Option {number}"),
        color: 0,
    };
    for number in 0..MAX_OPTIONS {
        sheet.edit(option(number)).unwrap();
    }
    assert_eq!(sheet.edit(option(MAX_OPTIONS)), Err(Error::TooMany));
}

#[test]
fn a_view_names_the_kind_of_its_block() {
    let mut sheet = Sheet::new();
    let table = sheet.workspace.views(&sheet.page, 0, 10).unwrap()[0]
        .id
        .clone();
    sheet
        .edit(Mutation::AddTextBlock {
            page: sheet.page.clone(),
            after: Some(table),
            text: "October".into(),
        })
        .unwrap();
    assert_eq!(sheet.kinds(), [BlockKind::Database, BlockKind::Text]);
    sheet.workspace.close_page(&sheet.page);
    assert_eq!(sheet.kinds(), [BlockKind::Database, BlockKind::Text]);
}

#[test]
fn rows_and_fields_added_last_go_after_all_others_when_applied() {
    let mut sheet = Sheet::new();
    let name = sheet.name_field();
    let a = sheet.add_row(Place::Last);
    sheet.set(&a, &name, Cell::Text("A".into())).unwrap();
    let b = sheet.add_row(Place::Last);
    sheet.set(&b, &name, Cell::Text("B".into())).unwrap();
    let c = sheet.add_row(Place::After(a.clone()));
    sheet.set(&c, &name, Cell::Text("C".into())).unwrap();
    let d = sheet.add_row(Place::Last);
    sheet.set(&d, &name, Cell::Text("D".into())).unwrap();
    let e = sheet.add_row(Place::First);
    sheet.set(&e, &name, Cell::Text("E".into())).unwrap();
    assert_eq!(
        sheet.cells(&all(), &name),
        ["E", "A", "C", "B", "D"].map(|text| Cell::Text(text.into())),
    );

    sheet.field("Amount", FieldKind::Number);
    sheet
        .edit(Mutation::AddField {
            block: sheet.block.clone(),
            place: Place::First,
            name: "Paid".into(),
            kind: FieldKind::Checkbox,
        })
        .unwrap();
    sheet.field("Date", FieldKind::Date);
    let names: Vec<_> = sheet
        .database()
        .fields
        .into_iter()
        .map(|field| field.name)
        .collect();
    assert_eq!(names, ["Paid", "Name", "Amount", "Date"]);
}

#[test]
fn actions_stored_before_places_still_read() {
    let mut sheet = Sheet::new();
    let row = sheet.add_row(Place::Last);
    let block = sheet.block.clone();
    for (after, place) in [
        (serde_json::Value::Null, Place::First),
        (
            serde_json::json!(row.to_string()),
            Place::After(row.clone()),
        ),
    ] {
        let mut stored = serde_json::to_value(Mutation::AddRow {
            block: block.clone(),
            place: Place::Last,
        })
        .unwrap();
        let fields = stored["AddRow"].as_object_mut().unwrap();
        fields.remove("place");
        fields.insert("after".into(), after);
        let read: Mutation = serde_json::from_value(stored).unwrap();
        assert_eq!(
            read,
            Mutation::AddRow {
                block: block.clone(),
                place,
            }
        );
    }
    let name = sheet.name_field();
    for (after, place) in [
        (serde_json::Value::Null, Place::First),
        (
            serde_json::json!(name.to_string()),
            Place::After(name.clone()),
        ),
    ] {
        let add = |place| Mutation::AddField {
            block: block.clone(),
            place,
            name: "Amount".into(),
            kind: FieldKind::Number,
        };
        let mut stored = serde_json::to_value(add(Place::Last)).unwrap();
        let fields = stored["AddField"].as_object_mut().unwrap();
        fields.remove("place");
        fields.insert("after".into(), after);
        let read: Mutation = serde_json::from_value(stored).unwrap();
        assert_eq!(read, add(place));
    }
}

fn first_tab(workspace: &mut Workspace, page: &PageId, view: &ViewId) -> TabId {
    let shown = workspace.view(page, view).unwrap().unwrap();
    shown
        .tabs
        .first()
        .map(|tab| tab.id.clone())
        .unwrap_or_default()
}

fn set_sort(
    workspace: &mut Workspace,
    page: &PageId,
    view: &ViewId,
    sort: Option<Sort>,
) -> Result<Applied, Error> {
    let set = Mutation::SetTabSort {
        page: page.clone(),
        view: view.clone(),
        tab: first_tab(workspace, page, view),
        sort,
    };
    apply(workspace, set)
}

fn set_filter(
    workspace: &mut Workspace,
    page: &PageId,
    view: &ViewId,
    field: &FieldId,
    matches: Match,
) -> Result<Applied, Error> {
    let filter = Some(Filter {
        field: field.clone(),
        matches,
    });
    let set = Mutation::SetTabFilter {
        page: page.clone(),
        view: view.clone(),
        tab: first_tab(workspace, page, view),
        filter,
    };
    apply(workspace, set)
}

#[test]
fn a_view_takes_only_a_sort_and_filter_its_table_can_answer() {
    let mut sheet = Sheet::new();
    let amount = sheet.field("Amount", FieldKind::Number);
    let note = sheet.field("Note", FieldKind::Text);
    let files = sheet.field("Files", FieldKind::File);
    let text = Mutation::AddTextBlock {
        page: sheet.page.clone(),
        after: None,
        text: "Above".into(),
    };
    apply(&mut sheet.workspace, text).unwrap();
    let [paragraph, table] =
        <[_; 2]>::try_from(sheet.workspace.views(&sheet.page, 0, 2).unwrap()).unwrap();
    let id = table.id;
    let on = |field: &FieldId| Sort {
        field: field.clone(),
        descending: false,
    };
    assert_eq!(
        set_sort(
            &mut sheet.workspace,
            &sheet.page,
            &paragraph.id,
            Some(on(&amount))
        )
        .unwrap_err(),
        Error::WrongKind
    );
    assert_eq!(
        set_sort(&mut sheet.workspace, &sheet.page, &id, Some(on(&files))).unwrap_err(),
        Error::WrongKind
    );
    let refused = [
        (&amount, Match::Is(Cell::Text("5".into())), Error::WrongKind),
        (&amount, Match::Is(Cell::Number(f64::NAN)), Error::WrongKind),
        (&amount, Match::Is(Cell::Empty), Error::WrongKind),
        (&amount, Match::Contains("5".into()), Error::WrongKind),
        (&files, Match::Is(Cell::Files(Vec::new())), Error::WrongKind),
        (
            &note,
            Match::Contains("x".repeat(MAX_FILTER_TEXT + 1)),
            Error::TooLong,
        ),
    ];
    for (field, matches, error) in refused {
        assert_eq!(
            set_filter(&mut sheet.workspace, &sheet.page, &id, field, matches).unwrap_err(),
            error
        );
    }
    set_filter(
        &mut sheet.workspace,
        &sheet.page,
        &id,
        &files,
        Match::NotEmpty,
    )
    .unwrap();
    set_filter(
        &mut sheet.workspace,
        &sheet.page,
        &id,
        &note,
        Match::Contains("x".into()),
    )
    .unwrap();
}

#[test]
fn a_removed_field_no_longer_sorts_or_filters() {
    let mut sheet = Sheet::new();
    let amount = sheet.field("Amount", FieldKind::Number);
    let name = sheet
        .workspace
        .database(&sheet.block)
        .unwrap()
        .unwrap()
        .fields[0]
        .id
        .clone();
    for label in ["a", "b"] {
        let added = sheet.row(None);
        sheet.set(&added, &name, Cell::Text(label.into())).unwrap();
    }
    let remove = Mutation::RemoveField {
        block: sheet.block.clone(),
        field: amount.clone(),
    };
    apply(&mut sheet.workspace, remove).unwrap();
    let query = RowQuery {
        filter: Some(Filter {
            field: amount,
            matches: Match::NotEmpty,
        }),
        ..all()
    };
    assert_eq!(sheet.workspace.rows(&sheet.block, &query).unwrap().count, 2);
}

#[test]
fn an_unticked_filter_finds_boxes_never_ticked() {
    let mut sheet = Sheet::new();
    let paid = sheet.field("Paid", FieldKind::Checkbox);
    let never = sheet.row(None);
    let ticked = sheet.row(None);
    sheet.set(&ticked, &paid, Cell::Checkbox(true)).unwrap();
    let query = RowQuery {
        filter: Some(Filter {
            field: paid,
            matches: Match::Is(Cell::Checkbox(false)),
        }),
        ..all()
    };
    let rows = sheet.workspace.rows(&sheet.block, &query).unwrap().rows;
    assert_eq!(rows.iter().map(|row| &row.id).collect::<Vec<_>>(), [&never]);
}

#[test]
fn a_moved_row_keeps_its_cells_at_its_new_place() {
    let mut sheet = Sheet::new();
    let name = sheet.name_field();
    let a = sheet.row(None);
    let b = sheet.row(Some(&a));
    let c = sheet.row(Some(&b));
    sheet.set(&c, &name, Cell::Text("Rent".into())).unwrap();
    let mut moved = |row: &RowId, place| {
        sheet.edit(Mutation::MoveRow {
            block: sheet.block.clone(),
            row: row.clone(),
            place,
        })
    };
    moved(&c, Place::First).unwrap();
    moved(&a, Place::After(b.clone())).unwrap();
    let missing = RowId::parse(&"0".repeat(32)).unwrap();
    assert_eq!(moved(&missing, Place::Last), Err(Error::NotFound));
    sheet.workspace.close_page(&sheet.page);
    assert_eq!(sheet.ids(&all()), [c.clone(), b, a]);
    assert_eq!(sheet.cells(&all(), &name)[0], Cell::Text("Rent".into()));
}
