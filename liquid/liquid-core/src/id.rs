//! Stable identities of Workspace objects: 128 random bits as 32 lowercase hex digits, so
//! devices create them offline without coordinating.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Error;

pub(crate) fn random_bytes() -> Result<[u8; 16], Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::Random)?;
    Ok(bytes)
}

macro_rules! ids {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub(crate) fn new() -> Result<Self, Error> {
                let hex = random_bytes()?.iter().map(|byte| format!("{byte:02x}")).collect();
                Ok(Self(hex))
            }

            /// An identity read back from a document key.
            #[allow(dead_code, reason = "Action identities are read back from headers")]
            pub(crate) fn stored(key: &str) -> Self {
                Self(key.to_string())
            }

            /// The identity `text` names, if it has the stored form.
            pub fn parse(text: &str) -> Option<Self> {
                let stored = text.len() == 32
                    && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
                stored.then(|| Self(text.to_string()))
            }

            /// The identity as documents store it.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    )*};
}

ids! {
    /// An area: a part of the Workspace with its own Page tree.
    AreaId,
    /// A Page.
    PageId,
    /// A Block, the canonical content its Views show.
    BlockId,
    /// One View of a Block on a Page.
    ViewId,
    /// One recorded Action.
    ActionId,
}

/// A person, as the account that signs in names them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PersonId(String);

impl PersonId {
    /// The person the account names `id`.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
