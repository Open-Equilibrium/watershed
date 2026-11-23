#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use liquid_core::{
    Actor, Applied, AreaId, ChartKind, ChartPoint, ChartSetup, ChartSource, ChartValue, Created,
    EntryQuery, Error, Found, MemoryStore, Mutation, Origin, PageId, PersonId, Request, Shown,
    SourceId, SourceKind,
};

type Workspace = liquid_core::Workspace<MemoryStore>;

fn owner() -> Actor {
    Actor::Person(PersonId::new("benni"))
}

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "liquid-sources-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
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
        let mut workspace =
            Workspace::create(PersonId::new("benni"), MemoryStore::default()).unwrap();
        let mut edit = |mutation| {
            workspace
                .apply(Request {
                    actor: owner(),
                    origin: Origin::Ui,
                    mutation,
                })
                .unwrap()
                .created
        };
        let Some(Created::Area(area)) = edit(Mutation::CreateArea {
            name: "Private".into(),
        }) else {
            panic!("an area")
        };
        let Some(Created::Page(page)) = edit(Mutation::CreatePage {
            area: area.clone(),
            parent: None,
            title: "Household budget".into(),
        }) else {
            panic!("a Page")
        };
        Self {
            workspace,
            area,
            page,
        }
    }

    fn edit(&mut self, mutation: Mutation) -> Result<Applied, Error> {
        self.workspace.apply(Request {
            actor: owner(),
            origin: Origin::Ui,
            mutation,
        })
    }

    fn add(&mut self, path: &Path) -> SourceId {
        let applied = self
            .workspace
            .add_source(&owner(), &self.area, None, path.to_str().unwrap())
            .unwrap();
        match applied.created {
            Some(Created::Source(source)) => source,
            other => panic!("a Source, got {other:?}"),
        }
    }
}

const BANK: &str = "Date,Text,Amount\n\
2026-11-01,REWE SAGT DANKE,-84.20\n\
2026-10-30,STADTWERKE,-96.00\n\
2026-10-24,GEHALT,\"+3,200.00\"\n\
2026-10-30,KINO,-11.80\n";

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

    let file = private.workspace.source_file(&source, "").unwrap();
    let Shown::Table(table) = file.shown else {
        panic!("a table")
    };
    let names: Vec<&str> = table
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect();
    assert_eq!(names, ["Date", "Text", "Amount"]);
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| column.number)
            .collect::<Vec<_>>(),
        [false, false, true]
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

    let markdown = private
        .workspace
        .source_file(&source, "packing-list.md")
        .unwrap();
    assert_eq!(
        markdown.shown,
        Shown::Text("# Packing list\n\n- Passport\n".into())
    );
    assert_eq!(
        private
            .workspace
            .source_file(&source, "schema.sql")
            .unwrap()
            .shown,
        Shown::Other
    );
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
    assert_eq!(
        private.workspace.source_file(&source, "").unwrap().shown,
        Shown::Text("here".into())
    );
}

#[test]
fn a_chart_draws_a_csv_source_and_follows_its_file() {
    let folder = Folder::new("chart");
    let path = folder.write("bank-export.csv", BANK);
    let mut private = Private::new();
    let source = private.add(&path);
    let Shown::Table(table) = private.workspace.source_file(&source, "").unwrap().shown else {
        panic!("a table")
    };
    let (date, amount) = (table.columns[0].id.clone(), table.columns[2].id.clone());
    let page = private.page.clone();
    let Some(Created::Block { block, .. }) = private
        .edit(Mutation::AddChartBlock {
            page: page.clone(),
            after: None,
        })
        .unwrap()
        .created
    else {
        panic!("a Chart")
    };
    private
        .edit(Mutation::SetChart {
            page: page.clone(),
            block: block.clone(),
            setup: ChartSetup {
                kind: ChartKind::Line,
                source: Some(ChartSource::Source(source.clone())),
                group: Some(date),
                value: ChartValue::Sum(amount.clone()),
            },
        })
        .unwrap();
    let chart = private.workspace.chart(&page, &block).unwrap().unwrap();
    assert_eq!(chart.source_name.as_deref(), Some("bank-export.csv"));
    let point = |label: &str, value: f64| ChartPoint {
        label: label.into(),
        value,
    };
    let close = |points: &[ChartPoint], expected: &[ChartPoint]| {
        assert_eq!(points.len(), expected.len(), "{points:?}");
        for (got, want) in points.iter().zip(expected) {
            assert_eq!(got.label, want.label);
            assert!((got.value - want.value).abs() < 1e-9, "{got:?} {want:?}");
        }
    };
    close(
        &chart.points,
        &[
            point("2026-10-24", 3200.0),
            point("2026-10-30", -107.8),
            point("2026-11-01", -84.2),
        ],
    );
    let users = private.workspace.source_users(&source).unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!((&users[0].page, &users[0].block), (&page, &block));

    std::fs::write(&path, "Date,Text,Amount\n2026-11-02,MIETE,-950\n").unwrap();
    let chart = private.workspace.chart(&page, &block).unwrap().unwrap();
    close(&chart.points, &[point("2026-11-02", -950.0)]);

    assert_eq!(
        private
            .edit(Mutation::SetChart {
                page: page.clone(),
                block: block.clone(),
                setup: ChartSetup {
                    kind: ChartKind::Bar,
                    source: Some(ChartSource::Source(source.clone())),
                    group: None,
                    value: ChartValue::Sum(table.columns[1].id.clone()),
                },
            })
            .unwrap_err(),
        Error::WrongKind
    );
    assert!(matches!(
        private.workspace.source_file(&source, "").unwrap().shown,
        Shown::Table(_)
    ));
}
