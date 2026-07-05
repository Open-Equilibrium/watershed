//! Building-block script model contracts for M0.

#![deny(missing_docs)]

include!("script/model.rs");
include!("script/registry.rs");
include!("script/naming.rs");
include!("script/semantics.rs");
include!("script/load.rs");
include!("script/parser.rs");
include!("script/canonical.rs");
include!("script/paths.rs");

#[cfg(test)]
mod tests;
