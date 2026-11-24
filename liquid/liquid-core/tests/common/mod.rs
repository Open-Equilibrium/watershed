#![allow(dead_code)]

use liquid_core::{
    Actor, Applied, AreaId, BlockId, Cell, Created, Error, FieldId, FieldKind, Layout, MemoryStore,
    Mutation, OptionId, Origin, PageId, PersonId, Place, Request, RowId, RowQuery, Store, ViewId,
};

pub type Workspace = liquid_core::Workspace<MemoryStore>;

pub fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

pub fn workspace() -> Workspace {
    Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap()
}

pub fn apply<S: Store>(
    workspace: &mut liquid_core::Workspace<S>,
    mutation: Mutation,
) -> Result<Applied, Error> {
    workspace.apply(Request {
        actor: owner(),
        origin: Origin::Ui,
        mutation,
    })
}

pub fn created<S: Store>(workspace: &mut liquid_core::Workspace<S>, mutation: Mutation) -> Created {
    apply(workspace, mutation).unwrap().created.unwrap()
}

pub fn all() -> RowQuery {
    RowQuery {
        sort: None,
        filter: None,
        within: None,
        start: 0,
        limit: usize::MAX,
        search: None,
        totals: Vec::new(),
    }
}

pub fn area(workspace: &mut Workspace, name: &str) -> AreaId {
    let Created::Area(area) = created(workspace, Mutation::CreateArea { name: name.into() }) else {
        panic!("an Area")
    };
    area
}

pub fn page(workspace: &mut Workspace, area: &AreaId, title: &str) -> PageId {
    let Created::Page(page) = created(
        workspace,
        Mutation::CreatePage {
            area: area.clone(),
            parent: None,
            title: title.into(),
        },
    ) else {
        panic!("a Page")
    };
    page
}

pub fn private_page(title: &str) -> (Workspace, AreaId, PageId) {
    let mut workspace = workspace();
    let area = area(&mut workspace, "Private");
    let page = page(&mut workspace, &area, title);
    (workspace, area, page)
}

pub fn block(workspace: &mut Workspace, mutation: Mutation) -> (BlockId, ViewId) {
    let Created::Block { block, view } = created(workspace, mutation) else {
        panic!("a Block")
    };
    (block, view)
}

pub fn table(workspace: &mut Workspace, page: &PageId, name: &str) -> (BlockId, ViewId) {
    block(
        workspace,
        Mutation::AddDatabaseBlock {
            page: page.clone(),
            after: None,
            name: name.into(),
            layout: Layout::Table,
        },
    )
}

pub fn text_block(workspace: &mut Workspace, page: &PageId, text: &str) -> (BlockId, ViewId) {
    block(
        workspace,
        Mutation::AddTextBlock {
            page: page.clone(),
            after: None,
            text: text.into(),
        },
    )
}

pub fn field(workspace: &mut Workspace, table: &BlockId, name: &str, kind: FieldKind) -> FieldId {
    let Created::Field(field) = created(
        workspace,
        Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: name.into(),
            kind,
        },
    ) else {
        panic!("a field")
    };
    field
}

pub fn option(workspace: &mut Workspace, table: &BlockId, field: &FieldId, name: &str) -> OptionId {
    let Created::Option(option) = created(
        workspace,
        Mutation::AddOption {
            block: table.clone(),
            field: field.clone(),
            name: name.into(),
            color: 0,
        },
    ) else {
        panic!("an option")
    };
    option
}

pub fn row(workspace: &mut Workspace, table: &BlockId) -> RowId {
    let Created::Row(row) = created(
        workspace,
        Mutation::AddRow {
            block: table.clone(),
            place: Place::Last,
        },
    ) else {
        panic!("a row")
    };
    row
}

pub fn set_cell(
    workspace: &mut Workspace,
    table: &BlockId,
    row: &RowId,
    field: &FieldId,
    value: Cell,
) {
    apply(
        workspace,
        Mutation::SetCell {
            block: table.clone(),
            row: row.clone(),
            field: field.clone(),
            value,
        },
    )
    .unwrap();
}
