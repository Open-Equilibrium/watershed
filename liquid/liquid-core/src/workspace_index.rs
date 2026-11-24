use crate::store::Store;
use crate::workspace::{Workspace, Written};
use crate::{
    BlockId, BlockKind, Changed, Created, Error, Found, Hit, INDEX_VERSION, IndexEntry, IndexWrite,
    Mutation, PageId, database, search_words,
};

pub(crate) fn page_entry(page: &PageId, title: &str) -> IndexWrite {
    IndexWrite {
        found: Found::Page,
        id: page.to_string(),
        entry: Some(IndexEntry {
            page: Some(page.clone()),
            text: title.to_string(),
        }),
    }
}

pub(crate) fn table_entry(table: &BlockId, name: Option<&str>) -> IndexWrite {
    IndexWrite {
        found: Found::Table,
        id: table.to_string(),
        entry: name.map(|name| IndexEntry {
            page: None,
            text: name.to_string(),
        }),
    }
}

impl<S: Store> Workspace<S> {
    pub fn search(&mut self, query: &str, found: Found, limit: usize) -> Result<Vec<Hit>, Error> {
        if query.chars().count() > database::MAX_FILTER_TEXT {
            return Err(Error::TooLong);
        }
        let words = search_words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        self.store.search(&words, found, limit)
    }

    pub(crate) fn ensure_index(&mut self) -> Result<(), Error> {
        if self.store.index_version()? >= INDEX_VERSION {
            return Ok(());
        }
        let mut complete = true;
        for area in self.areas()? {
            let sources: Vec<IndexWrite> = self
                .sources(&area.id)?
                .iter()
                .map(|source| crate::source::source_entry(&source.id, Some(&source.name)))
                .collect();
            self.store.index(&sources, None)?;
            for page in self.pages(&area.id)? {
                let mut writes = vec![page_entry(&page.id, &page.title)];
                match self.page_text(&page.id) {
                    Ok(text) => writes.extend(text),
                    Err(_) => complete = false,
                }
                self.close_page(&page.id);
                self.store.index(&writes, None)?;
            }
        }
        let tables: Vec<IndexWrite> = self
            .tables()?
            .iter()
            .map(|table| table_entry(&table.id, Some(&table.name)))
            .collect();
        self.store.index(&tables, complete.then_some(INDEX_VERSION))
    }

    pub(crate) fn page_text(&mut self, page: &PageId) -> Result<Vec<IndexWrite>, Error> {
        let blocks: Vec<BlockId> = self
            .views(page, 0, usize::MAX)?
            .into_iter()
            .filter(|view| {
                matches!(
                    view.kind,
                    BlockKind::Text | BlockKind::Code | BlockKind::Math
                )
            })
            .map(|view| view.block)
            .collect();
        blocks
            .iter()
            .map(|block| self.text_entry(page, block))
            .collect()
    }

    fn text_entry(&mut self, page: &PageId, block: &BlockId) -> Result<IndexWrite, Error> {
        let text = match self.text(page, block)? {
            Some(text) => Some(text),
            None => match self.code(page, block)? {
                Some(code) => Some(code.text),
                None => self.math(page, block)?,
            },
        };
        Ok(IndexWrite {
            found: Found::Text,
            id: block.to_string(),
            entry: text.map(|text| IndexEntry {
                page: Some(page.clone()),
                text,
            }),
        })
    }

    pub(crate) fn leaving(&mut self, mutation: &Mutation) -> Option<(PageId, BlockId)> {
        let (Mutation::RemoveView { page, view } | Mutation::JoinTextBlock { page, view }) =
            mutation
        else {
            return None;
        };
        let block = self.page(page).ok()?.1.view(view)?.block.clone();
        Some((page.clone(), block))
    }

    pub(crate) fn index_writes(
        &mut self,
        mutation: &Mutation,
        written: &Written,
        leaving: Option<(PageId, BlockId)>,
    ) -> Result<Vec<IndexWrite>, Error> {
        let mut writes = Vec::new();
        let mut blocks: Vec<(PageId, BlockId)> = leaving.into_iter().collect();
        let mut tables: Vec<BlockId> = Vec::new();
        match (mutation, &written.created) {
            (Mutation::CreatePage { title, .. }, Some(Created::Page(page))) => {
                writes.push(page_entry(page, title));
            }
            (Mutation::RenamePage { page, title }, _) => writes.push(page_entry(page, title)),
            (Mutation::AddSource { name, .. }, Some(Created::Source(source)))
            | (Mutation::RenameSource { source, name }, _) => {
                writes.push(crate::source::source_entry(source, Some(name)));
            }
            (Mutation::RemoveSource { source }, _) => {
                writes.push(crate::source::source_entry(source, None));
            }
            (
                Mutation::AddTextBlock { page, .. } | Mutation::SplitTextBlock { page, .. },
                Some(Created::Block { block, .. }),
            ) => blocks.push((page.clone(), block.clone())),
            (Mutation::CreateTable { .. }, Some(Created::Table(table)))
            | (
                Mutation::AddDatabaseBlock { .. } | Mutation::DuplicateTable { .. },
                Some(Created::Block { block: table, .. }),
            ) => tables.push(table.clone()),
            _ => {}
        }
        if let Mutation::SplitTextBlock { page, block, .. } = mutation {
            blocks.push((page.clone(), block.clone()));
        }
        for changed in &written.changed {
            match changed {
                Changed::Block(page, block) => blocks.push((page.clone(), block.clone())),
                Changed::Table(table) => tables.push(table.clone()),
                _ => {}
            }
        }
        blocks.sort();
        blocks.dedup();
        tables.sort();
        tables.dedup();
        for (page, block) in blocks {
            writes.push(self.text_entry(&page, &block)?);
        }
        for table in tables {
            let name = self.table_name(&table)?;
            writes.push(table_entry(&table, name.as_deref()));
        }
        Ok(writes)
    }
}
