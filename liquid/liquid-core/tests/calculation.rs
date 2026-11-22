#![allow(clippy::unwrap_used)]

use liquid_core::{
    Actor, Applied, BlockId, Cell, Created, Error, FieldId, FieldKind, LineId, LineSetup,
    LineStyle, MAX_LINES, MemoryStore, Mutation, Origin, PageId, PersonId, Place, Request, RowId,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

struct Home {
    workspace: Workspace,
    page: PageId,
    other: PageId,
    table: BlockId,
    amount: FieldId,
}

fn line(label: &str, formula: &str, style: LineStyle) -> LineSetup {
    LineSetup {
        label: label.into(),
        formula: formula.into(),
        style,
    }
}

impl Home {
    fn new() -> Self {
        let mut home = Self {
            workspace: Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap(),
            page: PageId::parse(&"0".repeat(32)).unwrap(),
            other: PageId::parse(&"0".repeat(32)).unwrap(),
            table: BlockId::parse(&"0".repeat(32)).unwrap(),
            amount: FieldId::parse(&"0".repeat(32)).unwrap(),
        };
        let Some(Created::Area(area)) = home
            .edit(Mutation::CreateArea {
                name: "Private".into(),
            })
            .unwrap()
            .created
        else {
            panic!("an Area")
        };
        let page = |home: &mut Self, title: &str| {
            let Some(Created::Page(page)) = home
                .edit(Mutation::CreatePage {
                    area: area.clone(),
                    parent: None,
                    title: title.into(),
                })
                .unwrap()
                .created
            else {
                panic!("a Page")
            };
            page
        };
        home.page = page(&mut home, "Household budget");
        home.other = page(&mut home, "Review");
        let Some(Created::Block { block, .. }) = home
            .edit(Mutation::AddDatabaseBlock {
                page: home.page.clone(),
                after: None,
                name: "Expenses".into(),
                layout: liquid_core::Layout::Table,
            })
            .unwrap()
            .created
        else {
            panic!("a Table")
        };
        home.table = block;
        let Some(Created::Field(amount)) = home
            .edit(Mutation::AddField {
                block: home.table.clone(),
                place: Place::Last,
                name: "Amount".into(),
                kind: FieldKind::Number,
            })
            .unwrap()
            .created
        else {
            panic!("a field")
        };
        home.amount = amount;
        for value in [900.0, 384.5] {
            let row = home.row();
            home.set(&row, Cell::Number(value));
        }
        home
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        self.workspace.apply(Request {
            actor: Actor::Person(PersonId::new("benni")),
            origin: Origin::Ui,
            mutation,
        })
    }

    fn row(&mut self) -> RowId {
        let Some(Created::Row(row)) = self
            .edit(Mutation::AddRow {
                block: self.table.clone(),
                place: Place::Last,
            })
            .unwrap()
            .created
        else {
            panic!("a row")
        };
        row
    }

    fn set(&mut self, row: &RowId, value: Cell) {
        self.edit(Mutation::SetCell {
            block: self.table.clone(),
            row: row.clone(),
            field: self.amount.clone(),
            value,
        })
        .unwrap();
    }

    fn calculation(&mut self, page: &PageId) -> BlockId {
        let Some(Created::Block { block, .. }) = self
            .edit(Mutation::AddCalculationBlock {
                page: page.clone(),
                after: None,
            })
            .unwrap()
            .created
        else {
            panic!("a Calculation Block")
        };
        block
    }

    fn add(
        &mut self,
        page: &PageId,
        block: &BlockId,
        after: Option<&LineId>,
        setup: LineSetup,
    ) -> Result<LineId, Error> {
        match self
            .edit(Mutation::AddLine {
                page: page.clone(),
                block: block.clone(),
                after: after.cloned(),
                line: setup,
            })?
            .created
        {
            Some(Created::Line(line)) => Ok(line),
            other => panic!("a line, got {other:?}"),
        }
    }

    fn lines(&mut self, page: &PageId, block: &BlockId) -> Vec<(String, String)> {
        let calculation = self.workspace.calculation(page, block).unwrap().unwrap();
        calculation
            .lines
            .iter()
            .map(|line| (line.label.clone(), line.value.to_string()))
            .collect()
    }
}

fn shown(lines: &[(&str, &str)]) -> Vec<(String, String)> {
    lines
        .iter()
        .map(|(label, value)| (label.to_string(), value.to_string()))
        .collect()
}

#[test]
fn a_calculation_block_computes_a_report_in_excel_s_layout() {
    let mut home = Home::new();
    let (page, other) = (home.page.clone(), home.other.clone());
    let report = home.calculation(&page);
    let first = home.workspace.calculation(&page, &report).unwrap().unwrap();
    assert_eq!(first.lines.len(), 1, "a new Block starts with one line");
    let first = first.lines[0].id.clone();
    let column = format!("{{{}.{}}}", home.table, home.amount);
    home.edit(Mutation::SetLine {
        page: page.clone(),
        block: report.clone(),
        line: first.clone(),
        setup: line("Income", "", LineStyle::Heading),
    })
    .unwrap();
    let mut last = first;
    let mut ids = Vec::new();
    for setup in [
        line("Salary", "3200", LineStyle::Plain),
        line("Side project", "450", LineStyle::Plain),
        line("Total income", "SUM(ABOVE)", LineStyle::Subtotal),
        line("Spending", "", LineStyle::Heading),
        line("Expenses", &format!("SUM({column})"), LineStyle::Plain),
        line("Savings plan", "500", LineStyle::Plain),
        line("Total spending", "SUM(ABOVE)", LineStyle::Subtotal),
    ] {
        last = home.add(&page, &report, Some(&last), setup).unwrap();
        ids.push(last.clone());
    }
    let reference = |line: &LineId| format!("{{{page}.{report}.{line}}}");
    let left = home
        .add(
            &page,
            &report,
            Some(&last),
            line(
                "Left over",
                &format!("{} − {}", reference(&ids[2]), reference(&ids[6])),
                LineStyle::Total,
            ),
        )
        .unwrap();
    let expected = [
        ("Income", ""),
        ("Salary", "3200"),
        ("Side project", "450"),
        ("Total income", "3650"),
        ("Spending", ""),
        ("Expenses", "1284.5"),
        ("Savings plan", "500"),
        ("Total spending", "1784.5"),
        ("Left over", "1865.5"),
    ];
    assert_eq!(home.lines(&page, &report), shown(&expected));

    let row = home.row();
    home.set(&row, Cell::Number(15.5));
    home.edit(Mutation::SetLine {
        page: page.clone(),
        block: report.clone(),
        line: ids[2].clone(),
        setup: line("Earned", "SUM(ABOVE)", LineStyle::Subtotal),
    })
    .unwrap();
    let elsewhere = home.calculation(&other);
    let copied = home
        .workspace
        .calculation(&other, &elsewhere)
        .unwrap()
        .unwrap()
        .lines[0]
        .id
        .clone();
    home.edit(Mutation::SetLine {
        page: other.clone(),
        block: elsewhere.clone(),
        line: copied,
        setup: line(
            "Left, twice",
            &format!("{} * 2", reference(&left)),
            LineStyle::Plain,
        ),
    })
    .unwrap();
    assert_eq!(
        home.lines(&other, &elsewhere),
        shown(&[("Left, twice", "3700")]),
        "a renamed line and a new row both carry through to another Page"
    );

    home.edit(Mutation::RemoveLine {
        page: page.clone(),
        block: report.clone(),
        line: ids[6].clone(),
    })
    .unwrap();
    home.edit(Mutation::SetLine {
        page: page.clone(),
        block: report.clone(),
        line: ids[1].clone(),
        setup: line("Side project", &reference(&left), LineStyle::Plain),
    })
    .unwrap();
    let lines = home.lines(&page, &report);
    assert_eq!(lines.len(), 8);
    assert_eq!(
        lines[2].1, "#REF!",
        "a line that refers to itself, through others"
    );
    assert_eq!(lines[7].1, "#REF!", "a removed line");
}

#[test]
fn lines_are_checked_before_they_are_written() {
    let mut home = Home::new();
    let page = home.page.clone();
    let report = home.calculation(&page);
    for (setup, error) in [
        (
            line("Line", "SUM(1,", LineStyle::Plain),
            Error::Formula { at: 6 },
        ),
        (
            line(&"x".repeat(1_001), "", LineStyle::Plain),
            Error::TooLong,
        ),
        (
            line(
                "",
                &"1".repeat(liquid_core::MAX_FORMULA + 1),
                LineStyle::Plain,
            ),
            Error::TooLong,
        ),
    ] {
        assert_eq!(home.add(&page, &report, None, setup), Err(error));
    }
    let table = home.table.clone();
    assert_eq!(
        home.edit(Mutation::AddLine {
            page: page.clone(),
            block: table,
            after: None,
            line: line("", "", LineStyle::Plain),
        }),
        Err(Error::NotFound),
        "only a Calculation Block holds lines"
    );
    let mut after = None;
    for _ in 1..MAX_LINES {
        after = Some(
            home.add(
                &page,
                &report,
                after.as_ref(),
                line("", "1", LineStyle::Plain),
            )
            .unwrap(),
        );
    }
    assert_eq!(
        home.add(&page, &report, None, line("", "", LineStyle::Plain)),
        Err(Error::TooMany)
    );
}
