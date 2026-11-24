#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Applied, AreaId, BlockId, Cell, Changed, Created, Error, FieldKind, Filter, Layout, Match,
    Mutation, PageId, RowId, RowQuery, Sort, TabSetup, View, ViewId,
};

struct Home {
    workspace: Workspace,
    area: AreaId,
    budget: PageId,
    review: PageId,
    table: BlockId,
}

impl Home {
    fn new() -> Self {
        let (mut workspace, area, budget) = private_page("Household budget");
        let review = page(&mut workspace, &area, "Monthly review");
        let (table, _) = table(&mut workspace, &budget, "Expenses");
        Self {
            workspace,
            area,
            budget,
            review,
            table,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn create(&mut self, mutation: Mutation) -> Created {
        created(&mut self.workspace, mutation)
    }

    fn field(&mut self, name: &str, kind: FieldKind) -> liquid_core::FieldId {
        field(&mut self.workspace, &self.table, name, kind)
    }

    fn row(&mut self) -> RowId {
        row(&mut self.workspace, &self.table)
    }

    fn set_cell(&mut self, row: &RowId, field: &liquid_core::FieldId, value: Cell) -> Applied {
        self.edit(Mutation::SetCell {
            block: self.table.clone(),
            row: row.clone(),
            field: field.clone(),
            value,
        })
        .unwrap()
    }

    fn name_field(&mut self) -> liquid_core::FieldId {
        self.workspace
            .database(&self.table)
            .unwrap()
            .unwrap()
            .fields[0]
            .id
            .clone()
    }

    fn views(&mut self, page: &PageId) -> Vec<View> {
        self.workspace.views(page, 0, 10).unwrap()
    }

    fn first_view(&mut self, page: &PageId) -> View {
        self.views(page).remove(0)
    }

    fn show(&mut self, page: &PageId, title: &str) -> ViewId {
        let Created::Block { view, .. } = self.create(Mutation::AddTableView {
            page: page.clone(),
            after: None,
            table: self.table.clone(),
            title: title.into(),
            tabs: Vec::new(),
        }) else {
            panic!("a View")
        };
        view
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

#[test]
fn a_table_is_listed_in_tables_and_read_without_its_page() {
    let mut home = Home::new();
    let tables = home.workspace.tables().unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(
        (&tables[0].id, &tables[0].area, tables[0].name.as_str()),
        (&home.table, &home.area, "Expenses")
    );
    let budget = home.budget.clone();
    let view = home.first_view(&budget);
    assert_eq!(
        (&view.block, view.title.as_str()),
        (&home.table, "Expenses")
    );
    home.workspace.close_page(&budget);
    let database = home.workspace.database(&home.table).unwrap().unwrap();
    assert_eq!(database.name, "Expenses");
    assert!(
        !home.workspace.is_loaded(&budget),
        "reading a Table loads no Page"
    );
}

#[test]
fn views_on_two_pages_show_and_change_the_same_rows() {
    let mut home = Home::new();
    let (budget, review) = (home.budget.clone(), home.review.clone());
    home.show(&review, "All costs");
    let name = home.name_field();
    let row = home.row();
    let set = home.set_cell(&row, &name, Cell::Text("Rent".into()));
    assert_eq!(set.changed, [Changed::Table(home.table.clone())]);
    let rows = home.workspace.rows(&home.table, &all()).unwrap().rows;
    assert_eq!(rows[0].cell(&name), Some(&Cell::Text("Rent".into())));
    let shown: Vec<_> = [budget.clone(), review.clone()]
        .iter()
        .map(|page| home.first_view(page).block)
        .collect();
    assert_eq!(shown, [home.table.clone(), home.table.clone()]);
    let mut where_shown = home.workspace.table_views(&home.table).unwrap();
    where_shown.sort();
    let mut expected = vec![budget, review];
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
    let (budget, review) = (home.budget.clone(), home.review.clone());
    let all_costs = home.show(&review, "All costs");
    home.edit(Mutation::RenameView {
        page: review.clone(),
        view: all_costs,
        title: "Costs".into(),
    })
    .unwrap();
    assert_eq!(home.first_view(&review).title, "Costs");
    assert_eq!(home.first_view(&budget).title, "Expenses");
    assert_eq!(home.names(), ["Expenses"]);
    home.edit(Mutation::RenameDatabase {
        block: home.table.clone(),
        name: "Spending".into(),
    })
    .unwrap();
    assert_eq!(home.names(), ["Spending"]);
    assert_eq!(home.first_view(&budget).title, "Expenses");

    let Created::Block { view, .. } = home.create(Mutation::AddDatabaseBlock {
        page: budget.clone(),
        after: None,
        name: String::new(),
        layout: Layout::Table,
    }) else {
        panic!("a Table")
    };
    for title in ["Reading list", "Books"] {
        home.edit(Mutation::RenameView {
            page: budget.clone(),
            view: view.clone(),
            title: title.into(),
        })
        .unwrap();
    }
    let mut names = home.names();
    names.sort();
    assert_eq!(names, ["Reading list", "Spending"]);
}

#[test]
fn a_table_outlives_its_last_view_until_it_is_deleted_with_all_its_views() {
    let mut home = Home::new();
    let (budget, review) = (home.budget.clone(), home.review.clone());
    let budget_view = home.first_view(&budget).id;
    home.edit(Mutation::RemoveView {
        page: budget.clone(),
        view: budget_view,
    })
    .unwrap();
    assert!(home.views(&budget).is_empty());
    assert_eq!(home.names(), ["Expenses"]);
    assert!(home.workspace.table_views(&home.table).unwrap().is_empty());

    home.show(&budget, "October");
    home.show(&review, "All costs");
    let deleted = home
        .edit(Mutation::DeleteTable {
            block: home.table.clone(),
        })
        .unwrap();
    assert!(deleted.changed.contains(&Changed::Tables));
    assert!(home.names().is_empty());
    assert!(home.views(&budget).is_empty());
    home.workspace.close_page(&review);
    assert!(home.views(&review).is_empty());
    assert_eq!(home.workspace.database(&home.table).unwrap(), None);
}

#[test]
fn a_table_made_in_tables_belongs_to_its_area_and_a_new_view_may_copy_a_setup() {
    let mut home = Home::new();
    let review = home.review.clone();
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

    let amount = home.field("Amount", FieldKind::Number);
    let setup = TabSetup {
        sort: Some(Sort {
            field: amount.clone(),
            descending: true,
        }),
        filter: Some(Filter {
            field: amount,
            matches: Match::Is(Cell::Number(5.0)),
        }),
        ..TabSetup::of(Layout::Table)
    };
    home.edit(Mutation::AddTableView {
        page: review.clone(),
        after: None,
        table: home.table.clone(),
        title: "Big costs".into(),
        tabs: vec![setup.clone()],
    })
    .unwrap();
    assert_eq!(home.first_view(&review).tabs[0].setup, setup);

    home.edit(Mutation::AddTableView {
        page: review.clone(),
        after: None,
        table: customers,
        title: "Customers".into(),
        tabs: vec![setup],
    })
    .unwrap();
    assert_eq!(
        home.first_view(&review).tabs[0].setup,
        TabSetup::of(Layout::Table),
        "another Table's fields are not this one's"
    );
}

#[test]
fn a_search_keeps_rows_whose_text_or_option_holds_it() {
    let mut home = Home::new();
    let table = home.table.clone();
    let name = home.name_field();
    let category = home.field("Category", FieldKind::Select);
    let Created::Option(home_option) = home.create(Mutation::AddOption {
        block: table.clone(),
        field: category.clone(),
        name: "Home".into(),
        color: 0,
    }) else {
        panic!("an option")
    };
    for (item, option) in [
        ("Rent", Some(&home_option)),
        ("Groceries", None),
        ("Train", None),
    ] {
        let row = home.row();
        home.set_cell(&row, &name, Cell::Text(item.into()));
        if let Some(option) = option {
            home.set_cell(&row, &category, Cell::Select(option.clone()));
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
