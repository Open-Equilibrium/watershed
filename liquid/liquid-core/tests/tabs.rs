#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Applied, BlockId, Cell, Created, Error, Field, FieldId, FieldKind, Filter, Group, Layout,
    MAX_DEVICE_KEY, MAX_DEVICE_VALUE, MAX_DEVICE_VALUES, MAX_TABS, Match, Mutation, OptionId,
    PageId, Place, RowId, RowQuery, Rows, Sort, Tab, TabId, TabSetup, ViewId,
};

struct Home {
    workspace: Workspace,
    page: PageId,
}

impl Home {
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
        Self { workspace, page }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn table(&mut self, name: &str, layout: Layout) -> (BlockId, ViewId) {
        let add = Mutation::AddDatabaseBlock {
            page: self.page.clone(),
            after: None,
            name: name.into(),
            layout,
        };
        let Created::Block { block, view } = created(&mut self.workspace, add) else {
            panic!("a Table")
        };
        (block, view)
    }

    fn tabs(&mut self, view: &ViewId) -> Vec<Tab> {
        self.workspace.view(&self.page, view).unwrap().unwrap().tabs
    }

    fn fields(&mut self, table: &BlockId) -> Vec<Field> {
        self.workspace.database(table).unwrap().unwrap().fields
    }

    fn rows(&mut self, table: &BlockId, query: &RowQuery) -> Rows {
        self.workspace.rows(table, query).unwrap()
    }

    fn groups(&mut self, table: &BlockId, query: &RowQuery, by: &FieldId) -> Vec<Group> {
        self.workspace.groups(table, query, by).unwrap()
    }

    fn counts(&mut self, table: &BlockId, by: &FieldId) -> Vec<(Cell, usize)> {
        self.groups(table, &all(), by)
            .into_iter()
            .map(|group| (group.value, group.count))
            .collect()
    }

    fn field(&mut self, table: &BlockId, name: &str, kind: FieldKind) -> FieldId {
        let add = Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: name.into(),
            kind,
        };
        let Created::Field(field) = created(&mut self.workspace, add) else {
            panic!("a field")
        };
        field
    }

    fn option(&mut self, table: &BlockId, field: &FieldId, name: &str) -> OptionId {
        let add = Mutation::AddOption {
            block: table.clone(),
            field: field.clone(),
            name: name.into(),
            color: 0,
        };
        let Created::Option(option) = created(&mut self.workspace, add) else {
            panic!("an option")
        };
        option
    }

    fn row(&mut self, table: &BlockId, cells: &[(&FieldId, Cell)]) -> RowId {
        let add = Mutation::AddRow {
            block: table.clone(),
            place: Place::Last,
        };
        let Created::Row(row) = created(&mut self.workspace, add) else {
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
        let Created::Tab(tab) = created(&mut self.workspace, add) else {
            panic!("a tab")
        };
        tab
    }

    fn tab_edit(
        &mut self,
        view: &ViewId,
        tab: &TabId,
        change: impl FnOnce(PageId, ViewId, TabId) -> Mutation,
    ) {
        let mutation = change(self.page.clone(), view.clone(), tab.clone());
        self.edit(mutation).unwrap();
    }
}

fn names(tabs: &[Tab]) -> Vec<&str> {
    tabs.iter().map(|tab| tab.setup.name.as_str()).collect()
}

fn within(field: &FieldId, value: Cell) -> RowQuery {
    RowQuery {
        within: Some(Filter {
            field: field.clone(),
            matches: Match::Is(value),
        }),
        ..all()
    }
}

#[test]
fn a_new_table_starts_with_one_tab_of_the_chosen_layout() {
    let mut home = Home::new();
    let (expenses, view) = home.table("Expenses", Layout::Table);
    assert_eq!(home.tabs(&view)[0].setup, TabSetup::of(Layout::Table));
    assert_eq!(home.tabs(&view)[0].setup.name, "Table");
    assert_eq!(home.fields(&expenses).len(), 1);

    let (tasks, view) = home.table("Tasks", Layout::Board);
    let fields = home.fields(&tasks);
    let status = &fields[1];
    assert_eq!(
        (status.name.as_str(), status.kind),
        ("Status", FieldKind::Select)
    );
    let options: Vec<_> = status.options.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(options, ["Not started", "In progress", "Done"]);
    let tab = home.tabs(&view).remove(0).setup;
    let group = Some(status.id.clone());
    assert_eq!(
        (tab.name.as_str(), tab.layout, tab.group),
        ("Board", Layout::Board, group)
    );

    let (events, view) = home.table("Events", Layout::Calendar);
    let fields = home.fields(&events);
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
    let board = home.add_tab(&view, Some(first), TabSetup::of(Layout::Board));
    home.tab_edit(&view, &board, |page, view, tab| Mutation::RenameTab {
        page,
        view,
        tab,
        name: "By category".into(),
    });
    home.tab_edit(&view, &board, |page, view, tab| Mutation::SetTabGroup {
        page,
        view,
        tab,
        group: Some(category.clone()),
    });
    home.tab_edit(&view, &board, |page, view, tab| Mutation::SetTabSort {
        page,
        view,
        tab,
        sort: Some(Sort {
            field: due.clone(),
            descending: true,
        }),
    });
    home.tab_edit(&view, &board, |page, view, tab| Mutation::SetTabFilter {
        page,
        view,
        tab,
        filter: Some(Filter {
            field: due.clone(),
            matches: Match::NotEmpty,
        }),
    });
    let calendar = home.add_tab(&view, None, TabSetup::of(Layout::Table));
    home.tab_edit(&view, &calendar, |page, view, tab| Mutation::SetTabLayout {
        page,
        view,
        tab,
        layout: Layout::Calendar,
    });
    home.tab_edit(&view, &calendar, |page, view, tab| Mutation::SetTabDate {
        page,
        view,
        tab,
        date: Some(due.clone()),
    });

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

    home.tab_edit(&view, &board, |page, view, tab| Mutation::MoveTab {
        page,
        view,
        tab,
        after: None,
    });
    home.tab_edit(&view, &calendar, |page, view, tab| Mutation::RemoveTab {
        page,
        view,
        tab,
    });
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
    let page = home.page.clone();
    let group = |group: &FieldId| Mutation::SetTabGroup {
        page: page.clone(),
        view: view.clone(),
        tab: tab.clone(),
        group: Some(group.clone()),
    };
    let by_date = Mutation::SetTabDate {
        page: page.clone(),
        view: view.clone(),
        tab: tab.clone(),
        date: Some(paid.clone()),
    };
    assert_eq!(home.edit(group(&note)), Err(Error::WrongKind));
    assert_eq!(home.edit(by_date), Err(Error::WrongKind));
    home.edit(group(&paid)).unwrap();
    let missing = Mutation::RenameTab {
        page,
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
    assert_eq!(
        home.counts(&table, &category),
        [
            (Cell::Select(food), 1),
            (Cell::Select(rent.clone()), 2),
            (Cell::Select(travel), 0),
        ]
    );
    let blank = home.row(&table, &[]);
    let groups = home.groups(&table, &all(), &category);
    assert_eq!((groups[0].value.clone(), groups[0].count), (Cell::Empty, 1));
    assert_eq!(
        home.counts(&table, &paid),
        [(Cell::Checkbox(false), 3), (Cell::Checkbox(true), 1)]
    );

    let empty = home.rows(&table, &within(&category, Cell::Empty));
    assert_eq!(
        empty.rows.iter().map(|r| &r.id).collect::<Vec<_>>(),
        [&blank]
    );
    let rent_rows = home.rows(&table, &within(&category, Cell::Select(rent)));
    assert_eq!(rent_rows.count, 2);
    let unpaid = RowQuery {
        filter: Some(Filter {
            field: category.clone(),
            matches: Match::NotEmpty,
        }),
        ..within(&paid, Cell::Checkbox(false))
    };
    assert_eq!(home.rows(&table, &unpaid).count, 2);
    let note = home.field(&table, "Note", FieldKind::Text);
    assert_eq!(
        home.workspace.groups(&table, &all(), &note),
        Err(Error::WrongKind)
    );

    let first = RowQuery {
        limit: 1,
        sort: Some(Sort {
            field: paid.clone(),
            descending: true,
        }),
        ..all()
    };
    let groups = home.groups(&table, &first, &category);
    assert_eq!(
        (groups[0].count, &groups[0].rows[0].id),
        (1, &blank),
        "the empty group"
    );
    assert_eq!(
        (groups[2].count, groups[2].rows.len()),
        (2, 1),
        "a group's first rows, up to the limit"
    );
    assert_eq!(
        groups[2].rows[0].cell(&paid),
        Some(&Cell::Checkbox(true)),
        "in the query's sort"
    );
    let second = RowQuery { start: 1, ..first };
    let groups = home.groups(&table, &second, &category);
    assert_eq!(groups[2].count, 2, "the count stays whole");
    assert_eq!(
        groups[2].rows[0].cell(&paid),
        None,
        "the window starts inside each group"
    );
    let counted = home.groups(&table, &RowQuery { limit: 0, ..all() }, &category);
    assert!(counted.iter().all(|group| group.rows.is_empty()));
}

#[test]
fn a_date_range_keeps_the_rows_dated_within_it() {
    let mut home = Home::new();
    let (table, _) = home.table("Events", Layout::Calendar);
    let date = home.fields(&table)[1].id.clone();
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
    let rows = home.rows(&table, &november).rows;
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
    let fields = home.fields(&table);
    let (name, status) = (fields[0].id.clone(), fields[1].clone());
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
    let Created::Block {
        block: copy,
        view: copied,
    } = created(&mut home.workspace, duplicate)
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
    assert_eq!(home.fields(&copy), home.fields(&table));
    let rows = home.rows(&copy, &all()).rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, rent);
    assert_eq!(rows[0].cell(&status.id), Some(&Cell::Select(done)));

    home.edit(Mutation::SetCell {
        block: copy,
        row: rent,
        field: name.clone(),
        value: Cell::Text("Lease".into()),
    })
    .unwrap();
    let original = home.rows(&table, &all()).rows;
    assert_eq!(original[0].cell(&name), Some(&Cell::Text("Rent".into())));
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
    let status = home.fields(&table)[1].clone();
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
    let mut workspace = workspace();
    let before = workspace.history(None, 100).unwrap().len();
    workspace.remember("tab/v", Some("t")).unwrap();
    assert_eq!(workspace.remembered("tab/v").unwrap().as_deref(), Some("t"));
    assert_eq!(workspace.history(None, 100).unwrap().len(), before);
    workspace.remember("tab/v", None).unwrap();
    assert_eq!(workspace.remembered("tab/v").unwrap(), None);
    let long_key = "k".repeat(MAX_DEVICE_KEY + 1);
    let long_value = "v".repeat(MAX_DEVICE_VALUE + 1);
    assert_eq!(
        workspace.remember(&long_key, Some("t")),
        Err(Error::TooLong)
    );
    assert_eq!(
        workspace.remember("k", Some(&long_value)),
        Err(Error::TooLong)
    );
    for at in 0..MAX_DEVICE_VALUES {
        workspace.remember(&at.to_string(), Some("t")).unwrap();
    }
    workspace.remember("0", Some("u")).unwrap();
    workspace.remember("one more", Some("t")).unwrap();
    assert_eq!(workspace.remembered("1").unwrap(), None, "the oldest goes");
    assert_eq!(workspace.remembered("0").unwrap().as_deref(), Some("u"));
    assert_eq!(
        workspace.remembered("one more").unwrap().as_deref(),
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
    home.tab_edit(&view, &tab, |page, view, tab| Mutation::SetTabSort {
        page,
        view,
        tab,
        sort: Some(Sort {
            field: due.clone(),
            descending: false,
        }),
    });
    let setup = home.tabs(&view)[0].setup.clone();
    assert!(setup.group.is_some() && setup.sort.is_some());
    for field in [due, status] {
        home.edit(Mutation::RemoveField {
            block: table.clone(),
            field,
        })
        .unwrap();
    }
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
    let Created::Block { view: copy, .. } = created(&mut home.workspace, duplicate) else {
        panic!("a View of the copy")
    };
    assert_eq!(home.tabs(&copy)[0].setup, kept.setup);
}
