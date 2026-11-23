use super::workspace::{Folder, Memory, lisbon, mutation, query};
use crate::api::calculation::{BridgeLineSetup, BridgeLineStyle, BridgeValueKind};
use crate::api::replica::LiquidWorkspace;
use crate::api::workspace::{
    BridgeBlockKind, BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind,
    BridgeResource, BridgeResourceKind,
};

fn setup(label: &str, formula: &str, style: BridgeLineStyle) -> Option<BridgeLineSetup> {
    Some(BridgeLineSetup {
        label: label.into(),
        formula: formula.into(),
        style,
    })
}

#[test]
fn a_calculation_block_s_lines_cross_and_come_back_computed() {
    let folder = Folder::new("calculation");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, _, intro) = lisbon(&workspace);
    let added = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(intro),
            ..mutation(BridgeMutationKind::AddCalculationBlock)
        })
        .unwrap();
    let block = added.block.unwrap();
    let views = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap()
        .views;
    assert_eq!(views[1].kind, BridgeBlockKind::Calculation);
    let read = || {
        workspace
            .query(BridgeQuery {
                page: Some(page.clone()),
                block: Some(block.clone()),
                ..query(BridgeQueryKind::Calculation)
            })
            .unwrap()
            .lines
    };
    let first = read()[0].id.clone();
    let edit = |kind, line: Option<String>, after: Option<String>, setup| {
        workspace.apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(block.clone()),
            line,
            after,
            line_setup: setup,
            ..mutation(kind)
        })
    };
    edit(
        BridgeMutationKind::SetLine,
        Some(first.clone()),
        None,
        setup("Flights", "420", BridgeLineStyle::Plain),
    )
    .unwrap();
    let reference = format!("{{{page}.{block}.{first}}}");
    let total = edit(
        BridgeMutationKind::AddLine,
        None,
        Some(first.clone()),
        setup(
            "Per person",
            &format!("{reference} / 2"),
            BridgeLineStyle::Total,
        ),
    )
    .unwrap()
    .line
    .unwrap();
    edit(
        BridgeMutationKind::AddLine,
        None,
        Some(total.clone()),
        setup("Note", "\"to pay\"", BridgeLineStyle::Plain),
    )
    .unwrap();
    let lines = read();
    let shown: Vec<_> = lines
        .iter()
        .map(|line| {
            (
                line.label.as_str(),
                line.value.as_str(),
                line.kind,
                line.style,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (
                "Flights",
                "420",
                BridgeValueKind::Number,
                BridgeLineStyle::Plain
            ),
            (
                "Per person",
                "210",
                BridgeValueKind::Number,
                BridgeLineStyle::Total
            ),
            (
                "Note",
                "to pay",
                BridgeValueKind::Text,
                BridgeLineStyle::Plain
            ),
        ]
    );
    assert_eq!(lines[1].formula, format!("{reference} / 2"));
    assert_eq!(lines[1].references, [Some("Flights".to_string())]);

    assert_eq!(
        edit(
            BridgeMutationKind::SetLine,
            Some(total.clone()),
            None,
            setup("Per person", "1 +", BridgeLineStyle::Total),
        ),
        Err("the formula can't be read from character 4".into())
    );
    edit(BridgeMutationKind::RemoveLine, Some(first), None, None).unwrap();
    let lines = read();
    assert_eq!(
        (lines.len(), lines[0].value.as_str(), lines[0].kind),
        (2, "#REF!", BridgeValueKind::Error)
    );
    assert!(
        edit(BridgeMutationKind::SetLine, Some(total), None, None).is_err(),
        "a line needs its label, formula and style"
    );
}

#[test]
fn a_chart_s_setup_crosses_and_its_points_come_back() {
    use crate::api::chart::{BridgeChartKind, BridgeChartSetup, BridgeChartValue};
    let folder = Folder::new("chart");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, _, intro) = lisbon(&workspace);
    let report = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(intro.clone()),
            ..mutation(BridgeMutationKind::AddCalculationBlock)
        })
        .unwrap()
        .block
        .unwrap();
    let first = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            block: Some(report.clone()),
            ..query(BridgeQueryKind::Calculation)
        })
        .unwrap()
        .lines[0]
        .id
        .clone();
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(report.clone()),
            line: Some(first),
            line_setup: setup("Flights", "420", BridgeLineStyle::Plain),
            ..mutation(BridgeMutationKind::SetLine)
        })
        .unwrap();
    let chart = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(intro),
            ..mutation(BridgeMutationKind::AddChartBlock)
        })
        .unwrap()
        .block
        .unwrap();
    let drawn = BridgeChartSetup {
        kind: BridgeChartKind::Pie,
        table: None,
        page: Some(page.clone()),
        block: Some(report),
        group: None,
        value: BridgeChartValue::Count,
        field: None,
    };
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(chart.clone()),
            chart: Some(drawn.clone()),
            ..mutation(BridgeMutationKind::SetChart)
        })
        .unwrap();
    let answer = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            block: Some(chart),
            ..query(BridgeQueryKind::Chart)
        })
        .unwrap();
    assert_eq!(
        answer.follows,
        [BridgeResource {
            kind: BridgeResourceKind::Block,
            area: None,
            page: Some(page.clone()),
            block: drawn.block.clone(),
        }]
    );
    let read = answer.chart.unwrap();
    assert_eq!(read.setup, drawn);
    assert_eq!(
        read.points
            .iter()
            .map(|point| (point.label.as_str(), point.value))
            .collect::<Vec<_>>(),
        [("Flights", 420.0)]
    );
    let views = workspace
        .query(BridgeQuery {
            page: Some(page),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap()
        .views;
    assert!(views.iter().any(|view| view.kind == BridgeBlockKind::Chart));
}

#[test]
fn a_math_block_crosses_as_its_latex() {
    let folder = Folder::new("math");
    let workspace = LiquidWorkspace::open_with(&folder.0, &Memory::default()).unwrap();
    let (_, page, block, _) = lisbon(&workspace);
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(block.clone()),
            ..mutation(BridgeMutationKind::TurnIntoMath)
        })
        .unwrap();
    let read = |kind| {
        workspace
            .query(BridgeQuery {
                page: Some(page.clone()),
                block: Some(block.clone()),
                ..query(kind)
            })
            .unwrap()
            .text
    };
    assert_eq!(read(BridgeQueryKind::Math).as_deref(), Some("Five days"));
    assert_eq!(read(BridgeQueryKind::Text), None);
    let views = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            ..query(BridgeQueryKind::Views)
        })
        .unwrap()
        .views;
    assert_eq!(views[0].kind, BridgeBlockKind::Math);
}
