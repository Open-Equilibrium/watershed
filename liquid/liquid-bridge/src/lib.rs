//! Bridge between the Liquid Flutter app and its Rust cores: the embedded Meta-Harness core
//! (ADR-0184) and the Workspace (ADR-0216), through flutter_rust_bridge (ADR-0186). Only `api`
//! is visible to Dart; the cores stay free of bridge code.

pub mod api;
mod convert;
mod frb_generated;
mod keys;
mod ready;
mod stream;

#[cfg(test)]
mod tests;
