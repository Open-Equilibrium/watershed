use serde::{Deserialize, Serialize};

use crate::PageId;

pub const INDEX_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Found {
    Page,
    Table,
    Text,
}

impl Found {
    pub fn key(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Table => "table",
            Self::Text => "text",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub page: Option<PageId>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexWrite {
    pub found: Found,
    pub id: String,
    pub entry: Option<IndexEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub found: Found,
    pub id: String,
    pub page: Option<PageId>,
    pub text: String,
}

pub fn search_words(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub(crate) fn matches(words: &[String], text: &str) -> bool {
    let held = search_words(text);
    words
        .iter()
        .all(|word| held.iter().any(|held| held.starts_with(word.as_str())))
}
