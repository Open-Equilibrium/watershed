mod canonical;
pub(super) mod error;
pub(super) mod load;
pub(super) mod model;
pub(super) mod naming;
pub(super) mod parser;
pub(super) mod paths;
mod registry;
mod semantics;
pub use semantics::validate_block_identity;
pub(super) mod values;

#[cfg(test)]
#[path = "../tests.rs"]
mod tests;
