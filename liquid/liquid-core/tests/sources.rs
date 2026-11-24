#![allow(clippy::unwrap_used)]

mod common;

use std::path::{Path, PathBuf};

use common::*;
use liquid_core::{
    Applied, AreaId, BlockId, Chart, ChartKind, ChartPoint, ChartSetup, ChartSource, ChartValue,
    Created, EntryQuery, Error, Found, Mutation, PageId, Shown, SourceId, SourceKind, SourceTable,
};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let name = format!("liquid-sources-{name}-{}", std::process::id());
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Private {
    workspace: Workspace,
    area: AreaId,
    page: PageId,
}

impl Private {
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
                area: area.clone(),
                parent: None,
                title: "Household budget".into(),
            },
        ) else {
            panic!("a Page")
        };
        Self {
            workspace,
            area,
            page,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        apply(&mut self.workspace, mutation)
    }

    fn add(&mut self, path: &Path) -> SourceId {
        let applied = self
            .workspace
            .add_source(&owner(), &self.area, None, path.to_str().unwrap())
            .unwrap();
        let Some(Created::Source(source)) = applied.created else {
            panic!("a Source")
        };
        source
    }

    fn file(&mut self, source: &SourceId, path: &str) -> Shown {
        self.workspace.source_file(source, path).unwrap().shown
    }

    fn table(&mut self, source: &SourceId) -> SourceTable {
        let Shown::Table(table) = self.file(source, "") else {
            panic!("a table")
        };
        table
    }

    fn new_chart(&mut self) -> BlockId {
        let added = self.edit(Mutation::AddChartBlock {
            page: self.page.clone(),
            after: None,
        });
        let Some(Created::Block { block, .. }) = added.unwrap().created else {
            panic!("a Chart")
        };
        block
    }

    fn set_chart(&mut self, block: &BlockId, setup: ChartSetup) -> Result<Applied, Error> {
        self.edit(Mutation::SetChart {
            page: self.page.clone(),
            block: block.clone(),
            setup,
        })
    }

    fn chart(&mut self, block: &BlockId) -> Chart {
        self.workspace.chart(&self.page, block).unwrap().unwrap()
    }
}

const BANK: &str = "Date,Text,Amount\n\
2026-11-01,REWE SAGT DANKE,-84.20\n\
2026-10-30,STADTWERKE,-96.00\n\
2026-10-24,GEHALT,\"+3,200.00\"\n\
2026-10-30,KINO,-11.80\n";

fn assert_points(got: &[ChartPoint], want: &[(&str, f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (point, (label, value)) in got.iter().zip(want) {
        assert_eq!(point.label, *label);
        assert!((point.value - value).abs() < 1e-9, "{point:?}");
    }
}

#[test]
fn a_file_source_lists_beside_the_pages_and_keeps_its_path_on_this_device() {
    let folder = Folder::new("file");
    let path = folder.write("bank-export.csv", BANK);
    let mut private = Private::new();
    let source = private.add(&path);

    let listed = private.workspace.sources(&private.area).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (&listed[0].id, listed[0].name.as_str(), listed[0].kind),
        (&source, "bank-export.csv", SourceKind::File)
    );
    assert_eq!(
        private.workspace.source_path(&source).unwrap().as_deref(),
        path.to_str()
    );
    let records = serde_json::to_string(&private.workspace.history(None, 100).unwrap()).unwrap();
    assert!(!records.contains(folder.0.to_str().unwrap()));

    let table = private.table(&source);
    let columns: Vec<_> = table
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column.number))
        .collect();
    assert_eq!(
        columns,
        [("Date", false), ("Text", false), ("Amount", true)]
    );
    assert_eq!(table.rows[2], ["2026-10-24", "GEHALT", "+3,200.00"]);

    let hits = private.workspace.search("bank", Found::Source, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(
        private
            .workspace
            .search("STADTWERKE", Found::Source, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn renaming_and_removing_a_source_are_reverted_from_history() {
    let folder = Folder::new("history");
    let path = folder.write("bank-export.csv", BANK);
    let mut private = Private::new();
    let source = private.add(&path);
    private
        .edit(Mutation::RenameSource {
            source: source.clone(),
            name: "Bank".into(),
        })
        .unwrap();
    let removed = private
        .edit(Mutation::RemoveSource {
            source: source.clone(),
        })
        .unwrap();
    assert!(private.workspace.sources(&private.area).unwrap().is_empty());
    let entries = private
        .workspace
        .entries(&EntryQuery {
            page: None,
            actor: None,
            origin: None,
            before: None,
            limit: 10,
        })
        .unwrap()
        .entries;
    assert_eq!(entries[0].title, "Removed the Source “Bank”");
    private
        .workspace
        .revert(&owner(), &[removed.action])
        .unwrap();
    let listed = private.workspace.sources(&private.area).unwrap();
    assert_eq!(listed[0].name, "Bank");
    assert!(private.workspace.source_file(&source, "").is_ok());
}

#[test]
fn a_folder_source_shows_its_files_and_nothing_outside_it() {
    let folder = Folder::new("folder");
    folder.write("docs/packing-list.md", "# Packing list\n\n- Passport\n");
    folder.write("docs/schema.sql", "CREATE TABLE t (x INT);");
    folder.write("docs/drafts/idea.txt", "later");
    folder.write("docs/.hidden", "x");
    folder.write("secret.txt", "outside");
    let mut private = Private::new();
    let source = private.add(&folder.0.join("docs"));
    assert_eq!(
        private.workspace.sources(&private.area).unwrap()[0].kind,
        SourceKind::Folder
    );

    let entries = private.workspace.source_entries(&source, "").unwrap();
    let names: Vec<(&str, bool)> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.folder))
        .collect();
    assert_eq!(
        names,
        [
            ("drafts", true),
            ("packing-list.md", false),
            ("schema.sql", false)
        ]
    );
    let inner = private.workspace.source_entries(&source, "drafts").unwrap();
    assert_eq!(inner[0].name, "idea.txt");

    assert_eq!(
        private.file(&source, "packing-list.md"),
        Shown::Text("# Packing list\n\n- Passport\n".into())
    );
    assert_eq!(private.file(&source, "schema.sql"), Shown::Other);
    for outside in ["../secret.txt", "drafts/../../secret.txt", "/etc/hosts"] {
        assert_eq!(
            private.workspace.source_file(&source, outside).unwrap_err(),
            Error::NotFound,
            "{outside}"
        );
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(folder.0.join("secret.txt"), folder.0.join("docs/link.txt"))
            .unwrap();
        assert_eq!(
            private
                .workspace
                .source_file(&source, "link.txt")
                .unwrap_err(),
            Error::NotFound
        );
    }
}

#[test]
fn an_image_or_pdf_source_comes_with_the_bytes_read_for_it() {
    let folder = Folder::new("media");
    folder.write("docs/receipt.png", "png bytes");
    folder.write("docs/offer.pdf", "pdf bytes");
    let mut private = Private::new();
    let source = private.add(&folder.0.join("docs"));
    assert_eq!(
        private.file(&source, "receipt.png"),
        Shown::Image(b"png bytes".to_vec())
    );
    assert_eq!(
        private.file(&source, "offer.pdf"),
        Shown::Pdf(b"pdf bytes".to_vec())
    );
    let huge = folder.write("docs/huge.pdf", "");
    std::fs::File::options()
        .write(true)
        .open(&huge)
        .unwrap()
        .set_len(liquid_core::MAX_SOURCE_FILE + 1)
        .unwrap();
    assert_eq!(
        private
            .workspace
            .source_file(&source, "huge.pdf")
            .unwrap_err(),
        Error::TooLarge
    );
}

#[test]
fn a_file_past_50_mib_is_refused_and_a_moved_file_is_located_again() {
    let folder = Folder::new("large");
    let path = folder.write("big.txt", "");
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(liquid_core::MAX_SOURCE_FILE + 1)
        .unwrap();
    let mut private = Private::new();
    let source = private.add(&path);
    assert_eq!(
        private.workspace.source_file(&source, "").unwrap_err(),
        Error::TooLarge
    );

    let moved = folder.write("moved.txt", "here");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        private.workspace.source_file(&source, "").unwrap_err(),
        Error::NotFound
    );
    private
        .workspace
        .locate_source(&owner(), &source, moved.to_str().unwrap())
        .unwrap();
    assert_eq!(private.file(&source, ""), Shown::Text("here".into()));
}

#[test]
fn a_chart_draws_a_csv_source_and_follows_its_file() {
    let folder = Folder::new("chart");
    let path = folder.write("bank-export.csv", BANK);
    let mut private = Private::new();
    let source = private.add(&path);
    let table = private.table(&source);
    let (date, amount) = (table.columns[0].id.clone(), table.columns[2].id.clone());
    let block = private.new_chart();
    private
        .set_chart(
            &block,
            ChartSetup {
                kind: ChartKind::Line,
                source: Some(ChartSource::Source(source.clone())),
                group: Some(date),
                value: ChartValue::Sum(amount),
            },
        )
        .unwrap();
    let chart = private.chart(&block);
    assert_eq!(chart.source_name.as_deref(), Some("bank-export.csv"));
    assert_points(
        &chart.points,
        &[
            ("2026-10-24", 3200.0),
            ("2026-10-30", -107.8),
            ("2026-11-01", -84.2),
        ],
    );
    let users = private.workspace.source_users(&source).unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!((&users[0].page, &users[0].block), (&private.page, &block));

    std::fs::write(&path, "Date,Text,Amount\n2026-11-02,MIETE,-950\n").unwrap();
    assert_points(&private.chart(&block).points, &[("2026-11-02", -950.0)]);

    let bar = ChartSetup {
        kind: ChartKind::Bar,
        source: Some(ChartSource::Source(source.clone())),
        group: None,
        value: ChartValue::Sum(table.columns[1].id.clone()),
    };
    assert_eq!(
        private.set_chart(&block, bar).unwrap_err(),
        Error::WrongKind
    );
    assert!(matches!(private.file(&source, ""), Shown::Table(_)));
}

#[test]
fn a_semicolon_csv_reads_commas_as_decimal_marks() {
    let folder = Folder::new("semicolon");
    let path = folder.write(
        "konto.csv",
        "Datum;Text;Betrag\n2026-11-01;REWE;-84,20\n2026-10-24;GEHALT;3.200,00\n",
    );
    let mut private = Private::new();
    let source = private.add(&path);
    let table = private.table(&source);
    assert!(table.columns[2].number);
    let block = private.new_chart();
    private
        .set_chart(
            &block,
            ChartSetup {
                kind: ChartKind::Bar,
                source: Some(ChartSource::Source(source)),
                group: None,
                value: ChartValue::Sum(table.columns[2].id.clone()),
            },
        )
        .unwrap();
    let chart = private.chart(&block);
    assert!(
        (chart.points[0].value - 3115.8).abs() < 1e-9,
        "{:?}",
        chart.points
    );
}

#[test]
fn a_semicolon_csv_with_commas_inside_a_quoted_header_keeps_its_columns() {
    let folder = Folder::new("quoted-separator");
    let path = folder.write(
        "names.csv",
        "\"Last, First, Middle\";Amount\n\"Smith, John, A\";10\n",
    );
    let mut private = Private::new();
    let source = private.add(&path);
    let table = private.table(&source);
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["Last, First, Middle", "Amount"]
    );
    assert_eq!(table.rows, [["Smith, John, A", "10"]]);
}

#[test]
fn a_chart_picks_the_second_of_two_columns_with_the_same_header() {
    let folder = Folder::new("duplicate-headers");
    let path = folder.write("double.csv", "Amount,Amount\n1,2\n3,4\n");
    let mut private = Private::new();
    let source = private.add(&path);
    let table = private.table(&source);
    assert_ne!(table.columns[0].id, table.columns[1].id);
    let block = private.new_chart();
    private
        .set_chart(
            &block,
            ChartSetup {
                kind: ChartKind::Bar,
                source: Some(ChartSource::Source(source)),
                group: None,
                value: ChartValue::Sum(table.columns[1].id.clone()),
            },
        )
        .unwrap();
    assert_eq!(private.chart(&block).points[0].value, 6.0);
}
