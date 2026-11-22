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
    let named = home.workspace.calculation(&page, &report).unwrap().unwrap();
    assert_eq!(
        named.lines[5].references,
        [Some("Expenses.Amount".to_string())]
    );
    assert_eq!(
        named.lines[8].references,
        [
            Some("Total income".to_string()),
            Some("Total spending".to_string())
        ]
    );

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
    let named = home.workspace.calculation(&page, &report).unwrap().unwrap();
    assert_eq!(
        named.lines[7].references,
        [Some("Earned".to_string()), None],
        "a removed line has no name"
    );
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

#[test]
fn a_formula_field_computes_each_row_from_the_row_s_fields() {
    let mut home = Home::new();
    let (table, amount) = (home.table.clone(), home.amount.clone());
    let field = |home: &mut Home, name: &str| {
        let Some(Created::Field(field)) = home
            .edit(Mutation::AddField {
                block: table.clone(),
                place: Place::Last,
                name: name.into(),
                kind: FieldKind::Formula,
            })
            .unwrap()
            .created
        else {
            panic!("a field")
        };
        field
    };
    let (with_tip, share) = (field(&mut home, "With tip"), field(&mut home, "Share"));
    let formula = |home: &mut Home, field: &FieldId, formula: String| {
        home.edit(Mutation::SetFieldFormula {
            block: table.clone(),
            field: field.clone(),
            formula,
        })
    };
    formula(&mut home, &with_tip, format!("{{{amount}}} * 1.1")).unwrap();
    formula(&mut home, &share, format!("{{{with_tip}}} / 2")).unwrap();
    let computed = |home: &mut Home| {
        let rows = home.workspace.rows(&table, &query(Vec::new())).unwrap();
        rows.rows
            .iter()
            .map(|row| {
                row.computed
                    .iter()
                    .map(|(field, value)| (field.clone(), value.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        computed(&mut home),
        [
            [
                (with_tip.clone(), "990".to_string()),
                (share.clone(), "495".to_string())
            ],
            [
                (with_tip.clone(), "422.95".to_string()),
                (share.clone(), "211.475".to_string())
            ],
        ]
    );

    let page = home.page.clone();
    let report = home.calculation(&page);
    let first = home
        .workspace
        .calculation(&page, &report)
        .unwrap()
        .unwrap()
        .lines[0]
        .id
        .clone();
    home.edit(Mutation::SetLine {
        page: page.clone(),
        block: report.clone(),
        line: first,
        setup: line(
            "Shares",
            &format!("SUM({{{table}.{share}}})"),
            LineStyle::Plain,
        ),
    })
    .unwrap();
    assert_eq!(home.lines(&page, &report), shown(&[("Shares", "706.475")]));

    formula(&mut home, &with_tip, format!("{{{share}}} + 1")).unwrap();
    assert_eq!(
        computed(&mut home)[0][0].1,
        "#REF!",
        "two fields that need each other"
    );

    let view = home
        .workspace
        .views(&page, 0, 10)
        .unwrap()
        .into_iter()
        .find(|view| view.block == table)
        .unwrap();
    let by_share = home.edit(Mutation::SetTabSort {
        page: page.clone(),
        view: view.id.clone(),
        tab: view.tabs[0].id.clone(),
        sort: Some(liquid_core::Sort {
            field: share.clone(),
            descending: false,
        }),
    });
    let row = home.row();
    for (refused, error) in [
        (by_share, Error::WrongKind),
        (
            home.edit(Mutation::SetCell {
                block: table.clone(),
                row,
                field: share.clone(),
                value: Cell::Number(1.0),
            }),
            Error::WrongKind,
        ),
        (formula(&mut home, &amount, "1".into()), Error::WrongKind),
        (
            formula(&mut home, &share, "SUM(".into()),
            Error::Formula { at: 4 },
        ),
    ] {
        assert_eq!(refused.map(|_| ()), Err(error));
    }
}

#[test]
fn a_tab_s_total_row_counts_the_rows_it_shows() {
    use liquid_core::{FieldTotal, Filter, Match, RowQuery, Total};
    let mut home = Home::new();
    let (table, amount, page) = (home.table.clone(), home.amount.clone(), home.page.clone());
    home.row();
    let Some(Created::Field(paid)) = home
        .edit(Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: "Paid".into(),
            kind: FieldKind::Checkbox,
        })
        .unwrap()
        .created
    else {
        panic!("a field")
    };
    let first = home
        .workspace
        .rows(&table, &query(Vec::new()))
        .unwrap()
        .rows[0]
        .id
        .clone();
    home.edit(Mutation::SetCell {
        block: table.clone(),
        row: first,
        field: paid.clone(),
        value: Cell::Checkbox(true),
    })
    .unwrap();
    let view = home
        .workspace
        .views(&page, 0, 10)
        .unwrap()
        .into_iter()
        .find(|view| view.block == table)
        .unwrap();
    let tab = view.tabs[0].id.clone();
    let totals = [
        Total::Sum,
        Total::Average,
        Total::Count,
        Total::Min,
        Total::Max,
        Total::Empty,
        Total::Filled,
    ];
    for total in totals {
        home.edit(Mutation::SetTabTotal {
            page: page.clone(),
            view: view.id.clone(),
            tab: tab.clone(),
            field: amount.clone(),
            total: Some(total),
        })
        .unwrap();
        let stored = home.workspace.view(&page, &view.id).unwrap().unwrap().tabs[0]
            .setup
            .totals
            .clone();
        assert_eq!(
            stored,
            [FieldTotal {
                field: amount.clone(),
                total
            }]
        );
    }
    let shown = |home: &mut Home, filter: Option<Filter>| {
        let all: Vec<_> = totals
            .iter()
            .map(|total| FieldTotal {
                field: amount.clone(),
                total: *total,
            })
            .collect();
        let rows = home
            .workspace
            .rows(
                &table,
                &RowQuery {
                    filter,
                    ..query(all)
                },
            )
            .unwrap();
        rows.totals
            .iter()
            .map(|(_, value)| value.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        shown(&mut home, None),
        ["1284.5", "642.25", "3", "384.5", "900", "1", "2"]
    );
    let unpaid = Filter {
        field: paid.clone(),
        matches: Match::Is(Cell::Checkbox(false)),
    };
    assert_eq!(
        shown(&mut home, Some(unpaid)),
        ["384.5", "384.5", "2", "384.5", "384.5", "1", "1"],
        "a filter changes what it counts"
    );
    let groups = home
        .workspace
        .groups(
            &table,
            &query(vec![FieldTotal {
                field: amount.clone(),
                total: Total::Sum,
            }]),
            &paid,
        )
        .unwrap();
    let sums: Vec<_> = groups
        .iter()
        .map(|group| (group.value.clone(), group.totals[0].1.to_string()))
        .collect();
    assert_eq!(
        sums,
        [
            (Cell::Checkbox(false), "384.5".to_string()),
            (Cell::Checkbox(true), "900".to_string())
        ]
    );

    let refused = home.edit(Mutation::SetTabTotal {
        page: page.clone(),
        view: view.id.clone(),
        tab: tab.clone(),
        field: paid,
        total: Some(Total::Sum),
    });
    assert_eq!(
        refused.map(|_| ()),
        Err(Error::WrongKind),
        "a sum needs numbers"
    );
    home.edit(Mutation::SetTabTotal {
        page: page.clone(),
        view: view.id.clone(),
        tab,
        field: amount,
        total: None,
    })
    .unwrap();
    assert!(
        home.workspace.view(&page, &view.id).unwrap().unwrap().tabs[0]
            .setup
            .totals
            .is_empty()
    );
}

fn query(totals: Vec<liquid_core::FieldTotal>) -> liquid_core::RowQuery {
    liquid_core::RowQuery {
        sort: None,
        filter: None,
        within: None,
        start: 0,
        limit: 10,
        search: None,
        totals,
    }
}

#[test]
fn a_chart_draws_a_table_s_groups_or_a_calculation_s_lines() {
    use liquid_core::{ChartKind, ChartSetup, ChartSource, ChartValue};
    let mut home = Home::new();
    let (table, amount, page) = (home.table.clone(), home.amount.clone(), home.page.clone());
    let Some(Created::Field(paid)) = home
        .edit(Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: "Paid".into(),
            kind: FieldKind::Checkbox,
        })
        .unwrap()
        .created
    else {
        panic!("a field")
    };
    let first = home
        .workspace
        .rows(&table, &query(Vec::new()))
        .unwrap()
        .rows[0]
        .id
        .clone();
    home.edit(Mutation::SetCell {
        block: table.clone(),
        row: first,
        field: paid.clone(),
        value: Cell::Checkbox(true),
    })
    .unwrap();
    let Some(Created::Block { block: chart, .. }) = home
        .edit(Mutation::AddChartBlock {
            page: page.clone(),
            after: None,
        })
        .unwrap()
        .created
    else {
        panic!("a Chart")
    };
    let empty = home.workspace.chart(&page, &chart).unwrap().unwrap();
    assert_eq!((empty.setup.source, empty.points.len()), (None, 0));

    let set = |home: &mut Home, setup: ChartSetup| {
        home.edit(Mutation::SetChart {
            page: page.clone(),
            block: chart.clone(),
            setup,
        })
    };
    let by_paid = ChartSetup {
        kind: ChartKind::Bar,
        source: Some(ChartSource::Table(table.clone())),
        group: Some(paid.clone()),
        value: ChartValue::Sum(amount.clone()),
    };
    set(&mut home, by_paid.clone()).unwrap();
    let points = |home: &mut Home| {
        let drawn = home.workspace.chart(&page, &chart).unwrap().unwrap();
        (
            drawn.source_name,
            drawn
                .points
                .into_iter()
                .map(|point| (point.label, point.value))
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        points(&mut home),
        (
            Some("Expenses".to_string()),
            vec![("No".to_string(), 384.5), ("Yes".to_string(), 900.0)]
        )
    );
    set(
        &mut home,
        ChartSetup {
            value: ChartValue::Count,
            group: None,
            ..by_paid.clone()
        },
    )
    .unwrap();
    assert_eq!(points(&mut home).1, [("Expenses".to_string(), 2.0)]);

    let report = home.calculation(&page);
    let lines = home
        .workspace
        .calculation(&page, &report)
        .unwrap()
        .unwrap()
        .lines;
    home.edit(Mutation::SetLine {
        page: page.clone(),
        block: report.clone(),
        line: lines[0].id.clone(),
        setup: line("Rent", "900", LineStyle::Plain),
    })
    .unwrap();
    let mut last = lines[0].id.clone();
    for setup in [
        line("Food", "384.5", LineStyle::Plain),
        line("Total", "SUM(ABOVE)", LineStyle::Total),
    ] {
        last = home.add(&page, &report, Some(&last), setup).unwrap();
    }
    set(
        &mut home,
        ChartSetup {
            kind: ChartKind::Pie,
            source: Some(ChartSource::Calculation {
                page: page.clone(),
                block: report.clone(),
            }),
            group: None,
            value: ChartValue::Count,
        },
    )
    .unwrap();
    assert_eq!(
        points(&mut home).1,
        [("Rent".to_string(), 900.0), ("Food".to_string(), 384.5)],
        "a total is not drawn"
    );

    for refused in [
        ChartSetup {
            group: Some(amount.clone()),
            ..by_paid.clone()
        },
        ChartSetup {
            value: ChartValue::Sum(paid.clone()),
            ..by_paid.clone()
        },
        ChartSetup {
            source: Some(ChartSource::Table(report.clone())),
            ..by_paid
        },
    ] {
        assert!(set(&mut home, refused).is_err());
    }
}

#[test]
fn a_formula_field_that_reads_its_own_column_stops_at_once() {
    let mut home = Home::new();
    let table = home.table.clone();
    for _ in 0..4_000 {
        home.row();
    }
    let Some(Created::Field(share)) = home
        .edit(Mutation::AddField {
            block: table.clone(),
            place: Place::Last,
            name: "Share".into(),
            kind: FieldKind::Formula,
        })
        .unwrap()
        .created
    else {
        panic!("a field")
    };
    home.edit(Mutation::SetFieldFormula {
        block: table.clone(),
        field: share.clone(),
        formula: format!("1 / SUM({{{table}.{share}}})"),
    })
    .unwrap();
    let rows = home
        .workspace
        .rows(&table, &query(Vec::new()))
        .unwrap()
        .rows;
    assert_eq!(rows[0].computed[0].1.to_string(), "#REF!");
}

#[test]
fn a_computed_value_names_every_block_and_table_it_read() {
    use liquid_core::{ChartSetup, ChartSource, ChartValue, Used};
    let mut home = Home::new();
    let (table, amount, page, other) = (
        home.table.clone(),
        home.amount.clone(),
        home.page.clone(),
        home.other.clone(),
    );
    let source = home.calculation(&page);
    let first = home
        .workspace
        .calculation(&page, &source)
        .unwrap()
        .unwrap()
        .lines[0]
        .id
        .clone();
    home.edit(Mutation::SetLine {
        page: page.clone(),
        block: source.clone(),
        line: first.clone(),
        setup: line(
            "Spent",
            &format!("SUM({{{table}.{amount}}})"),
            LineStyle::Plain,
        ),
    })
    .unwrap();
    let reader = home.calculation(&other);
    let reading = home
        .workspace
        .calculation(&other, &reader)
        .unwrap()
        .unwrap()
        .lines[0]
        .id
        .clone();
    home.edit(Mutation::SetLine {
        page: other.clone(),
        block: reader.clone(),
        line: reading,
        setup: line(
            "Twice",
            &format!("{{{page}.{source}.{first}}} * 2"),
            LineStyle::Plain,
        ),
    })
    .unwrap();
    let uses = home
        .workspace
        .calculation(&other, &reader)
        .unwrap()
        .unwrap()
        .uses;
    assert_eq!(
        uses,
        [
            Used::Table(table.clone()),
            Used::Block(page.clone(), source.clone())
        ]
    );

    let Some(Created::Block { block: chart, .. }) = home
        .edit(Mutation::AddChartBlock {
            page: other.clone(),
            after: None,
        })
        .unwrap()
        .created
    else {
        panic!("a Chart")
    };
    home.edit(Mutation::SetChart {
        page: other.clone(),
        block: chart.clone(),
        setup: ChartSetup {
            source: Some(ChartSource::Calculation {
                page: other.clone(),
                block: reader.clone(),
            }),
            value: ChartValue::Count,
            ..ChartSetup::default()
        },
    })
    .unwrap();
    let uses = home.workspace.chart(&other, &chart).unwrap().unwrap().uses;
    let mut expected = vec![
        Used::Table(table),
        Used::Block(page, source),
        Used::Block(other, reader),
    ];
    expected.sort();
    assert_eq!(uses, expected);
}

#[test]
fn a_chain_of_references_deeper_than_the_bound_stops_with_an_error() {
    let mut home = Home::new();
    let page = home.page.clone();
    let report = home.calculation(&page);
    let mut ids = vec![
        home.workspace
            .calculation(&page, &report)
            .unwrap()
            .unwrap()
            .lines[0]
            .id
            .clone(),
    ];
    for _ in 0..liquid_core::MAX_DEPTH + 40 {
        let after = ids.last().cloned();
        ids.push(
            home.add(
                &page,
                &report,
                after.as_ref(),
                line("", "1", LineStyle::Plain),
            )
            .unwrap(),
        );
    }
    for pair in ids.windows(2) {
        home.edit(Mutation::SetLine {
            page: page.clone(),
            block: report.clone(),
            line: pair[0].clone(),
            setup: line(
                "",
                &format!("{{{page}.{report}.{}}} + 1", pair[1]),
                LineStyle::Plain,
            ),
        })
        .unwrap();
    }
    let lines = home.lines(&page, &report);
    assert_eq!(lines[0].1, "#REF!", "too deep to follow");
    assert_eq!(lines.last().unwrap().1, "1");
}
