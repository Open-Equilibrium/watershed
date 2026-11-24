use super::workspace::{lisbon, mutation, open, query};
use crate::api::chart::{BridgeChartKind, BridgeChartSetup, BridgeChartValue};
use crate::api::source::{BridgeFileView, BridgeRead, BridgeSourceKind};
use crate::api::workspace::{
    BridgeFound, BridgeMutation, BridgeMutationKind, BridgeQuery, BridgeQueryKind,
};

#[test]
fn an_image_source_crosses_the_bridge_with_the_bytes_read_for_it() {
    let (folder, workspace) = open("source-image");
    let (area, ..) = lisbon(&workspace);
    let files = folder.0.join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("receipt.png"), b"png bytes").unwrap();
    std::fs::write(files.join("notes.md"), "# Notes").unwrap();
    let dir = workspace
        .add_source(area, None, files.to_str().unwrap().into())
        .unwrap();
    let image = workspace
        .source_file(dir.clone(), "receipt.png".into())
        .unwrap();
    assert_eq!(
        (image.shown, image.bytes.as_deref()),
        (BridgeFileView::Image, Some(&b"png bytes"[..]))
    );
    let notes = workspace.source_file(dir, "notes.md".into()).unwrap();
    assert_eq!(notes.bytes, None);
}

#[test]
fn a_source_crosses_the_bridge_and_feeds_a_chart() {
    let (folder, workspace) = open("source");
    let (area, page, _, view) = lisbon(&workspace);
    let files = folder.0.join("files");
    std::fs::create_dir_all(files.join("drafts")).unwrap();
    let csv = files.join("bank-export.csv");
    std::fs::write(&csv, "Date,Amount\n2026-11-01,-84.20\n2026-10-24,3200\n").unwrap();
    std::fs::write(files.join("notes.md"), "# Notes").unwrap();

    let file = workspace
        .add_source(area.clone(), None, csv.to_str().unwrap().into())
        .unwrap();
    let dir = workspace
        .add_source(
            area.clone(),
            Some(page.clone()),
            files.to_str().unwrap().into(),
        )
        .unwrap();
    let sources = workspace.sources(area.clone()).unwrap();
    let mut listed: Vec<_> = sources
        .iter()
        .map(|source| (source.name.as_str(), source.kind, source.parent.as_deref()))
        .collect();
    listed.sort_by_key(|(name, ..)| *name);
    assert_eq!(
        listed,
        [
            ("bank-export.csv", BridgeSourceKind::File, None),
            ("files", BridgeSourceKind::Folder, Some(page.as_str()))
        ]
    );
    let hits = workspace
        .query(BridgeQuery {
            search: Some("bank".into()),
            ..query(BridgeQueryKind::Search)
        })
        .unwrap()
        .hits;
    assert_eq!(
        hits.iter()
            .filter(|hit| hit.found == BridgeFound::Source)
            .map(|hit| hit.id.as_str())
            .collect::<Vec<_>>(),
        [file.as_str()]
    );

    let folder_listing = workspace.source_folder(dir.clone(), String::new()).unwrap();
    assert_eq!(folder_listing.read, BridgeRead::Read);
    let names: Vec<_> = folder_listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["drafts", "bank-export.csv", "notes.md"]);
    let notes = workspace
        .source_file(dir.clone(), "notes.md".into())
        .unwrap();
    assert_eq!(
        (notes.shown, notes.text.as_deref()),
        (BridgeFileView::Text, Some("# Notes"))
    );
    assert_eq!(
        workspace
            .source_file(dir.clone(), "../outside".into())
            .unwrap()
            .read,
        BridgeRead::Missing
    );

    let table = workspace.source_file(file.clone(), String::new()).unwrap();
    assert_eq!(table.shown, BridgeFileView::Table);
    let chart = workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            after: Some(view),
            ..mutation(BridgeMutationKind::AddChartBlock)
        })
        .unwrap()
        .block
        .unwrap();
    workspace
        .apply(BridgeMutation {
            page: Some(page.clone()),
            block: Some(chart.clone()),
            chart: Some(BridgeChartSetup {
                kind: BridgeChartKind::Line,
                table: None,
                page: None,
                block: None,
                source: Some(file.clone()),
                group: Some(table.columns[0].id.clone()),
                value: BridgeChartValue::Sum,
                field: Some(table.columns[1].id.clone()),
            }),
            ..mutation(BridgeMutationKind::SetChart)
        })
        .unwrap();
    let drawn = workspace
        .query(BridgeQuery {
            page: Some(page.clone()),
            block: Some(chart.clone()),
            ..query(BridgeQueryKind::Chart)
        })
        .unwrap()
        .chart
        .unwrap();
    assert_eq!(drawn.setup.source.as_deref(), Some(file.as_str()));
    assert_eq!(
        drawn
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.number))
            .collect::<Vec<_>>(),
        [("Date", false), ("Amount", true)]
    );
    assert_eq!(
        drawn
            .points
            .iter()
            .map(|point| (point.label.as_str(), point.value))
            .collect::<Vec<_>>(),
        [("2026-10-24", 3200.0), ("2026-11-01", -84.2)]
    );
    let users = workspace.source_users(file.clone()).unwrap();
    assert_eq!(
        (users[0].page.as_str(), users[0].block.as_str()),
        (page.as_str(), chart.as_str())
    );

    std::fs::rename(&csv, files.join("moved.csv")).unwrap();
    assert_eq!(
        workspace
            .source_file(file.clone(), String::new())
            .unwrap()
            .read,
        BridgeRead::Missing
    );
    workspace
        .locate_source(
            file.clone(),
            files.join("moved.csv").to_str().unwrap().into(),
        )
        .unwrap();
    assert_eq!(
        workspace
            .source_file(file.clone(), String::new())
            .unwrap()
            .read,
        BridgeRead::Read
    );
    let versions = workspace.versions(page.clone()).unwrap();
    let preview = workspace
        .preview(page.clone(), versions[0].after.clone())
        .unwrap();
    assert!(
        preview
            .locate_source(
                file.clone(),
                files.join("moved.csv").to_str().unwrap().into()
            )
            .is_err()
    );
    workspace
        .rename_source(file.clone(), "Bank".into())
        .unwrap();
    workspace.remove_source(file).unwrap();
    let left: Vec<_> = workspace
        .sources(area)
        .unwrap()
        .into_iter()
        .map(|source| source.name)
        .collect();
    assert_eq!(left, ["files"]);
}

#[test]
fn a_source_file_refused_for_its_size_or_kind_still_names_its_path() {
    let (folder, workspace) = open("source-refused");
    let (area, ..) = lisbon(&workspace);
    let files = folder.0.join("files");
    std::fs::create_dir_all(&files).unwrap();
    let text = files.join("notes.txt");
    std::fs::write(&text, [0xff, 0xfe, 0x00]).unwrap();
    let big = files.join("scan.png");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(liquid_core::MAX_SOURCE_FILE + 1)
        .unwrap();
    for (path, read) in [
        (&text, BridgeRead::Unreadable),
        (&big, BridgeRead::TooLarge),
    ] {
        let source = workspace
            .add_source(area.clone(), None, path.to_str().unwrap().into())
            .unwrap();
        let answer = workspace.source_file(source, String::new()).unwrap();
        assert_eq!(
            (answer.read, answer.name.as_str(), answer.path.as_str()),
            (
                read,
                path.file_name().unwrap().to_str().unwrap(),
                path.to_str().unwrap()
            )
        );
    }
}
