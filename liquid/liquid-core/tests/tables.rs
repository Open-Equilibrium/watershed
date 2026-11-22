#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, AreaId, BlockId, Cell, Changed, Created, Error, FieldKind, Filter, Layout, Match,
    MemoryStore, Mutation, Origin, PageId, PersonId, Place, Request, RowQuery, Sort, TabSetup,
    View,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn apply(workspace: &mut Workspace, mutation: Mutation) -> Result<liquid_core::Applied, Error> {
    workspace.apply(Request {
        actor: Actor::Person(PersonId::new("benni")),
        origin: Origin::Ui,
        mutation,
    })
}

struct Home {
    workspace: Workspace,
    area: AreaId,
    budget: PageId,
    review: PageId,
    table: BlockId,
}

impl Home {
    fn new() -> Self {
        let mut workspace =
            Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let area = match apply(
            &mut workspace,
            Mutation::CreateArea {
                name: "Private".into(),
            },
        )
        .unwrap()
        .created
        {
            Some(Created::Area(area)) => area,
            other => panic!("an area, got {other:?}"),
        };
        let mut page = |title: &str| match apply(
            &mut workspace,
            Mutation::CreatePage {
                area: area.clone(),
                parent: None,
                title: title.into(),
            },
        )
        .unwrap()
        .created
        {
            Some(Created::Page(page)) => page,
            other => panic!("a Page, got {other:?}"),
        };
        let (budget, review) = (page("Household budget"), page("Monthly review"));
        let add = Mutation::AddDatabaseBlock {
            page: budget.clone(),
            after: None,
            name: "Expenses".into(),
            layout: liquid_core::Layout::Table,
        };
        let table = match apply(&mut workspace, add).unwrap().created {
            Some(Created::Block { block, .. }) => block,
            other => panic!("a Table, got {other:?}"),
        };
        Self {
            workspace,
            area,
            budget,
            review,
            table,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<liquid_core::Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn views(&mut self, page: &PageId) -> Vec<View> {
        self.workspace.views(page, 0, 10).unwrap()
    }

    fn show(&mut self, page: &PageId, title: &str) -> liquid_core::ViewId {
        let add = Mutation::AddTableView {
            page: page.clone(),
            after: None,
            table: self.table.clone(),
            title: title.into(),
            tabs: Vec::new(),
        };
        match self.edit(add).unwrap().created {
            Some(Created::Block { view, .. }) => view,
            other => panic!("a View, got {other:?}"),
        }
    }

    fn names(&mut self) -> Vec<String> {
        self.workspace
            .tables()
            .unwrap()
            .into_iter()
            .map(|table| table.name)
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
fn a_table_is_listed_in_tables_and_read_without_its_page() {
    let mut home = Home::new();
    let tables = home.workspace.tables().unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(
        (&tables[0].id, &tables[0].area, tables[0].name.as_str()),
        (&home.table, &home.area, "Expenses")
    );
    let view = home.views(&home.budget.clone()).remove(0);
    assert_eq!(
        (&view.block, view.title.as_str()),
        (&home.table, "Expenses")
    );
    home.workspace.close_page(&home.budget);
    let database = home.workspace.database(&home.table).unwrap().unwrap();
    assert_eq!(database.name, "Expenses");
    assert!(
        !home.workspace.is_loaded(&home.budget),
        "reading a Table loads no Page"
    );
}

#[test]
fn views_on_two_pages_show_and_change_the_same_rows() {
    let mut home = Home::new();
    home.show(&home.review.clone(), "All costs");
    let name = home
        .workspace
        .database(&home.table)
        .unwrap()
        .unwrap()
        .fields[0]
        .id
        .clone();
    let row = match home
        .edit(Mutation::AddRow {
            block: home.table.clone(),
            place: Place::Last,
        })
        .unwrap()
        .created
    {
        Some(Created::Row(row)) => row,
        other => panic!("a row, got {other:?}"),
    };
    let set = home
        .edit(Mutation::SetCell {
            block: home.table.clone(),
            row,
            field: name.clone(),
            value: Cell::Text("Rent".into()),
        })
        .unwrap();
    assert_eq!(set.changed, [Changed::Table(home.table.clone())]);
    let rows = home.workspace.rows(&home.table, &all()).unwrap().rows;
    assert_eq!(rows[0].cell(&name), Some(&Cell::Text("Rent".into())));
    let shown: Vec<_> = [home.budget.clone(), home.review.clone()]
        .iter()
        .map(|page| home.views(page).remove(0).block)
        .collect();
    assert_eq!(shown, [home.table.clone(), home.table.clone()]);
    let mut where_shown = home.workspace.table_views(&home.table).unwrap();
    where_shown.sort();
    let mut expected = vec![home.budget.clone(), home.review.clone()];
    expected.sort();
    assert_eq!(
        where_shown
            .into_iter()
            .map(|(page, _)| page)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn a_view_s_title_is_its_own_and_the_first_one_names_an_unnamed_table() {
    let mut home = Home::new();
    let all_costs = home.show(&home.review.clone(), "All costs");
    home.edit(Mutation::RenameView {
        page: home.review.clone(),
        view: all_costs,
        title: "Costs".into(),
    })
    .unwrap();
    assert_eq!(home.views(&home.review.clone())[0].title, "Costs");
    assert_eq!(home.views(&home.budget.clone())[0].title, "Expenses");
    assert_eq!(home.names(), ["Expenses"]);
    home.edit(Mutation::RenameDatabase {
        block: home.table.clone(),
        name: "Spending".into(),
    })
    .unwrap();
    assert_eq!(home.names(), ["Spending"]);
    assert_eq!(home.views(&home.budget.clone())[0].title, "Expenses");

    let add = Mutation::AddDatabaseBlock {
        page: home.budget.clone(),
        after: None,
        name: String::new(),
        layout: liquid_core::Layout::Table,
    };
    let Some(Created::Block { view, .. }) = home.edit(add).unwrap().created else {
        panic!("a Table")
    };
    home.edit(Mutation::RenameView {
        page: home.budget.clone(),
        view: view.clone(),
        title: "Reading list".into(),
    })
    .unwrap();
    home.edit(Mutation::RenameView {
        page: home.budget.clone(),
        view,
        title: "Books".into(),
    })
    .unwrap();
    let mut names = home.names();
    names.sort();
    assert_eq!(names, ["Reading list", "Spending"]);
}

#[test]
fn a_table_outlives_its_last_view_until_it_is_deleted_with_all_its_views() {
    let mut home = Home::new();
    let budget_view = home.views(&home.budget.clone()).remove(0).id;
    home.edit(Mutation::RemoveView {
        page: home.budget.clone(),
        view: budget_view,
    })
    .unwrap();
    assert!(home.views(&home.budget.clone()).is_empty());
    assert_eq!(home.names(), ["Expenses"]);
    assert!(home.workspace.table_views(&home.table).unwrap().is_empty());

    home.show(&home.budget.clone(), "October");
    home.show(&home.review.clone(), "All costs");
    let deleted = home
        .edit(Mutation::DeleteTable {
            block: home.table.clone(),
        })
        .unwrap();
    assert!(deleted.changed.contains(&Changed::Tables));
    assert!(home.names().is_empty());
    assert!(home.views(&home.budget.clone()).is_empty());
    home.workspace.close_page(&home.review);
    assert!(home.views(&home.review.clone()).is_empty());
    assert_eq!(home.workspace.database(&home.table).unwrap(), None);
}

#[test]
fn a_table_made_in_tables_belongs_to_its_area_and_a_new_view_may_copy_a_setup() {
    let mut home = Home::new();
    let made = home
        .edit(Mutation::CreateTable {
            area: home.area.clone(),
            name: "Customers".into(),
        })
        .unwrap();
    let Some(Created::Table(customers)) = made.created else {
        panic!("a Table")
    };
    assert_eq!(made.changed, [Changed::Tables]);
    assert!(home.workspace.table_views(&customers).unwrap().is_empty());

    let amount = match home
        .edit(Mutation::AddField {
            block: home.table.clone(),
            place: Place::Last,
            name: "Amount".into(),
            kind: FieldKind::Number,
        })
        .unwrap()
        .created
    {
        Some(Created::Field(field)) => field,
        other => panic!("a field, got {other:?}"),
    };
    let sort = Sort {
        field: amount.clone(),
        descending: true,
    };
    let filter = Filter {
        field: amount,
        matches: Match::Is(Cell::Number(5.0)),
    };
    let setup = TabSetup {
        sort: Some(sort),
        filter: Some(filter),
        ..TabSetup::of(Layout::Table)
    };
    let add = Mutation::AddTableView {
        page: home.review.clone(),
        after: None,
        table: home.table.clone(),
        title: "Big costs".into(),
        tabs: vec![setup.clone()],
    };
    home.edit(add).unwrap();
    let view = home.views(&home.review.clone()).remove(0);
    assert_eq!(view.tabs[0].setup, setup);

    let foreign = Mutation::AddTableView {
        page: home.review.clone(),
        after: None,
        table: customers,
        title: "Customers".into(),
        tabs: vec![setup],
    };
    assert_eq!(home.edit(foreign), Err(Error::NotFound));
}

#[test]
fn a_search_keeps_rows_whose_text_or_option_holds_it() {
    let mut home = Home::new();
    let table = home.table.clone();
    let name = home.workspace.database(&table).unwrap().unwrap().fields[0]
        .id
        .clone();
    let Some(Created::Field(category)) = home
        .edit(Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: "Category".into(),
            kind: FieldKind::Select,
        })
        .unwrap()
        .created
    else {
        panic!("a field")
    };
    let Some(Created::Option(home_option)) = home
        .edit(Mutation::AddOption {
            block: table.clone(),
            field: category.clone(),
            name: "Home".into(),
            color: 0,
        })
        .unwrap()
        .created
    else {
        panic!("an option")
    };
    for (item, option) in [
        ("Rent", Some(&home_option)),
        ("Groceries", None),
        ("Train", None),
    ] {
        let Some(Created::Row(row)) = home
            .edit(Mutation::AddRow {
                block: table.clone(),
                place: Place::Last,
            })
            .unwrap()
            .created
        else {
            panic!("a row")
        };
        home.edit(Mutation::SetCell {
            block: table.clone(),
            row: row.clone(),
            field: name.clone(),
            value: Cell::Text(item.into()),
        })
        .unwrap();
        if let Some(option) = option {
            home.edit(Mutation::SetCell {
                block: table.clone(),
                row,
                field: category.clone(),
                value: Cell::Select(option.clone()),
            })
            .unwrap();
        }
    }
    let found = |home: &mut Home, search: &str| -> Vec<Cell> {
        let query = RowQuery {
            search: Some(search.into()),
            ..all()
        };
        home.workspace
            .rows(&table, &query)
            .unwrap()
            .rows
            .into_iter()
            .map(|row| row.cell(&name).cloned().unwrap_or(Cell::Empty))
            .collect()
    };
    assert_eq!(
        found(&mut home, "O"),
        [Cell::Text("Rent".into()), Cell::Text("Groceries".into())]
    );
    assert_eq!(found(&mut home, "home"), [Cell::Text("Rent".into())]);
    assert_eq!(found(&mut home, "plane"), Vec::<Cell>::new());
}
