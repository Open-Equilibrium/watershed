#![allow(clippy::unwrap_used)]

mod common;

use common::*;
use liquid_core::{
    Applied, BlockId, Cell, ChartKind, ChartSetup, ChartSource, ChartValue, Created, Error,
    FieldId, FieldKind, FieldTotal, Filter, LineId, LineSetup, LineStyle, MAX_CHART_POINTS,
    MAX_DEPTH, MAX_FORMULA, MAX_LINES, MAX_NESTING, Match, Mutation, PageId, RowId, RowQuery, Sort,
    Total, Used,
};

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

fn query(totals: Vec<FieldTotal>) -> RowQuery {
    RowQuery {
        sort: None,
        filter: None,
        within: None,
        start: 0,
        limit: 10,
        search: None,
        totals,
    }
}

fn shown(lines: &[(&str, &str)]) -> Vec<(String, String)> {
    lines
        .iter()
        .map(|(label, value)| (label.to_string(), value.to_string()))
        .collect()
}

impl Home {
    fn new() -> Self {
        let (mut workspace, area, first) = private_page("Household budget");
        let other = page(&mut workspace, &area, "Review");
        let (table, _) = table(&mut workspace, &first, "Expenses");
        let amount = field(&mut workspace, &table, "Amount", FieldKind::Number);
        let mut home = Self {
            workspace,
            page: first,
            other,
            table,
            amount,
        };
        let amount = home.amount.clone();
        for value in [900.0, 384.5] {
            let row = home.row();
            home.set_cell(&row, &amount, Cell::Number(value));
        }
        home
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn row(&mut self) -> RowId {
        row(&mut self.workspace, &self.table)
    }

    fn rows(&mut self) -> Vec<liquid_core::Row> {
        let table = self.table.clone();
        self.workspace
            .rows(&table, &query(Vec::new()))
            .unwrap()
            .rows
    }

    fn first_row(&mut self) -> RowId {
        self.rows()[0].id.clone()
    }

    fn set_cell(&mut self, row: &RowId, field: &FieldId, value: Cell) {
        set_cell(&mut self.workspace, &self.table, row, field, value);
    }

    fn field(&mut self, name: &str, kind: FieldKind) -> FieldId {
        field(&mut self.workspace, &self.table, name, kind)
    }

    fn set_formula(&mut self, field: &FieldId, formula: String) -> Result<Applied, Error> {
        self.edit(Mutation::SetFieldFormula {
            block: self.table.clone(),
            field: field.clone(),
            formula,
        })
    }

    fn computed(&mut self) -> Vec<Vec<(FieldId, String)>> {
        self.rows()
            .iter()
            .map(|row| {
                row.computed
                    .iter()
                    .map(|(field, value)| (field.clone(), value.to_string()))
                    .collect()
            })
            .collect()
    }

    fn block(&mut self, mutation: Mutation) -> BlockId {
        let Created::Block { block, .. } = created(&mut self.workspace, mutation) else {
            panic!("a Block")
        };
        block
    }

    fn calculation(&mut self, page: &PageId) -> BlockId {
        self.block(Mutation::AddCalculationBlock {
            page: page.clone(),
            after: None,
        })
    }

    fn chart(&mut self, page: &PageId) -> BlockId {
        self.block(Mutation::AddChartBlock {
            page: page.clone(),
            after: None,
        })
    }

    fn set_chart(
        &mut self,
        page: &PageId,
        chart: &BlockId,
        setup: ChartSetup,
    ) -> Result<Applied, Error> {
        self.edit(Mutation::SetChart {
            page: page.clone(),
            block: chart.clone(),
            setup,
        })
    }

    fn table_chart(&mut self, page: &PageId, setup: ChartSetup) -> BlockId {
        let chart = self.chart(page);
        self.set_chart(page, &chart, setup).unwrap();
        chart
    }

    fn points(&mut self, page: &PageId, chart: &BlockId) -> (Option<String>, Vec<(String, f64)>) {
        let drawn = self.workspace.chart(page, chart).unwrap().unwrap();
        (
            drawn.source_name,
            drawn
                .points
                .into_iter()
                .map(|point| (point.label, point.value))
                .collect(),
        )
    }

    fn table_view(&mut self, page: &PageId) -> liquid_core::View {
        let table = self.table.clone();
        self.workspace
            .views(page, 0, 10)
            .unwrap()
            .into_iter()
            .find(|view| view.block == table)
            .unwrap()
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

    fn calc(&mut self, page: &PageId, block: &BlockId) -> liquid_core::Calculation {
        self.workspace.calculation(page, block).unwrap().unwrap()
    }

    fn first_line(&mut self, page: &PageId, block: &BlockId) -> LineId {
        self.calc(page, block).lines[0].id.clone()
    }

    fn set_line(&mut self, page: &PageId, block: &BlockId, line: &LineId, setup: LineSetup) {
        self.edit(Mutation::SetLine {
            page: page.clone(),
            block: block.clone(),
            line: line.clone(),
            setup,
        })
        .unwrap();
    }

    fn lines(&mut self, page: &PageId, block: &BlockId) -> Vec<(String, String)> {
        self.calc(page, block)
            .lines
            .iter()
            .map(|line| (line.label.clone(), line.value.to_string()))
            .collect()
    }

    fn chain(&mut self, page: &PageId, report: &BlockId, extra: usize) -> Vec<LineId> {
        let mut ids = vec![self.first_line(page, report)];
        for _ in 0..extra {
            let after = ids.last().cloned();
            ids.push(
                self.add(
                    page,
                    report,
                    after.as_ref(),
                    line("", "1", LineStyle::Plain),
                )
                .unwrap(),
            );
        }
        ids
    }
}

#[test]
fn a_calculation_block_computes_a_report_in_excel_s_layout() {
    let mut home = Home::new();
    let (page, other, table, amount) = (
        home.page.clone(),
        home.other.clone(),
        home.table.clone(),
        home.amount.clone(),
    );
    let report = home.calculation(&page);
    assert_eq!(
        home.lines(&page, &report).len(),
        1,
        "a new Block starts with one line"
    );
    let first = home.first_line(&page, &report);
    let column = format!("{{{table}.{amount}}}");
    home.set_line(
        &page,
        &report,
        &first,
        line("Income", "", LineStyle::Heading),
    );
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
    let named = home.calc(&page, &report);
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
    home.set_cell(&row, &amount, Cell::Number(15.5));
    home.set_line(
        &page,
        &report,
        &ids[2],
        line("Earned", "SUM(ABOVE)", LineStyle::Subtotal),
    );
    let elsewhere = home.calculation(&other);
    let copied = home.first_line(&other, &elsewhere);
    home.set_line(
        &other,
        &elsewhere,
        &copied,
        line(
            "Left, twice",
            &format!("{} * 2", reference(&left)),
            LineStyle::Plain,
        ),
    );
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
    home.set_line(
        &page,
        &report,
        &ids[1],
        line("Side project", &reference(&left), LineStyle::Plain),
    );
    let lines = home.lines(&page, &report);
    assert_eq!(lines.len(), 8);
    assert_eq!(
        lines[2].1, "#REF!",
        "a line that refers to itself, through others"
    );
    assert_eq!(lines[7].1, "#REF!", "a removed line");
    let named = home.calc(&page, &report);
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
            line("", &"1".repeat(MAX_FORMULA + 1), LineStyle::Plain),
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
    let with_tip = home.field("With tip", FieldKind::Formula);
    let share = home.field("Share", FieldKind::Formula);
    home.set_formula(&with_tip, format!("{{{amount}}} * 1.1"))
        .unwrap();
    home.set_formula(&share, format!("{{{with_tip}}} / 2"))
        .unwrap();
    assert_eq!(
        home.computed(),
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
    let first = home.first_line(&page, &report);
    home.set_line(
        &page,
        &report,
        &first,
        line(
            "Shares",
            &format!("SUM({{{table}.{share}}})"),
            LineStyle::Plain,
        ),
    );
    assert_eq!(home.lines(&page, &report), shown(&[("Shares", "706.475")]));

    home.set_formula(&with_tip, format!("{{{share}}} + 1"))
        .unwrap();
    assert_eq!(
        home.computed()[0][0].1,
        "#REF!",
        "two fields that need each other"
    );

    let view = home.table_view(&page);
    let by_share = home.edit(Mutation::SetTabSort {
        page: page.clone(),
        view: view.id.clone(),
        tab: view.tabs[0].id.clone(),
        sort: Some(Sort {
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
        (home.set_formula(&amount, "1".into()), Error::WrongKind),
        (
            home.set_formula(&share, "SUM(".into()),
            Error::Formula { at: 4 },
        ),
    ] {
        assert_eq!(refused.map(|_| ()), Err(error));
    }
}

#[test]
fn a_formula_field_that_reads_its_own_column_stops_at_once() {
    let mut home = Home::new();
    let table = home.table.clone();
    for _ in 0..4_000 {
        home.row();
    }
    let share = home.field("Share", FieldKind::Formula);
    home.set_formula(&share, format!("1 / SUM({{{table}.{share}}})"))
        .unwrap();
    assert_eq!(home.computed()[0][0].1, "#REF!");
}

#[test]
fn a_tab_s_total_row_counts_the_rows_it_shows() {
    let mut home = Home::new();
    let (table, amount, page) = (home.table.clone(), home.amount.clone(), home.page.clone());
    home.row();
    let paid = home.field("Paid", FieldKind::Checkbox);
    let first = home.first_row();
    home.set_cell(&first, &paid, Cell::Checkbox(true));
    let view = home.table_view(&page);
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
    let shown_totals = |home: &mut Home, filter: Option<Filter>| {
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
        shown_totals(&mut home, None),
        ["1284.5", "642.25", "3", "384.5", "900", "1", "2"]
    );
    let unpaid = Filter {
        field: paid.clone(),
        matches: Match::Is(Cell::Checkbox(false)),
    };
    assert_eq!(
        shown_totals(&mut home, Some(unpaid)),
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

#[test]
fn a_chart_draws_a_table_s_groups_or_a_calculation_s_lines() {
    let mut home = Home::new();
    let (table, amount, page) = (home.table.clone(), home.amount.clone(), home.page.clone());
    let paid = home.field("Paid", FieldKind::Checkbox);
    let first = home.first_row();
    home.set_cell(&first, &paid, Cell::Checkbox(true));
    let chart = home.chart(&page);
    let empty = home.workspace.chart(&page, &chart).unwrap().unwrap();
    assert_eq!((empty.setup.source, empty.points.len()), (None, 0));

    let by_paid = ChartSetup {
        kind: ChartKind::Bar,
        source: Some(ChartSource::Table(table.clone())),
        group: Some(paid.clone()),
        value: ChartValue::Sum(amount.clone()),
    };
    home.set_chart(&page, &chart, by_paid.clone()).unwrap();
    assert_eq!(
        home.points(&page, &chart),
        (
            Some("Expenses".to_string()),
            vec![("No".to_string(), 384.5), ("Yes".to_string(), 900.0)]
        )
    );
    home.set_chart(
        &page,
        &chart,
        ChartSetup {
            value: ChartValue::Count,
            group: None,
            ..by_paid.clone()
        },
    )
    .unwrap();
    assert_eq!(
        home.points(&page, &chart).1,
        [("Expenses".to_string(), 2.0)]
    );

    let report = home.calculation(&page);
    let lines = home.calc(&page, &report).lines;
    home.set_line(
        &page,
        &report,
        &lines[0].id,
        line("Rent", "900", LineStyle::Plain),
    );
    let mut last = lines[0].id.clone();
    for setup in [
        line("Food", "384.5", LineStyle::Plain),
        line("Total", "SUM(ABOVE)", LineStyle::Total),
    ] {
        last = home.add(&page, &report, Some(&last), setup).unwrap();
    }
    home.set_chart(
        &page,
        &chart,
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
        home.points(&page, &chart).1,
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
        assert!(home.set_chart(&page, &chart, refused).is_err());
    }
}

#[test]
fn a_computed_value_names_every_block_and_table_it_read() {
    let mut home = Home::new();
    let (table, amount, page, other) = (
        home.table.clone(),
        home.amount.clone(),
        home.page.clone(),
        home.other.clone(),
    );
    let source = home.calculation(&page);
    let first = home.first_line(&page, &source);
    home.set_line(
        &page,
        &source,
        &first,
        line(
            "Spent",
            &format!("SUM({{{table}.{amount}}})"),
            LineStyle::Plain,
        ),
    );
    let reader = home.calculation(&other);
    let reading = home.first_line(&other, &reader);
    home.set_line(
        &other,
        &reader,
        &reading,
        line(
            "Twice",
            &format!("{{{page}.{source}.{first}}} * 2"),
            LineStyle::Plain,
        ),
    );
    assert_eq!(
        home.calc(&other, &reader).uses,
        [
            Used::Table(table.clone()),
            Used::Block(page.clone(), source.clone())
        ]
    );

    let chart = home.chart(&other);
    home.set_chart(
        &other,
        &chart,
        ChartSetup {
            source: Some(ChartSource::Calculation {
                page: other.clone(),
                block: reader.clone(),
            }),
            value: ChartValue::Count,
            ..ChartSetup::default()
        },
    )
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
fn chains_that_read_too_deep_or_too_nested_stop_with_an_error() {
    type Formula = fn(&PageId, &BlockId, &[LineId], usize) -> Option<String>;
    let reads_next: Formula = |page, report, ids, at| {
        (at + 1 < ids.len()).then(|| format!("{{{page}.{report}.{}}} + 1", ids[at + 1]))
    };
    let nested: Formula = |page, report, ids, at| {
        (at + 1 < ids.len()).then(|| {
            let reference = format!("{{{page}.{report}.{}}}", ids[at + 1]);
            let depth = MAX_NESTING - 1;
            format!("{}{reference}{}", "SUM(".repeat(depth), ")".repeat(depth))
        })
    };
    let reads_next_two: Formula = |page, report, ids, at| {
        (at + 2 < ids.len()).then(|| {
            format!(
                "{{{page}.{report}.{}}} + {{{page}.{report}.{}}}",
                ids[at + 1],
                ids[at + 2]
            )
        })
    };
    for (extra, formula, back, value) in [
        (MAX_DEPTH + 40, reads_next, 1, "1"),
        (MAX_DEPTH, nested, 2, "1"),
        (MAX_DEPTH, reads_next_two, 3, "2"),
    ] {
        let mut home = Home::new();
        let page = home.page.clone();
        let report = home.calculation(&page);
        let ids = home.chain(&page, &report, extra);
        for at in 0..ids.len() {
            if let Some(formula) = formula(&page, &report, &ids, at) {
                home.set_line(
                    &page,
                    &report,
                    &ids[at],
                    line("", &formula, LineStyle::Plain),
                );
            }
        }
        let lines = home.lines(&page, &report);
        assert_eq!(lines[0].1, "#REF!", "too deep or nested to follow");
        assert_eq!(lines[lines.len() - back].1, value);
    }
}

#[test]
fn lines_that_need_each_other_are_errors_whatever_order_reads_them() {
    let mut home = Home::new();
    let page = home.page.clone();
    for swapped in [false, true] {
        let report = home.calculation(&page);
        let first = home.first_line(&page, &report);
        let second = home
            .add(&page, &report, Some(&first), line("", "", LineStyle::Plain))
            .unwrap();
        let (guarded, plain) = if swapped {
            (&second, &first)
        } else {
            (&first, &second)
        };
        let at = |line: &LineId| format!("{{{page}.{report}.{line}}}");
        for (line_id, formula) in [
            (guarded, format!("IFERROR({}, 0)", at(plain))),
            (plain, format!("{} + 1", at(guarded))),
        ] {
            home.set_line(
                &page,
                &report,
                line_id,
                line("", &formula, LineStyle::Plain),
            );
        }
        let values: Vec<_> = home
            .lines(&page, &report)
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        assert_eq!(values, ["#REF!", "#REF!"], "swapped: {swapped}");
    }
}

#[test]
fn a_chart_of_a_select_with_many_options_draws_the_ones_in_use() {
    let mut home = Home::new();
    let (table, page) = (home.table.clone(), home.page.clone());
    let kind = home.field("Kind", FieldKind::Select);
    let mut last = None;
    for at in 0..MAX_CHART_POINTS + 20 {
        last = Some(option(
            &mut home.workspace,
            &table,
            &kind,
            &format!("Kind {at}"),
        ));
    }
    let row = home.first_row();
    home.set_cell(&row, &kind, Cell::Select(last.unwrap()));
    let chart = home.table_chart(
        &page,
        ChartSetup {
            source: Some(ChartSource::Table(table)),
            group: Some(kind),
            value: ChartValue::Count,
            ..ChartSetup::default()
        },
    );
    assert_eq!(
        home.points(&page, &chart).1,
        [
            (format!("Kind {}", MAX_CHART_POINTS + 19), 1.0),
            ("Empty".to_string(), 1.0)
        ]
    );
}

#[test]
fn a_duplicated_table_keeps_its_formulas_and_overflow_is_an_error() {
    let mut home = Home::new();
    let (table, amount, page) = (home.table.clone(), home.amount.clone(), home.page.clone());
    let double = home.field("Double", FieldKind::Formula);
    home.set_formula(
        &double,
        format!("{{{amount}}} * 2 + SUM({{{table}.{amount}}})"),
    )
    .unwrap();
    let Created::Block { block: copy, .. } = created(
        &mut home.workspace,
        Mutation::DuplicateTable {
            page: page.clone(),
            after: None,
            table: table.clone(),
            name: "Expenses copy".into(),
            tabs: Vec::new(),
        },
    ) else {
        panic!("a copy")
    };
    let copied = home.workspace.database(&copy).unwrap().unwrap();
    let field = copied
        .fields
        .iter()
        .find(|field| field.id == double)
        .unwrap();
    assert_eq!(
        field.formula,
        format!("{{{amount}}} * 2 + SUM({{{copy}.{amount}}})"),
        "the copy reads its own column"
    );

    for row in home.rows() {
        home.edit(Mutation::SetCell {
            block: table.clone(),
            row: row.id,
            field: amount.clone(),
            value: Cell::Number(1e308),
        })
        .unwrap();
    }
    let sum = vec![FieldTotal {
        field: amount.clone(),
        total: Total::Sum,
    }];
    let totals = home.workspace.rows(&table, &query(sum)).unwrap().totals;
    assert_eq!(totals[0].1.to_string(), "#NUM!");
    let chart = home.table_chart(
        &page,
        ChartSetup {
            source: Some(ChartSource::Table(table)),
            value: ChartValue::Sum(amount),
            ..ChartSetup::default()
        },
    );
    let points = home.workspace.chart(&page, &chart).unwrap().unwrap().points;
    assert!(points.iter().all(|point| point.value.is_finite()));
}

#[test]
fn a_chart_keeps_select_options_with_the_same_name_apart() {
    let mut home = Home::new();
    let (table, page) = (home.table.clone(), home.page.clone());
    let kind = home.field("Kind", FieldKind::Select);
    let options: Vec<_> = (0..2)
        .map(|_| option(&mut home.workspace, &table, &kind, "Other"))
        .collect();
    let rows = home.rows();
    for (row, option) in rows.iter().zip(options) {
        home.set_cell(&row.id, &kind, Cell::Select(option));
    }
    let chart = home.table_chart(
        &page,
        ChartSetup {
            source: Some(ChartSource::Table(table)),
            group: Some(kind),
            value: ChartValue::Count,
            ..ChartSetup::default()
        },
    );
    assert_eq!(
        home.points(&page, &chart).1,
        [("Other".to_string(), 1.0), ("Other".to_string(), 1.0)]
    );
}

#[test]
fn a_total_and_a_chart_average_keep_the_mean_of_huge_values() {
    let mut home = Home::new();
    let (table, amount, page) = (home.table.clone(), home.amount.clone(), home.page.clone());
    for row in home.rows() {
        home.set_cell(&row.id, &amount, Cell::Number(1e308));
    }
    let average = FieldTotal {
        field: amount.clone(),
        total: Total::Average,
    };
    let totals = home
        .workspace
        .rows(&table, &query(vec![average]))
        .unwrap()
        .totals;
    assert_eq!(totals[0].1.to_string(), "1E+308");
    let chart = home.table_chart(
        &page,
        ChartSetup {
            source: Some(ChartSource::Table(table)),
            value: ChartValue::Average(amount),
            ..ChartSetup::default()
        },
    );
    assert_eq!(
        home.points(&page, &chart).1,
        [("Expenses".to_string(), 1e308)]
    );
}

#[test]
fn a_calculation_uses_the_table_named_in_an_inactive_branch() {
    let mut home = Home::new();
    let (table, amount, page) = (home.table.clone(), home.amount.clone(), home.page.clone());
    let report = home.calculation(&page);
    let first = home.first_line(&page, &report);
    home.set_line(
        &page,
        &report,
        &first,
        line(
            "Guard",
            &format!("IF(FALSE, SUM({{{table}.{amount}}}), 0)"),
            LineStyle::Plain,
        ),
    );
    assert_eq!(home.calc(&page, &report).uses, [Used::Table(table)]);
}
