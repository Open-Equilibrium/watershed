//! Database Blocks: typed fields and rows in their home Page's document, written through the
//! one pipeline and read in sorted, filtered windows that Rust computes (client architecture
//! rule 13).

use liquid_core::{
    Actor, Applied, BlockId, BlockKind, Cell, Created, Error, FieldId, FieldKind, MAX_FIELDS,
    MAX_OPTIONS, Match, MemoryStore, Mutation, Origin, PageId, PersonId, Place, Request, RowId,
    RowQuery, Sort,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn apply(workspace: &mut Workspace, mutation: Mutation) -> Result<Applied, Error> {
    workspace.apply(Request {
        actor: Actor::Person(PersonId::new("benni")),
        origin: Origin::Ui,
        mutation,
    })
}

/// A Workspace with one Page holding a Database Block named "Expenses".
fn expenses() -> (Workspace, PageId, BlockId) {
    let mut workspace = Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
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
    };
    let Some(Created::Block { block, .. }) = apply(&mut workspace, add).unwrap().created else {
        panic!("a Database Block")
    };
    (workspace, page, block)
}

fn field(
    workspace: &mut Workspace,
    page: &PageId,
    block: &BlockId,
    name: &str,
    kind: FieldKind,
) -> FieldId {
    // Last, as the header's + adds one.
    let add = Mutation::AddField {
        page: page.clone(),
        block: block.clone(),
        place: Place::Last,
        name: name.into(),
        kind,
    };
    match apply(workspace, add).unwrap().created {
        Some(Created::Field(field)) => field,
        other => panic!("expected a field, got {other:?}"),
    }
}

fn row(workspace: &mut Workspace, page: &PageId, block: &BlockId, after: Option<&RowId>) -> RowId {
    let place = after.map_or(Place::First, |after| Place::After(after.clone()));
    add_row(workspace, page, block, place)
}

fn add_row(
    workspace: &mut Workspace,
    page: &PageId,
    block: &BlockId,
    place: Place<RowId>,
) -> RowId {
    let add = Mutation::AddRow {
        page: page.clone(),
        block: block.clone(),
        place,
    };
    match apply(workspace, add).unwrap().created {
        Some(Created::Row(row)) => row,
        other => panic!("expected a row, got {other:?}"),
    }
}

fn set(
    workspace: &mut Workspace,
    page: &PageId,
    block: &BlockId,
    row: &RowId,
    field: &FieldId,
    value: Cell,
) -> Result<Applied, Error> {
    apply(
        workspace,
        Mutation::SetCell {
            page: page.clone(),
            block: block.clone(),
            row: row.clone(),
            field: field.clone(),
            value,
        },
    )
}

fn all() -> RowQuery {
    RowQuery {
        sort: None,
        filter: None,
        start: 0,
        limit: usize::MAX,
    }
}

/// The text of `field` in each row of `query`'s window.
fn column(
    workspace: &mut Workspace,
    page: &PageId,
    block: &BlockId,
    query: &RowQuery,
    field: &FieldId,
) -> Vec<Cell> {
    workspace
        .rows(page, block, query)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.cell(field).cloned().unwrap_or(Cell::Empty))
        .collect()
}

#[test]
fn a_new_database_has_a_name_field_and_no_rows() {
    let (mut workspace, page, block) = expenses();
    let database = workspace.database(&page, &block).unwrap().unwrap();
    assert_eq!(database.name, "Expenses");
    assert_eq!(
        database
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.kind))
            .collect::<Vec<_>>(),
        [("Name", FieldKind::Text)]
    );
    let rows = workspace.rows(&page, &block, &all()).unwrap();
    assert_eq!(rows.count, 0);
    assert!(rows.rows.is_empty());
    // A Database shows on its Page like any Block.
    assert_eq!(workspace.view_count(&page).unwrap(), 1);
}

#[test]
fn fields_and_rows_keep_their_order_and_cells_their_values() {
    let (mut workspace, page, block) = expenses();
    let name = workspace.database(&page, &block).unwrap().unwrap().fields[0]
        .id
        .clone();
    let amount = field(&mut workspace, &page, &block, "Amount", FieldKind::Number);
    let paid = field(&mut workspace, &page, &block, "Paid", FieldKind::Checkbox);
    let date = field(&mut workspace, &page, &block, "Date", FieldKind::Date);
    let rent = row(&mut workspace, &page, &block, None);
    let food = row(&mut workspace, &page, &block, Some(&rent));
    let train = row(&mut workspace, &page, &block, Some(&rent));
    for (row, item, euros) in [
        (&rent, "Rent", 950.0),
        (&food, "Groceries", 84.2),
        (&train, "Train", 42.5),
    ] {
        set(
            &mut workspace,
            &page,
            &block,
            row,
            &name,
            Cell::Text(item.into()),
        )
        .unwrap();
        set(
            &mut workspace,
            &page,
            &block,
            row,
            &amount,
            Cell::Number(euros),
        )
        .unwrap();
    }
    set(
        &mut workspace,
        &page,
        &block,
        &rent,
        &paid,
        Cell::Checkbox(true),
    )
    .unwrap();
    set(
        &mut workspace,
        &page,
        &block,
        &train,
        &date,
        Cell::Date("2026-10-28".into()),
    )
    .unwrap();

    let fields: Vec<_> = workspace.database(&page, &block).unwrap().unwrap().fields;
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["Name", "Amount", "Paid", "Date"]
    );
    assert_eq!(
        column(&mut workspace, &page, &block, &all(), &name),
        [
            Cell::Text("Rent".into()),
            Cell::Text("Train".into()),
            Cell::Text("Groceries".into())
        ]
    );
    let rows = workspace.rows(&page, &block, &all()).unwrap().rows;
    assert_eq!(rows[0].cell(&paid), Some(&Cell::Checkbox(true)));
    assert_eq!(rows[1].cell(&date), Some(&Cell::Date("2026-10-28".into())));
    assert_eq!(rows[2].cell(&paid), None);

    // Clearing a cell leaves it empty, and an unchanged value records nothing.
    set(&mut workspace, &page, &block, &rent, &paid, Cell::Empty).unwrap();
    assert_eq!(
        set(&mut workspace, &page, &block, &rent, &paid, Cell::Empty),
        Err(Error::NoChange)
    );
    assert_eq!(
        workspace.rows(&page, &block, &all()).unwrap().rows[0].cell(&paid),
        None
    );
}

#[test]
fn a_cell_takes_only_values_of_its_field_s_kind() {
    let (mut workspace, page, block) = expenses();
    let amount = field(&mut workspace, &page, &block, "Amount", FieldKind::Number);
    let date = field(&mut workspace, &page, &block, "Date", FieldKind::Date);
    let category = field(&mut workspace, &page, &block, "Category", FieldKind::Select);
    let rent = row(&mut workspace, &page, &block, None);
    let refused = [
        (&amount, Cell::Text("950".into())),
        (&amount, Cell::Number(f64::NAN)),
        (&amount, Cell::Number(f64::INFINITY)),
        (&date, Cell::Date("28.10.2026".into())),
        (&date, Cell::Date("2026-02-30".into())),
        // Only days, until the times' zones are decided.
        (&date, Cell::Date("2026-10-28T09:30".into())),
    ];
    for (field, value) in refused {
        assert_eq!(
            set(&mut workspace, &page, &block, &rent, field, value.clone()),
            Err(Error::WrongKind),
            "{value:?}"
        );
    }
    // A select cell names one of its field's options.
    let elsewhere = Mutation::AddOption {
        page: page.clone(),
        block: block.clone(),
        field: category.clone(),
        name: "Home".into(),
        color: 2,
    };
    let Some(Created::Option(home)) = apply(&mut workspace, elsewhere).unwrap().created else {
        panic!("an option")
    };
    let unknown = liquid_core::OptionId::parse(&"0".repeat(32)).unwrap();
    assert_eq!(
        set(
            &mut workspace,
            &page,
            &block,
            &rent,
            &category,
            Cell::Select(unknown)
        ),
        Err(Error::NotFound)
    );
    set(
        &mut workspace,
        &page,
        &block,
        &rent,
        &category,
        Cell::Select(home.clone()),
    )
    .unwrap();
    set(
        &mut workspace,
        &page,
        &block,
        &rent,
        &date,
        Cell::Date("2024-02-29".into()),
    )
    .unwrap();
    let database = workspace.database(&page, &block).unwrap().unwrap();
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
    let (mut workspace, page, block) = expenses();
    let name = workspace.database(&page, &block).unwrap().unwrap().fields[0]
        .id
        .clone();
    let amount = field(&mut workspace, &page, &block, "Amount", FieldKind::Number);
    let mut last = None;
    for (item, euros) in [
        ("rent", Some(950.0)),
        ("Groceries", Some(84.2)),
        ("Cinema", Some(111.8)),
        ("Gift", None),
        ("Train", Some(42.5)),
    ] {
        let added = row(&mut workspace, &page, &block, last.as_ref());
        set(
            &mut workspace,
            &page,
            &block,
            &added,
            &name,
            Cell::Text(item.into()),
        )
        .unwrap();
        if let Some(euros) = euros {
            set(
                &mut workspace,
                &page,
                &block,
                &added,
                &amount,
                Cell::Number(euros),
            )
            .unwrap();
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
    // Numbers sort by value, empty cells last either way.
    let ascending = column(&mut workspace, &page, &block, &by(&amount, false), &name);
    assert_eq!(
        names(ascending),
        ["Train", "Groceries", "Cinema", "rent", "Gift"]
    );
    let descending = column(&mut workspace, &page, &block, &by(&amount, true), &name);
    assert_eq!(
        names(descending),
        ["rent", "Cinema", "Groceries", "Train", "Gift"]
    );
    // Text sorts ignoring case.
    let alphabetical = column(&mut workspace, &page, &block, &by(&name, false), &name);
    assert_eq!(
        names(alphabetical),
        ["Cinema", "Gift", "Groceries", "rent", "Train"]
    );

    // A window of the sorted rows, with the count of all that match.
    let window = RowQuery {
        start: 1,
        limit: 2,
        ..by(&name, false)
    };
    let rows = workspace.rows(&page, &block, &window).unwrap();
    assert_eq!(rows.count, 5);
    assert_eq!(
        names(column(&mut workspace, &page, &block, &window, &name)),
        ["Gift", "Groceries"]
    );

    // Filters.
    let filtered = |matches| RowQuery {
        filter: Some(liquid_core::Filter {
            field: amount.clone(),
            matches,
        }),
        ..by(&name, false)
    };
    let empty = filtered(Match::Empty);
    assert_eq!(
        names(column(&mut workspace, &page, &block, &empty, &name)),
        ["Gift"]
    );
    assert_eq!(
        workspace
            .rows(&page, &block, &filtered(Match::NotEmpty))
            .unwrap()
            .count,
        4
    );
    let exact = filtered(Match::Is(Cell::Number(42.5)));
    assert_eq!(
        names(column(&mut workspace, &page, &block, &exact, &name)),
        ["Train"]
    );
    let contains = RowQuery {
        filter: Some(liquid_core::Filter {
            field: name.clone(),
            matches: Match::Contains("R".into()),
        }),
        ..by(&name, false)
    };
    assert_eq!(
        names(column(&mut workspace, &page, &block, &contains, &name)),
        ["Groceries", "rent", "Train"]
    );
}

#[test]
fn removed_rows_and_fields_disappear_and_renames_show() {
    let (mut workspace, page, block) = expenses();
    let amount = field(&mut workspace, &page, &block, "Amount", FieldKind::Number);
    let first = row(&mut workspace, &page, &block, None);
    let second = row(&mut workspace, &page, &block, Some(&first));
    set(
        &mut workspace,
        &page,
        &block,
        &second,
        &amount,
        Cell::Number(3.0),
    )
    .unwrap();
    for mutation in [
        Mutation::RemoveRow {
            page: page.clone(),
            block: block.clone(),
            row: first.clone(),
        },
        Mutation::RemoveField {
            page: page.clone(),
            block: block.clone(),
            field: amount.clone(),
        },
        Mutation::RenameDatabase {
            page: page.clone(),
            block: block.clone(),
            name: "Spending".into(),
        },
    ] {
        apply(&mut workspace, mutation).unwrap();
    }
    let rows = workspace.rows(&page, &block, &all()).unwrap();
    assert_eq!(
        rows.rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>(),
        std::slice::from_ref(&second)
    );
    assert_eq!(rows.rows[0].cell(&amount), None);
    let database = workspace.database(&page, &block).unwrap().unwrap();
    assert_eq!(database.name, "Spending");
    assert_eq!(database.fields.len(), 1);
    // A removed field takes no values, and a removed row none either.
    assert_eq!(
        set(
            &mut workspace,
            &page,
            &block,
            &second,
            &amount,
            Cell::Number(1.0)
        ),
        Err(Error::NotFound)
    );
    let name = database.fields[0].id.clone();
    assert_eq!(
        set(
            &mut workspace,
            &page,
            &block,
            &first,
            &name,
            Cell::Text("x".into())
        ),
        Err(Error::NotFound)
    );
}

#[test]
fn database_writes_change_only_their_block_and_its_text_reads_none() {
    let (mut workspace, page, block) = expenses();
    let applied = apply(
        &mut workspace,
        Mutation::AddRow {
            page: page.clone(),
            block: block.clone(),
            place: Place::First,
        },
    )
    .unwrap();
    assert_eq!(
        applied.changed,
        [liquid_core::Changed::Block(page.clone(), block.clone())]
    );
    // A Database is no Text Block.
    assert_eq!(workspace.text(&page, &block).unwrap(), None);
    assert_eq!(
        apply(
            &mut workspace,
            Mutation::EditText {
                page: page.clone(),
                block: block.clone(),
                at: 0,
                delete: 0,
                insert: "x".into(),
            },
        ),
        Err(Error::NotFound)
    );
}

#[test]
fn rows_read_the_same_after_their_page_is_loaded_again() {
    let (mut workspace, page, block) = expenses();
    let name = workspace.database(&page, &block).unwrap().unwrap().fields[0]
        .id
        .clone();
    let first = row(&mut workspace, &page, &block, None);
    let second = row(&mut workspace, &page, &block, Some(&first));
    set(
        &mut workspace,
        &page,
        &block,
        &first,
        &name,
        Cell::Text("Rent".into()),
    )
    .unwrap();
    set(
        &mut workspace,
        &page,
        &block,
        &second,
        &name,
        Cell::Text("Food".into()),
    )
    .unwrap();
    let before = workspace.rows(&page, &block, &all()).unwrap();
    workspace.close_page(&page);
    assert_eq!(workspace.rows(&page, &block, &all()).unwrap(), before);
    // The index read back from the document places new rows as before.
    let third = row(&mut workspace, &page, &block, Some(&first));
    let order: Vec<_> = workspace
        .rows(&page, &block, &all())
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(order, [first, third, second]);
}

#[test]
fn zero_is_one_value_whatever_its_sign() {
    let (mut workspace, page, block) = expenses();
    let amount = field(&mut workspace, &page, &block, "Amount", FieldKind::Number);
    let first = row(&mut workspace, &page, &block, None);
    let second = row(&mut workspace, &page, &block, Some(&first));
    set(
        &mut workspace,
        &page,
        &block,
        &first,
        &amount,
        Cell::Number(0.0),
    )
    .unwrap();
    set(
        &mut workspace,
        &page,
        &block,
        &second,
        &amount,
        Cell::Number(-0.0),
    )
    .unwrap();
    // The sign of zero changes nothing, so the second write is no change either.
    assert_eq!(
        set(
            &mut workspace,
            &page,
            &block,
            &first,
            &amount,
            Cell::Number(-0.0)
        ),
        Err(Error::NoChange)
    );
    let sorted = RowQuery {
        sort: Some(Sort {
            field: amount.clone(),
            descending: false,
        }),
        ..all()
    };
    let order: Vec<_> = workspace
        .rows(&page, &block, &sorted)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(order, [first.clone(), second.clone()]);
    // A filter for either zero finds both.
    let zero = RowQuery {
        filter: Some(liquid_core::Filter {
            field: amount.clone(),
            matches: Match::Is(Cell::Number(-0.0)),
        }),
        ..all()
    };
    assert_eq!(workspace.rows(&page, &block, &zero).unwrap().count, 2);
    workspace.close_page(&page);
    let reloaded: Vec<_> = workspace
        .rows(&page, &block, &sorted)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(reloaded, order);
}

#[test]
fn a_database_takes_no_text_style() {
    let (mut workspace, page, block) = expenses();
    assert_eq!(workspace.style(&page, &block).unwrap(), None);
    assert_eq!(
        apply(
            &mut workspace,
            Mutation::SetTextStyle {
                page: page.clone(),
                block: block.clone(),
                style: liquid_core::TextStyle::Quote,
            },
        ),
        Err(Error::NotFound)
    );
}

#[test]
fn every_cell_equals_itself() {
    // Mutations compare as equal values, so a cell must equal itself even holding NaN,
    // which the pipeline refuses to store.
    let nan = Cell::Number(f64::NAN);
    assert_eq!(nan, nan.clone());
    assert_ne!(Cell::Number(1.0), Cell::Text("1".into()));
}

#[test]
fn a_database_holds_a_bounded_number_of_fields_and_options() {
    let (mut workspace, page, block) = expenses();
    // "Name" is the first.
    for number in 1..MAX_FIELDS {
        let add = Mutation::AddField {
            page: page.clone(),
            block: block.clone(),
            place: Place::First,
            name: format!("Field {number}"),
            kind: FieldKind::Select,
        };
        apply(&mut workspace, add).unwrap();
    }
    let one_more = Mutation::AddField {
        page: page.clone(),
        block: block.clone(),
        place: Place::First,
        name: "One more".into(),
        kind: FieldKind::Text,
    };
    assert_eq!(apply(&mut workspace, one_more.clone()), Err(Error::TooMany));
    let fields = workspace.database(&page, &block).unwrap().unwrap().fields;
    assert_eq!(fields.len(), MAX_FIELDS);
    // A removed field frees its place.
    let remove = Mutation::RemoveField {
        page: page.clone(),
        block: block.clone(),
        field: fields[0].id.clone(),
    };
    apply(&mut workspace, remove).unwrap();
    apply(&mut workspace, one_more).unwrap();

    let select = fields[1].id.clone();
    let option = |number: usize| Mutation::AddOption {
        page: page.clone(),
        block: block.clone(),
        field: select.clone(),
        name: format!("Option {number}"),
        color: 0,
    };
    for number in 0..MAX_OPTIONS {
        apply(&mut workspace, option(number)).unwrap();
    }
    assert_eq!(
        apply(&mut workspace, option(MAX_OPTIONS)),
        Err(Error::TooMany)
    );
}

#[test]
fn a_view_names_the_kind_of_its_block() {
    let (mut workspace, page, _) = expenses();
    let table = workspace.views(&page, 0, 10).unwrap()[0].id.clone();
    let add = Mutation::AddTextBlock {
        page: page.clone(),
        after: Some(table),
        text: "October".into(),
    };
    apply(&mut workspace, add).unwrap();
    let kinds = |workspace: &mut Workspace| -> Vec<BlockKind> {
        let views = workspace.views(&page, 0, 10).unwrap();
        views.into_iter().map(|view| view.kind).collect()
    };
    assert_eq!(
        kinds(&mut workspace),
        [BlockKind::Database, BlockKind::Text]
    );
    // Read again from the document, as after a restart.
    workspace.close_page(&page);
    assert_eq!(
        kinds(&mut workspace),
        [BlockKind::Database, BlockKind::Text]
    );
}

#[test]
fn rows_and_fields_added_last_go_after_all_others_when_applied() {
    let (mut workspace, page, block) = expenses();
    let name = workspace.database(&page, &block).unwrap().unwrap().fields[0]
        .id
        .clone();
    let text = |workspace: &mut Workspace, row: &RowId, text: &str| {
        set(
            workspace,
            &page,
            &block,
            row,
            &name,
            Cell::Text(text.into()),
        )
        .unwrap();
    };
    let a = add_row(&mut workspace, &page, &block, Place::Last);
    text(&mut workspace, &a, "A");
    let b = add_row(&mut workspace, &page, &block, Place::Last);
    text(&mut workspace, &b, "B");
    let c = add_row(&mut workspace, &page, &block, Place::After(a.clone()));
    text(&mut workspace, &c, "C");
    let d = add_row(&mut workspace, &page, &block, Place::Last);
    text(&mut workspace, &d, "D");
    let e = add_row(&mut workspace, &page, &block, Place::First);
    text(&mut workspace, &e, "E");
    assert_eq!(
        column(&mut workspace, &page, &block, &all(), &name),
        ["E", "A", "C", "B", "D"].map(|text| Cell::Text(text.into())),
    );

    field(&mut workspace, &page, &block, "Amount", FieldKind::Number);
    let first = Mutation::AddField {
        page: page.clone(),
        block: block.clone(),
        place: Place::First,
        name: "Paid".into(),
        kind: FieldKind::Checkbox,
    };
    apply(&mut workspace, first).unwrap();
    field(&mut workspace, &page, &block, "Date", FieldKind::Date);
    let names: Vec<_> = workspace
        .database(&page, &block)
        .unwrap()
        .unwrap()
        .fields
        .into_iter()
        .map(|field| field.name)
        .collect();
    assert_eq!(names, ["Paid", "Name", "Amount", "Date"]);
}

#[test]
fn actions_stored_before_places_still_read() {
    let (mut workspace, page, block) = expenses();
    let row = add_row(&mut workspace, &page, &block, Place::Last);
    // As an Action recorded before ADR-0247 holds it: `after`, none or a row.
    for (after, place) in [
        (serde_json::Value::Null, Place::First),
        (
            serde_json::json!(row.to_string()),
            Place::After(row.clone()),
        ),
    ] {
        let mut stored = serde_json::to_value(Mutation::AddRow {
            page: page.clone(),
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
                page: page.clone(),
                block: block.clone(),
                place,
            }
        );
    }
    let name = workspace.database(&page, &block).unwrap().unwrap().fields[0]
        .id
        .clone();
    for (after, place) in [
        (serde_json::Value::Null, Place::First),
        (
            serde_json::json!(name.to_string()),
            Place::After(name.clone()),
        ),
    ] {
        let add = |place| Mutation::AddField {
            page: page.clone(),
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
