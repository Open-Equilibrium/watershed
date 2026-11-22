#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, BlockId, Cell, Created, Error, FieldId, FieldKind, Filter, Group, Layout,
    MAX_DEVICE_KEY, MAX_DEVICE_VALUE, MAX_DEVICE_VALUES, MAX_TABS, Match, MemoryStore, Mutation,
    OptionId, Origin, PageId, PersonId, Place, Request, RowId, RowQuery, Sort, Tab, TabId,
    TabSetup, ViewId,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

struct Home {
    workspace: Workspace,
    page: PageId,
}

impl Home {
    fn new() -> Self {
        let mut workspace =
            Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let mut home = |mutation| {
            workspace
                .apply(Request {
                    actor: Actor::Person(PersonId::new("benni")),
                    origin: Origin::Ui,
                    mutation,
                })
                .unwrap()
                .created
        };
        let Some(Created::Area(area)) = home(Mutation::CreateArea {
            name: "Private".into(),
        }) else {
            panic!("an area")
        };
        let Some(Created::Page(page)) = home(Mutation::CreatePage {
            area,
            parent: None,
            title: "Household budget".into(),
        }) else {
            panic!("a Page")
        };
        Self { workspace, page }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        self.workspace.apply(Request {
            actor: Actor::Person(PersonId::new("benni")),
            origin: Origin::Ui,
            mutation,
        })
    }

    fn table(&mut self, name: &str, layout: Layout) -> (BlockId, ViewId) {
        let add = Mutation::AddDatabaseBlock {
            page: self.page.clone(),
            after: None,
            name: name.into(),
            layout,
        };
        match self.edit(add).unwrap().created {
            Some(Created::Block { block, view }) => (block, view),
            other => panic!("a Table, got {other:?}"),
        }
    }

    fn tabs(&mut self, view: &ViewId) -> Vec<Tab> {
        self.workspace
            .view(&self.page.clone(), view)
            .unwrap()
            .unwrap()
            .tabs
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

    fn option(&mut self, table: &BlockId, field: &FieldId, name: &str) -> OptionId {
        let add = Mutation::AddOption {
            block: table.clone(),
            field: field.clone(),
            name: name.into(),
            color: 0,
        };
        match self.edit(add).unwrap().created {
            Some(Created::Option(option)) => option,
            other => panic!("an option, got {other:?}"),
        }
    }

    fn row(&mut self, table: &BlockId, cells: &[(&FieldId, Cell)]) -> RowId {
        let add = Mutation::AddRow {
            block: table.clone(),
            place: Place::Last,
        };
        let Some(Created::Row(row)) = self.edit(add).unwrap().created else {
            panic!("a row")
        };
        for (field, value) in cells {
            self.edit(Mutation::SetCell {
                block: table.clone(),
                row: row.clone(),
                field: (*field).clone(),
                value: value.clone(),
            })
            .unwrap();
        }
        row
    }

    fn add_tab(&mut self, view: &ViewId, after: Option<TabId>, setup: TabSetup) -> TabId {
        let add = Mutation::AddTab {
            page: self.page.clone(),
            view: view.clone(),
            after,
            setup: Box::new(setup),
        };
        match self.edit(add).unwrap().created {
            Some(Created::Tab(tab)) => tab,
            other => panic!("a tab, got {other:?}"),
        }
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

fn names(tabs: &[Tab]) -> Vec<&str> {
    tabs.iter().map(|tab| tab.setup.name.as_str()).collect()
}

#[test]
fn a_new_table_starts_with_one_tab_of_the_chosen_layout() {
    let mut home = Home::new();
    let (expenses, view) = home.table("Expenses", Layout::Table);
    assert_eq!(home.tabs(&view)[0].setup, TabSetup::of(Layout::Table));
    assert_eq!(home.tabs(&view)[0].setup.name, "Table");
    let fields = home.workspace.database(&expenses).unwrap().unwrap().fields;
    assert_eq!(fields.len(), 1);

    let (tasks, view) = home.table("Tasks", Layout::Board);
    let fields = home.workspace.database(&tasks).unwrap().unwrap().fields;
    let status = &fields[1];
    assert_eq!(
        (status.name.as_str(), status.kind),
        ("Status", FieldKind::Select)
    );
    let options: Vec<_> = status.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(options, ["Not started", "In progress", "Done"]);
    let tab = home.tabs(&view).remove(0).setup;
    assert_eq!((tab.name.as_str(), tab.layout), ("Board", Layout::Board));
    assert_eq!(tab.group, Some(status.id.clone()));

    let (events, view) = home.table("Events", Layout::Calendar);
    let fields = home.workspace.database(&events).unwrap().unwrap().fields;
    assert_eq!(
        (fields[1].name.as_str(), fields[1].kind),
        ("Date", FieldKind::Date)
    );
    let tab = home.tabs(&view).remove(0).setup;
    assert_eq!(
        (tab.layout, tab.date),
        (Layout::Calendar, Some(fields[1].id.clone()))
    );
}

#[test]
fn a_view_s_tabs_keep_their_own_settings_and_order() {
    let mut home = Home::new();
    let (table, view) = home.table("Expenses", Layout::Table);
    let category = home.field(&table, "Category", FieldKind::Select);
    let due = home.field(&table, "Due", FieldKind::Date);
    let first = home.tabs(&view)[0].id.clone();
    let board = home.add_tab(&view, Some(first.clone()), TabSetup::of(Layout::Board));
    let page = home.page.clone();
    let tab = |tab: &TabId| (page.clone(), view.clone(), tab.clone());
    let (p, v, t) = tab(&board);
    for change in [
        Mutation::RenameTab {
            page: p.clone(),
            view: v.clone(),
            tab: t.clone(),
            name: "By category".into(),
        },
        Mutation::SetTabGroup {
            page: p.clone(),
            view: v.clone(),
            tab: t.clone(),
            group: Some(category.clone()),
        },
        Mutation::SetTabSort {
            page: p.clone(),
            view: v.clone(),
            tab: t.clone(),
            sort: Some(Sort {
                field: due.clone(),
                descending: true,
            }),
        },
        Mutation::SetTabFilter {
            page: p.clone(),
            view: v.clone(),
            tab: t.clone(),
            filter: Some(Filter {
                field: due.clone(),
                matches: Match::NotEmpty,
            }),
        },
    ] {
        home.edit(change).unwrap();
    }
    let calendar = home.add_tab(&view, None, TabSetup::of(Layout::Table));
    let (p, v, t) = tab(&calendar);
    home.edit(Mutation::SetTabLayout {
        page: p.clone(),
        view: v.clone(),
        tab: t.clone(),
        layout: Layout::Calendar,
    })
    .unwrap();
    home.edit(Mutation::SetTabDate {
        page: p,
        view: v,
        tab: t,
        date: Some(due.clone()),
    })
    .unwrap();

    home.workspace.close_page(&home.page);
    let tabs = home.tabs(&view);
    assert_eq!(names(&tabs), ["Table", "Table", "By category"]);
    assert_eq!(
        (tabs[0].setup.layout, tabs[0].setup.date.as_ref()),
        (Layout::Calendar, Some(&due))
    );
    let by_category = &tabs[2].setup;
    assert_eq!(by_category.group, Some(category));
    assert_eq!(by_category.sort.as_ref().map(|s| s.descending), Some(true));
    assert_eq!(
        by_category.filter.as_ref().map(|f| &f.matches),
        Some(&Match::NotEmpty)
    );
    assert_eq!(tabs[1].setup, TabSetup::of(Layout::Table));

    let (p, v, t) = tab(&board);
    home.edit(Mutation::MoveTab {
        page: p,
        view: v,
        tab: t,
        after: None,
    })
    .unwrap();
    let (p, v, t) = tab(&calendar);
    home.edit(Mutation::RemoveTab {
        page: p,
        view: v,
        tab: t,
    })
    .unwrap();
    home.workspace.close_page(&home.page);
    assert_eq!(names(&home.tabs(&view)), ["By category", "Table"]);
}

#[test]
fn a_tab_groups_by_a_select_or_checkbox_and_dates_by_a_date_field() {
    let mut home = Home::new();
    let (table, view) = home.table("Expenses", Layout::Table);
    let note = home.field(&table, "Note", FieldKind::Text);
    let paid = home.field(&table, "Paid", FieldKind::Checkbox);
    let tab = home.tabs(&view)[0].id.clone();
    let group = |group: &FieldId| Mutation::SetTabGroup {
        page: home.page.clone(),
        view: view.clone(),
        tab: tab.clone(),
        group: Some(group.clone()),
    };
    let (by_note, by_paid) = (group(&note), group(&paid));
    let by_date = Mutation::SetTabDate {
        page: home.page.clone(),
        view: view.clone(),
        tab: tab.clone(),
        date: Some(paid.clone()),
    };
    assert_eq!(home.edit(by_note), Err(Error::WrongKind));
    assert_eq!(home.edit(by_date), Err(Error::WrongKind));
    home.edit(by_paid).unwrap();
    let missing = Mutation::RenameTab {
        page: home.page.clone(),
        view,
        tab: TabId::parse(&"0".repeat(32)).unwrap(),
        name: "Gone".into(),
    };
    assert_eq!(home.edit(missing), Err(Error::NotFound));
}

#[test]
fn a_view_keeps_at_least_one_and_at_most_max_tabs() {
    let mut home = Home::new();
    let (_, view) = home.table("Expenses", Layout::Table);
    let only = home.tabs(&view)[0].id.clone();
    let remove = Mutation::RemoveTab {
        page: home.page.clone(),
        view: view.clone(),
        tab: only,
    };
    assert_eq!(home.edit(remove), Err(Error::NotAllowed));
    for _ in 1..MAX_TABS {
        home.add_tab(&view, None, TabSetup::of(Layout::Board));
    }
    let one_more = Mutation::AddTab {
        page: home.page.clone(),
        view,
        after: None,
        setup: Box::new(TabSetup::of(Layout::Table)),
    };
    assert_eq!(home.edit(one_more), Err(Error::TooMany));
}

#[test]
fn groups_count_rows_per_value_with_the_empty_group_first_when_it_holds_rows() {
    let mut home = Home::new();
    let (table, _) = home.table("Expenses", Layout::Table);
    let category = home.field(&table, "Category", FieldKind::Select);
    let paid = home.field(&table, "Paid", FieldKind::Checkbox);
    let food = home.option(&table, &category, "Food");
    let rent = home.option(&table, &category, "Rent");
    let travel = home.option(&table, &category, "Travel");
    home.row(&table, &[(&category, Cell::Select(rent.clone()))]);
    home.row(&table, &[(&category, Cell::Select(food.clone()))]);
    home.row(
        &table,
        &[
            (&category, Cell::Select(rent.clone())),
            (&paid, Cell::Checkbox(true)),
        ],
    );
    let counts = |groups: Vec<Group>| -> Vec<(Cell, usize)> {
        groups.into_iter().map(|g| (g.value, g.count)).collect()
    };
    let groups = home.workspace.groups(&table, &all(), &category).unwrap();
    assert_eq!(
        counts(groups),
        [
            (Cell::Select(food.clone()), 1),
            (Cell::Select(rent.clone()), 2),
            (Cell::Select(travel), 0),
        ]
    );
    let blank = home.row(&table, &[]);
    let groups = home.workspace.groups(&table, &all(), &category).unwrap();
    assert_eq!(groups[0].value, Cell::Empty);
    assert_eq!(groups[0].count, 1);
    let groups = home.workspace.groups(&table, &all(), &paid).unwrap();
    assert_eq!(
        counts(groups),
        [(Cell::Checkbox(false), 3), (Cell::Checkbox(true), 1)]
    );

    let within = |field: &FieldId, value: Cell| RowQuery {
        within: Some(Filter {
            field: field.clone(),
            matches: Match::Is(value),
        }),
        ..all()
    };
    let empty = home
        .workspace
        .rows(&table, &within(&category, Cell::Empty))
        .unwrap();
    assert_eq!(
        empty.rows.iter().map(|r| &r.id).collect::<Vec<_>>(),
        [&blank]
    );
    let rent_rows = home
        .workspace
        .rows(&table, &within(&category, Cell::Select(rent)))
        .unwrap();
    assert_eq!(rent_rows.count, 2);
    let unpaid = RowQuery {
        filter: Some(Filter {
            field: category.clone(),
            matches: Match::NotEmpty,
        }),
        ..within(&paid, Cell::Checkbox(false))
    };
    assert_eq!(home.workspace.rows(&table, &unpaid).unwrap().count, 2);
    let note = home.field(&table, "Note", FieldKind::Text);
    assert_eq!(
        home.workspace.groups(&table, &all(), &note),
        Err(Error::WrongKind)
    );
}

#[test]
fn a_date_range_keeps_the_rows_dated_within_it() {
    let mut home = Home::new();
    let (table, _) = home.table("Events", Layout::Calendar);
    let date = home.workspace.database(&table).unwrap().unwrap().fields[1]
        .id
        .clone();
    for day in ["2026-10-31", "2026-11-01", "2026-11-30", "2026-12-01"] {
        home.row(&table, &[(&date, Cell::Date(day.into()))]);
    }
    home.row(&table, &[]);
    let november = RowQuery {
        within: Some(Filter {
            field: date.clone(),
            matches: Match::Between("2026-11-01".into(), "2026-11-30".into()),
        }),
        ..all()
    };
    let rows = home.workspace.rows(&table, &november).unwrap().rows;
    let days: Vec<_> = rows.iter().map(|row| row.cell(&date).cloned()).collect();
    assert_eq!(
        days,
        [
            Some(Cell::Date("2026-11-01".into())),
            Some(Cell::Date("2026-11-30".into()))
        ]
    );
}

#[test]
fn duplicating_a_table_copies_its_fields_rows_and_tabs_into_an_independent_table() {
    let mut home = Home::new();
    let (table, view) = home.table("Expenses", Layout::Board);
    let status = home.workspace.database(&table).unwrap().unwrap().fields[1].clone();
    let name = home.workspace.database(&table).unwrap().unwrap().fields[0]
        .id
        .clone();
    let done = status.options[2].id.clone();
    let rent = home.row(
        &table,
        &[
            (&name, Cell::Text("Rent".into())),
            (&status.id, Cell::Select(done.clone())),
        ],
    );
    let gone = home.row(&table, &[(&name, Cell::Text("Gone".into()))]);
    home.edit(Mutation::RemoveRow {
        block: table.clone(),
        row: gone,
    })
    .unwrap();
    let tabs: Vec<TabSetup> = home.tabs(&view).into_iter().map(|tab| tab.setup).collect();
    let duplicate = Mutation::DuplicateTable {
        page: home.page.clone(),
        after: Some(view.clone()),
        table: table.clone(),
        name: "Expenses (copy)".into(),
        tabs: tabs.clone(),
    };
    let Some(Created::Block {
        block: copy,
        view: copied,
    }) = home.edit(duplicate).unwrap().created
    else {
        panic!("a View of a new Table")
    };
    assert_ne!(copy, table);
    let views = home.workspace.views(&home.page, 0, 10).unwrap();
    assert_eq!(
        views.iter().map(|v| &v.id).collect::<Vec<_>>(),
        [&view, &copied]
    );
    assert_eq!(views[1].title, "Expenses (copy)");
    let copied_tabs: Vec<TabSetup> = views[1].tabs.iter().map(|t| t.setup.clone()).collect();
    assert_eq!(copied_tabs, tabs);
    let tables: Vec<_> = home
        .workspace
        .tables()
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(tables, ["Expenses", "Expenses (copy)"]);
    assert_eq!(
        home.workspace.database(&copy).unwrap().unwrap().fields,
        home.workspace.database(&table).unwrap().unwrap().fields
    );
    let rows = home.workspace.rows(&copy, &all()).unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, rent);
    assert_eq!(rows[0].cell(&status.id), Some(&Cell::Select(done)));

    home.edit(Mutation::SetCell {
        block: copy.clone(),
        row: rent.clone(),
        field: name.clone(),
        value: Cell::Text("Lease".into()),
    })
    .unwrap();
    let original = home.workspace.rows(&table, &all()).unwrap().rows;
    assert_eq!(original[0].cell(&name), Some(&Cell::Text("Rent".into())));
}

#[test]
fn a_view_of_an_existing_table_starts_with_one_table_tab_or_copies_tabs() {
    let mut home = Home::new();
    let (table, view) = home.table("Expenses", Layout::Board);
    let add = |tabs: Vec<TabSetup>| Mutation::AddTableView {
        page: home.page.clone(),
        after: None,
        table: table.clone(),
        title: "Expenses".into(),
        tabs,
    };
    let (plain, copied) = (add(Vec::new()), add(vec![TabSetup::of(Layout::Calendar)]));
    let Some(Created::Block { view: plain, .. }) = home.edit(plain).unwrap().created else {
        panic!("a View")
    };
    assert_eq!(names(&home.tabs(&plain)), ["Table"]);
    let Some(Created::Block { view: copied, .. }) = home.edit(copied).unwrap().created else {
        panic!("a View")
    };
    assert_eq!(home.tabs(&copied)[0].setup.layout, Layout::Calendar);
    assert_eq!(home.tabs(&view)[0].setup.layout, Layout::Board);
}

#[test]
fn actions_recorded_before_tabs_still_read() {
    for stored in [
        r#"{"SetViewSort":{"page":"p","view":"v","sort":null}}"#,
        r#"{"SetViewFilter":{"page":"p","view":"v","filter":null}}"#,
        r#"{"AddDatabaseBlock":{"page":"p","after":null,"name":"Expenses"}}"#,
        r#"{"AddTableView":{"page":"p","after":null,"table":"t","title":"T","sort":null,"filter":null}}"#,
    ] {
        let read: Result<Mutation, _> = serde_json::from_str(stored);
        assert!(read.is_ok(), "{stored}");
    }
}

#[test]
fn a_tab_folds_groups_for_everyone_by_their_value() {
    let mut home = Home::new();
    let (table, view) = home.table("Tasks", Layout::Board);
    let status = home.workspace.database(&table).unwrap().unwrap().fields[1].clone();
    let tab = home.tabs(&view)[0].id.clone();
    let fold = |folded: Vec<Cell>| Mutation::SetTabFolded {
        page: home.page.clone(),
        view: view.clone(),
        tab: tab.clone(),
        folded,
    };
    let done = Cell::Select(status.options[2].id.clone());
    let (text, kept) = (
        fold(vec![Cell::Text("Done".into())]),
        fold(vec![Cell::Empty, done.clone()]),
    );
    assert_eq!(home.edit(text), Err(Error::WrongKind));
    home.edit(kept).unwrap();
    home.workspace.close_page(&home.page);
    assert_eq!(home.tabs(&view)[0].setup.folded, [Cell::Empty, done]);
}

#[test]
fn a_device_value_stays_out_of_history_and_the_oldest_makes_room() {
    let mut home = Home::new();
    let before = home.workspace.history(None, 100).unwrap().len();
    home.workspace.remember("tab/v", Some("t")).unwrap();
    assert_eq!(
        home.workspace.remembered("tab/v").unwrap().as_deref(),
        Some("t")
    );
    assert_eq!(home.workspace.history(None, 100).unwrap().len(), before);
    home.workspace.remember("tab/v", None).unwrap();
    assert_eq!(home.workspace.remembered("tab/v").unwrap(), None);
    let long_key = "k".repeat(MAX_DEVICE_KEY + 1);
    let long_value = "v".repeat(MAX_DEVICE_VALUE + 1);
    assert_eq!(
        home.workspace.remember(&long_key, Some("t")),
        Err(Error::TooLong)
    );
    assert_eq!(
        home.workspace.remember("k", Some(&long_value)),
        Err(Error::TooLong)
    );
    for at in 0..MAX_DEVICE_VALUES {
        home.workspace.remember(&at.to_string(), Some("t")).unwrap();
    }
    home.workspace.remember("0", Some("u")).unwrap();
    home.workspace.remember("one more", Some("t")).unwrap();
    assert_eq!(
        home.workspace.remembered("1").unwrap(),
        None,
        "the oldest goes"
    );
    assert_eq!(
        home.workspace.remembered("0").unwrap().as_deref(),
        Some("u")
    );
    assert_eq!(
        home.workspace.remembered("one more").unwrap().as_deref(),
        Some("t")
    );
}

#[test]
fn a_copied_tab_drops_what_refers_to_a_removed_field() {
    let mut home = Home::new();
    let (table, view) = home.table("Tasks", Layout::Board);
    let due = home.field(&table, "Due", FieldKind::Date);
    let status = home.tabs(&view)[0].setup.group.clone().unwrap();
    let tab = home.tabs(&view)[0].id.clone();
    home.edit(Mutation::SetTabSort {
        page: home.page.clone(),
        view: view.clone(),
        tab,
        sort: Some(Sort {
            field: due.clone(),
            descending: false,
        }),
    })
    .unwrap();
    for field in [due, status] {
        home.edit(Mutation::RemoveField {
            block: table.clone(),
            field,
        })
        .unwrap();
    }
    let setup = home.tabs(&view)[0].setup.clone();
    let copied = home.add_tab(&view, None, setup.clone());
    let kept = home
        .tabs(&view)
        .into_iter()
        .find(|tab| tab.id == copied)
        .unwrap();
    assert_eq!(
        kept.setup,
        TabSetup {
            group: None,
            sort: None,
            ..setup.clone()
        }
    );
    let duplicate = Mutation::DuplicateTable {
        page: home.page.clone(),
        after: Some(view),
        table,
        name: "Tasks (copy)".into(),
        tabs: vec![setup],
    };
    home.edit(duplicate).unwrap();
}
