use liquid_core::{
    AreaId, AttachmentId, AutomationId, BlockId, Changed, Error, FieldId, LineId, Mutation, PageId,
    Place, RowId, TabId, TextStyle, Tile, ViewId,
};

use super::automation::{BridgeAutomationSetup, BridgeRan, BridgeWorkspaceAutomation};
use super::calculation::{BridgeLine, BridgeLineSetup};
use super::chart::{BridgeChart, BridgeChartSetup};
use super::database::{
    BridgeCell, BridgeComputed, BridgeDatabase, BridgeFieldKind, BridgeFieldTotal, BridgeFilter,
    BridgeGroup, BridgeLayout, BridgeRow, BridgeSort, BridgeTab, BridgeTotal, filter, sort,
};
use super::drawing::{BridgeDrawing, BridgeDrawingEdit, BridgeDrawingStyle};
use super::media::BridgeMedia;
use super::role::{BridgeRole, BridgeRoleAllows, BridgeRoleSetup, role_id};

pub const MAX_WINDOW: i64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMutationKind {
    RenameWorkspace,
    SetWorkspaceIcon,
    CreateArea,
    CreatePage,
    RenamePage,
    AddTextBlock,
    EditText,
    RemoveView,
    SetTile,
    SetInnerTile,
    MoveViews,
    AddTab,
    RenameTab,
    SetTabLayout,
    SetTabGroup,
    SetTabDate,
    SetTabSort,
    SetTabFilter,
    SetTabTotal,
    SetTabFolded,
    MoveTab,
    RemoveTab,
    SplitTextBlock,
    JoinTextBlock,
    SetTextStyle,
    SetSectionBoundary,
    TurnIntoCode,
    TurnIntoText,
    TurnIntoMath,
    SetCodeLanguage,
    AddMediaBlock,
    AddWhiteboardBlock,
    AddCalculationBlock,
    AddChartBlock,
    SetChart,
    AddLine,
    SetLine,
    RemoveLine,
    SetDrawingStyle,
    EditDrawing,
    AddDatabaseBlock,
    CreateTable,
    AddTableView,
    DuplicateTable,
    RenameView,
    DeleteTable,
    RenameDatabase,
    AddField,
    RenameField,
    SetFieldFormula,
    RemoveField,
    AddOption,
    AddRow,
    RemoveRow,
    MoveRow,
    SetCell,
    CreateAutomation,
    SetAutomation,
    SetAutomationOn,
    RemoveAutomation,
    SetAutomationRole,
    CreateRole,
    SetRole,
    RemoveRole,
    ProtectTable,
    UnprotectTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTextStyle {
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    Bullet,
    Numbered,
    Todo,
    Done,
    Quote,
}

impl BridgeTextStyle {
    fn to_core(self) -> TextStyle {
        match self {
            Self::Paragraph => TextStyle::Paragraph,
            Self::Heading1 => TextStyle::Heading(1),
            Self::Heading2 => TextStyle::Heading(2),
            Self::Heading3 => TextStyle::Heading(3),
            Self::Bullet => TextStyle::Bullet,
            Self::Numbered => TextStyle::Numbered,
            Self::Todo => TextStyle::Todo { done: false },
            Self::Done => TextStyle::Todo { done: true },
            Self::Quote => TextStyle::Quote,
        }
    }

    pub(crate) fn of(style: TextStyle) -> Self {
        match style {
            TextStyle::Heading(1) => Self::Heading1,
            TextStyle::Heading(2) => Self::Heading2,
            TextStyle::Heading(_) => Self::Heading3,
            TextStyle::Bullet => Self::Bullet,
            TextStyle::Numbered => Self::Numbered,
            TextStyle::Todo { done: false } => Self::Todo,
            TextStyle::Todo { done: true } => Self::Done,
            TextStyle::Quote => Self::Quote,
            TextStyle::Paragraph => Self::Paragraph,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeMutation {
    pub kind: BridgeMutationKind,
    pub area: Option<String>,
    pub page: Option<String>,
    pub parent: Option<String>,
    pub block: Option<String>,
    pub view: Option<String>,
    pub after: Option<String>,
    pub last: bool,
    pub text: Option<String>,
    pub at: i64,
    pub delete: i64,
    pub style: BridgeTextStyle,
    pub boundary: bool,
    pub field: Option<String>,
    pub row: Option<String>,
    pub field_kind: Option<BridgeFieldKind>,
    pub color: i64,
    pub cell: Option<BridgeCell>,
    pub sort: Option<BridgeSort>,
    pub filter: Option<BridgeFilter>,
    pub tab: Option<String>,
    pub tile: Option<BridgeTile>,
    pub last_view: Option<String>,
    pub layout: BridgeLayout,
    pub tabs: Vec<BridgeTab>,
    pub media_type: Option<String>,
    pub attachment: Option<String>,
    pub drawing_style: Option<BridgeDrawingStyle>,
    pub drawing: Option<BridgeDrawingEdit>,
    pub line: Option<String>,
    pub line_setup: Option<BridgeLineSetup>,
    pub total: Option<BridgeTotal>,
    pub chart: Option<BridgeChartSetup>,
    pub automation: Option<String>,
    pub automation_setup: Option<BridgeAutomationSetup>,
    pub on: bool,
    pub role: Option<String>,
    pub role_setup: Option<BridgeRoleSetup>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeApplied {
    pub action: String,
    pub area: Option<String>,
    pub page: Option<String>,
    pub block: Option<String>,
    pub view: Option<String>,
    pub field: Option<String>,
    pub row: Option<String>,
    pub option: Option<String>,
    pub tab: Option<String>,
    pub elements: Vec<String>,
    pub line: Option<String>,
    pub automation: Option<String>,
    pub role: Option<String>,
    pub ran: Vec<BridgeRan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeQueryKind {
    Workspace,
    Areas,
    Pages,
    Views,
    Text,
    Code,
    Math,
    Media,
    Drawing,
    Calculation,
    Chart,
    Database,
    Rows,
    Groups,
    Search,
    Tables,
    TableViews,
    Automations,
    Roles,
    Allows,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeQuery {
    pub kind: BridgeQueryKind,
    pub area: Option<String>,
    pub page: Option<String>,
    pub block: Option<String>,
    pub start: i64,
    pub limit: i64,
    pub field: Option<String>,
    pub sort: Option<BridgeSort>,
    pub filter: Option<BridgeFilter>,
    pub within: Option<BridgeFilter>,
    pub search: Option<String>,
    pub totals: Vec<BridgeFieldTotal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeArea {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePage {
    pub id: String,
    pub parent: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTable {
    pub id: String,
    pub area: String,
    pub name: String,
    pub protected: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeShown {
    pub page: String,
    pub view: String,
    pub title: String,
    pub tabs: Vec<BridgeTab>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeBlockKind {
    Text,
    Database,
    Code,
    Media,
    Whiteboard,
    Calculation,
    Chart,
    Math,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeView {
    pub id: String,
    pub block: String,
    pub title: String,
    pub boundary: bool,
    pub kind: BridgeBlockKind,
    pub tabs: Vec<BridgeTab>,
    pub tile: Option<BridgeTile>,
    pub inner: Option<BridgeTile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFound {
    Page,
    Table,
    Text,
    Source,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHit {
    pub found: BridgeFound,
    pub id: String,
    pub page: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeTile {
    pub column: u8,
    pub span: u8,
}

impl BridgeTile {
    pub(crate) fn of(tile: Tile) -> Self {
        Self {
            column: tile.column,
            span: tile.span,
        }
    }

    fn to_core(self) -> Tile {
        Tile {
            column: self.column,
            span: self.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAnswer {
    pub areas: Vec<BridgeArea>,
    pub pages: Vec<BridgePage>,
    pub views: Vec<BridgeView>,
    pub count: i64,
    pub arranged: bool,
    pub text: Option<String>,
    pub style: Option<BridgeTextStyle>,
    pub language: Option<String>,
    pub media: Option<BridgeMedia>,
    pub drawing: Option<BridgeDrawing>,
    pub database: Option<BridgeDatabase>,
    pub rows: Vec<BridgeRow>,
    pub groups: Vec<BridgeGroup>,
    pub hits: Vec<BridgeHit>,
    pub tables: Vec<BridgeTable>,
    pub shown: Vec<BridgeShown>,
    pub icon: Option<String>,
    pub lines: Vec<BridgeLine>,
    pub totals: Vec<BridgeComputed>,
    pub chart: Option<BridgeChart>,
    pub follows: Vec<BridgeResource>,
    pub automations: Vec<BridgeWorkspaceAutomation>,
    pub roles: Vec<BridgeRole>,
    pub allows: Vec<BridgeRoleAllows>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BridgeResourceKind {
    Workspace,
    Areas,
    Pages,
    Views,
    Block,
    Tables,
    Table,
    Automations,
    Roles,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BridgeResource {
    pub kind: BridgeResourceKind,
    pub area: Option<String>,
    pub page: Option<String>,
    pub block: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeChange {
    pub resource: BridgeResource,
    pub seq: i64,
}

pub(crate) fn follows(uses: &[liquid_core::Used]) -> Vec<BridgeResource> {
    uses.iter()
        .map(|used| match used {
            liquid_core::Used::Table(table) => BridgeResource {
                kind: BridgeResourceKind::Table,
                area: None,
                page: None,
                block: Some(table.to_string()),
            },
            liquid_core::Used::Block(page, block) => BridgeResource {
                kind: BridgeResourceKind::Block,
                area: None,
                page: Some(page.to_string()),
                block: Some(block.to_string()),
            },
        })
        .collect()
}

pub(crate) fn message(error: Error) -> String {
    match error {
        Error::NotAllowed => "not allowed".into(),
        Error::NotFound => "not found".into(),
        Error::OutOfRange => "text position out of range".into(),
        Error::NoChange => "the write changes nothing".into(),
        Error::SectionBoundary => "the Text Block starts a Section".into(),
        Error::WrongKind => "the value is not of its kind or not valid".into(),
        Error::TooMany => "it holds the most it may".into(),
        Error::TooLong => "no more fit at this spot; move one of its neighbors first".into(),
        Error::TooLarge => "a file can be at most 5 GiB".into(),
        Error::Formula { at } => format!("the formula can't be read from character {}", at + 1),
        Error::Denied { capability, table } => {
            format!("its Role allows no {capability} on “{table}”")
        }
        Error::Changed => "the file changed while it was added; add it again".into(),
        Error::Conflict => {
            "a later change touched the same thing; restore a version instead".into()
        }
        Error::Purged => "its content was deleted for good".into(),
        Error::Random => "the random source failed".into(),
        Error::Document(message) | Error::Storage(message) => message,
    }
}

pub(crate) fn id<T>(
    text: &Option<String>,
    parse: fn(&str) -> Option<T>,
    what: &str,
) -> Result<T, String> {
    text.as_deref()
        .and_then(parse)
        .ok_or_else(|| format!("{what} is missing or no identity"))
}

fn optional_id<T>(
    text: &Option<String>,
    parse: fn(&str) -> Option<T>,
    what: &str,
) -> Result<Option<T>, String> {
    text.as_ref().map(|_| id(text, parse, what)).transpose()
}

impl BridgeMutation {
    fn line_setup(&self) -> Result<liquid_core::LineSetup, String> {
        Ok(self
            .line_setup
            .as_ref()
            .ok_or("line_setup is missing")?
            .to_core())
    }

    fn role_setup(&self) -> Result<liquid_core::RoleSetup, String> {
        self.role_setup
            .as_ref()
            .ok_or("role_setup is missing")?
            .to_core()
    }

    fn automation_setup(&self) -> Result<liquid_core::AutomationSetup, String> {
        self.automation_setup
            .as_ref()
            .ok_or("automation_setup is missing")?
            .to_core()
    }

    fn place<T>(&self, parse: fn(&str) -> Option<T>) -> Result<Place<T>, String> {
        match (self.last, optional_id(&self.after, parse, "after")?) {
            (true, Some(_)) => Err("after and last cannot both be set".into()),
            (true, None) => Ok(Place::Last),
            (false, Some(after)) => Ok(Place::After(after)),
            (false, None) => Ok(Place::First),
        }
    }
}

pub(crate) fn units(value: i64, what: &str) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| format!("{what} is negative"))
}

impl BridgeMutation {
    pub(crate) fn to_core(&self) -> Result<Mutation, String> {
        let text = || {
            self.text
                .clone()
                .ok_or_else(|| "text is missing".to_string())
        };
        let page = || id(&self.page, PageId::parse, "page");
        let block = || id(&self.block, BlockId::parse, "block");
        let field = || id(&self.field, FieldId::parse, "field");
        let row = || id(&self.row, RowId::parse, "row");
        let view = || id(&self.view, ViewId::parse, "view");
        let tab = || id(&self.tab, TabId::parse, "tab");
        let automation = || id(&self.automation, AutomationId::parse, "automation");
        let tabs = || {
            self.tabs
                .iter()
                .map(BridgeTab::to_core)
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(match self.kind {
            BridgeMutationKind::RenameWorkspace => Mutation::RenameWorkspace { name: text()? },
            BridgeMutationKind::SetWorkspaceIcon => Mutation::SetWorkspaceIcon { icon: text()? },
            BridgeMutationKind::CreateArea => Mutation::CreateArea { name: text()? },
            BridgeMutationKind::CreatePage => Mutation::CreatePage {
                area: id(&self.area, AreaId::parse, "area")?,
                parent: optional_id(&self.parent, PageId::parse, "parent")?,
                title: text()?,
            },
            BridgeMutationKind::RenamePage => Mutation::RenamePage {
                page: id(&self.page, PageId::parse, "page")?,
                title: text()?,
            },
            BridgeMutationKind::AddTextBlock => Mutation::AddTextBlock {
                page: id(&self.page, PageId::parse, "page")?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
                text: text()?,
            },
            BridgeMutationKind::EditText => Mutation::EditText {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
                at: units(self.at, "at")?,
                delete: units(self.delete, "delete")?,
                insert: text()?,
            },
            BridgeMutationKind::RemoveView => Mutation::RemoveView {
                page: id(&self.page, PageId::parse, "page")?,
                view: id(&self.view, ViewId::parse, "view")?,
            },
            BridgeMutationKind::SetTile => Mutation::SetTile {
                page: page()?,
                view: view()?,
                tile: self.tile.map(BridgeTile::to_core),
            },
            BridgeMutationKind::SetInnerTile => Mutation::SetInnerTile {
                page: page()?,
                view: view()?,
                tile: self.tile.map(BridgeTile::to_core),
            },
            BridgeMutationKind::MoveViews => Mutation::MoveViews {
                page: page()?,
                first: view()?,
                last: id(&self.last_view, ViewId::parse, "last view")?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
            },
            BridgeMutationKind::AddTab => Mutation::AddTab {
                page: page()?,
                view: view()?,
                after: optional_id(&self.after, TabId::parse, "after")?,
                setup: Box::new(self.tabs.first().ok_or("tabs is empty")?.to_core()?),
            },
            BridgeMutationKind::RenameTab => Mutation::RenameTab {
                page: page()?,
                view: view()?,
                tab: tab()?,
                name: text()?,
            },
            BridgeMutationKind::SetTabLayout => Mutation::SetTabLayout {
                page: page()?,
                view: view()?,
                tab: tab()?,
                layout: self.layout.to_core(),
            },
            BridgeMutationKind::SetTabGroup => Mutation::SetTabGroup {
                page: page()?,
                view: view()?,
                tab: tab()?,
                group: optional_id(&self.field, FieldId::parse, "field")?,
            },
            BridgeMutationKind::SetTabDate => Mutation::SetTabDate {
                page: page()?,
                view: view()?,
                tab: tab()?,
                date: optional_id(&self.field, FieldId::parse, "field")?,
            },
            BridgeMutationKind::SetTabSort => Mutation::SetTabSort {
                page: page()?,
                view: view()?,
                tab: tab()?,
                sort: sort(&self.sort)?,
            },
            BridgeMutationKind::SetTabFilter => Mutation::SetTabFilter {
                page: page()?,
                view: view()?,
                tab: tab()?,
                filter: filter(&self.filter)?,
            },
            BridgeMutationKind::SetTabTotal => Mutation::SetTabTotal {
                page: page()?,
                view: view()?,
                tab: tab()?,
                field: field()?,
                total: self.total.map(BridgeTotal::to_core),
            },
            BridgeMutationKind::SetTabFolded => Mutation::SetTabFolded {
                page: page()?,
                view: view()?,
                tab: tab()?,
                folded: self.tabs.first().ok_or("tabs is empty")?.to_core()?.folded,
            },
            BridgeMutationKind::MoveTab => Mutation::MoveTab {
                page: page()?,
                view: view()?,
                tab: tab()?,
                after: optional_id(&self.after, TabId::parse, "after")?,
            },
            BridgeMutationKind::RemoveTab => Mutation::RemoveTab {
                page: page()?,
                view: view()?,
                tab: tab()?,
            },
            BridgeMutationKind::SplitTextBlock => Mutation::SplitTextBlock {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
                view: id(&self.view, ViewId::parse, "view")?,
                at: units(self.at, "at")?,
                style: self.style.to_core(),
            },
            BridgeMutationKind::JoinTextBlock => Mutation::JoinTextBlock {
                page: id(&self.page, PageId::parse, "page")?,
                view: id(&self.view, ViewId::parse, "view")?,
            },
            BridgeMutationKind::SetTextStyle => Mutation::SetTextStyle {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
                style: self.style.to_core(),
            },
            BridgeMutationKind::SetSectionBoundary => Mutation::SetSectionBoundary {
                page: id(&self.page, PageId::parse, "page")?,
                view: id(&self.view, ViewId::parse, "view")?,
                boundary: self.boundary,
            },
            BridgeMutationKind::TurnIntoCode => Mutation::TurnIntoCode {
                page: page()?,
                block: block()?,
            },
            BridgeMutationKind::TurnIntoText => Mutation::TurnIntoText {
                page: page()?,
                block: block()?,
            },
            BridgeMutationKind::TurnIntoMath => Mutation::TurnIntoMath {
                page: page()?,
                block: block()?,
            },
            BridgeMutationKind::SetCodeLanguage => Mutation::SetCodeLanguage {
                page: page()?,
                block: block()?,
                language: text()?,
            },
            BridgeMutationKind::AddMediaBlock => Mutation::AddMediaBlock {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
                name: text()?,
                media_type: self.media_type.clone().unwrap_or_default(),
                attachment: id(&self.attachment, AttachmentId::parse, "attachment")?,
            },
            BridgeMutationKind::AddWhiteboardBlock => Mutation::AddWhiteboardBlock {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
            },
            BridgeMutationKind::AddCalculationBlock => Mutation::AddCalculationBlock {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
            },
            BridgeMutationKind::AddChartBlock => Mutation::AddChartBlock {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
            },
            BridgeMutationKind::SetChart => Mutation::SetChart {
                page: page()?,
                block: block()?,
                setup: self.chart.as_ref().ok_or("chart is missing")?.to_core()?,
            },
            BridgeMutationKind::AddLine => Mutation::AddLine {
                page: page()?,
                block: block()?,
                after: optional_id(&self.after, LineId::parse, "after")?,
                line: self.line_setup()?,
            },
            BridgeMutationKind::SetLine => Mutation::SetLine {
                page: page()?,
                block: block()?,
                line: id(&self.line, LineId::parse, "line")?,
                setup: self.line_setup()?,
            },
            BridgeMutationKind::RemoveLine => Mutation::RemoveLine {
                page: page()?,
                block: block()?,
                line: id(&self.line, LineId::parse, "line")?,
            },
            BridgeMutationKind::SetDrawingStyle => Mutation::SetDrawingStyle {
                page: page()?,
                block: block()?,
                style: self
                    .drawing_style
                    .ok_or("drawing_style is missing")?
                    .to_core(),
            },
            BridgeMutationKind::EditDrawing => {
                let (add, change, remove) = self
                    .drawing
                    .as_ref()
                    .ok_or("drawing is missing")?
                    .to_core()?;
                Mutation::EditDrawing {
                    page: page()?,
                    block: block()?,
                    add,
                    change,
                    remove,
                }
            }
            BridgeMutationKind::AddDatabaseBlock => Mutation::AddDatabaseBlock {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
                name: text()?,
                layout: self.layout.to_core(),
            },
            BridgeMutationKind::DuplicateTable => Mutation::DuplicateTable {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
                table: block()?,
                name: text()?,
                tabs: tabs()?,
            },
            BridgeMutationKind::CreateTable => Mutation::CreateTable {
                area: id(&self.area, AreaId::parse, "area")?,
                name: text()?,
            },
            BridgeMutationKind::AddTableView => Mutation::AddTableView {
                page: page()?,
                after: optional_id(&self.after, ViewId::parse, "after")?,
                table: block()?,
                title: text()?,
                tabs: tabs()?,
            },
            BridgeMutationKind::RenameView => Mutation::RenameView {
                page: page()?,
                view: id(&self.view, ViewId::parse, "view")?,
                title: text()?,
            },
            BridgeMutationKind::DeleteTable => Mutation::DeleteTable { block: block()? },
            BridgeMutationKind::RenameDatabase => Mutation::RenameDatabase {
                block: block()?,
                name: text()?,
            },
            BridgeMutationKind::AddField => Mutation::AddField {
                block: block()?,
                place: self.place(FieldId::parse)?,
                name: text()?,
                kind: self.field_kind.ok_or("field kind is missing")?.to_core(),
            },
            BridgeMutationKind::SetFieldFormula => Mutation::SetFieldFormula {
                block: block()?,
                field: field()?,
                formula: text()?,
            },
            BridgeMutationKind::RenameField => Mutation::RenameField {
                block: block()?,
                field: field()?,
                name: text()?,
            },
            BridgeMutationKind::RemoveField => Mutation::RemoveField {
                block: block()?,
                field: field()?,
            },
            BridgeMutationKind::AddOption => Mutation::AddOption {
                block: block()?,
                field: field()?,
                name: text()?,
                color: u8::try_from(self.color).map_err(|_| "color is out of range")?,
            },
            BridgeMutationKind::AddRow => Mutation::AddRow {
                block: block()?,
                place: self.place(RowId::parse)?,
            },
            BridgeMutationKind::RemoveRow => Mutation::RemoveRow {
                block: block()?,
                row: row()?,
            },
            BridgeMutationKind::MoveRow => Mutation::MoveRow {
                block: block()?,
                row: row()?,
                place: self.place(RowId::parse)?,
            },
            BridgeMutationKind::SetCell => Mutation::SetCell {
                block: block()?,
                row: row()?,
                field: field()?,
                value: self.cell.as_ref().ok_or("cell is missing")?.to_core()?,
            },
            BridgeMutationKind::CreateAutomation => Mutation::CreateAutomation {
                setup: Box::new(self.automation_setup()?),
            },
            BridgeMutationKind::SetAutomation => Mutation::SetAutomation {
                automation: automation()?,
                setup: Box::new(self.automation_setup()?),
            },
            BridgeMutationKind::SetAutomationOn => Mutation::SetAutomationOn {
                automation: automation()?,
                on: self.on,
            },
            BridgeMutationKind::RemoveAutomation => Mutation::RemoveAutomation {
                automation: automation()?,
            },
            BridgeMutationKind::SetAutomationRole => Mutation::SetAutomationRole {
                automation: automation()?,
                role: role_id(&self.role)?,
            },
            BridgeMutationKind::CreateRole => Mutation::CreateRole {
                setup: self.role_setup()?,
            },
            BridgeMutationKind::SetRole => Mutation::SetRole {
                role: role_id(&self.role)?,
                setup: self.role_setup()?,
            },
            BridgeMutationKind::RemoveRole => Mutation::RemoveRole {
                role: role_id(&self.role)?,
            },
            BridgeMutationKind::ProtectTable => Mutation::ProtectTable {
                table: block()?,
                roles: self
                    .roles
                    .iter()
                    .map(|role| role_id(&Some(role.clone())))
                    .collect::<Result<_, _>>()?,
            },
            BridgeMutationKind::UnprotectTable => Mutation::UnprotectTable { table: block()? },
        })
    }
}

impl BridgeResource {
    pub(crate) fn normalized(&self) -> Result<Self, String> {
        let area = || id(&self.area, AreaId::parse, "area").map(|area| Some(area.to_string()));
        let page = || id(&self.page, PageId::parse, "page").map(|page| Some(page.to_string()));
        let block =
            || id(&self.block, BlockId::parse, "block").map(|block| Some(block.to_string()));
        let (area, page, block) = match self.kind {
            BridgeResourceKind::Workspace | BridgeResourceKind::Areas => (None, None, None),
            BridgeResourceKind::Pages => (area()?, None, None),
            BridgeResourceKind::Views => (None, page()?, None),
            BridgeResourceKind::Block => (None, page()?, block()?),
            BridgeResourceKind::Tables
            | BridgeResourceKind::Automations
            | BridgeResourceKind::Roles => (None, None, None),
            BridgeResourceKind::Table => (None, None, block()?),
        };
        Ok(Self {
            kind: self.kind,
            area,
            page,
            block,
        })
    }

    pub(crate) fn of(changed: &Changed) -> Self {
        let (kind, area, page, block) = match changed {
            Changed::Workspace => (BridgeResourceKind::Workspace, None, None, None),
            Changed::Areas => (BridgeResourceKind::Areas, None, None, None),
            Changed::Pages(area) => (
                BridgeResourceKind::Pages,
                Some(area.to_string()),
                None,
                None,
            ),
            Changed::Views(page) => (
                BridgeResourceKind::Views,
                None,
                Some(page.to_string()),
                None,
            ),
            Changed::Block(page, block) => (
                BridgeResourceKind::Block,
                None,
                Some(page.to_string()),
                Some(block.to_string()),
            ),
            Changed::Tables => (BridgeResourceKind::Tables, None, None, None),
            Changed::Automations => (BridgeResourceKind::Automations, None, None, None),
            Changed::Roles => (BridgeResourceKind::Roles, None, None, None),
            Changed::Table(block) => (
                BridgeResourceKind::Table,
                None,
                None,
                Some(block.to_string()),
            ),
        };
        Self {
            kind,
            area,
            page,
            block,
        }
    }

    pub(crate) fn page(&self) -> Option<&str> {
        matches!(
            self.kind,
            BridgeResourceKind::Views | BridgeResourceKind::Block
        )
        .then_some(self.page.as_deref())
        .flatten()
    }

    pub(crate) fn table(&self) -> Option<&str> {
        (self.kind == BridgeResourceKind::Table)
            .then_some(self.block.as_deref())
            .flatten()
    }
}

pub(crate) fn window(start: i64, limit: i64) -> Result<(usize, usize), String> {
    Ok((
        units(start, "start")?,
        units(limit.min(MAX_WINDOW), "limit")?,
    ))
}
