//! Bridge between the Liquid Flutter app and the embedded Meta-Harness core (ADR-0184,
//! ADR-0186). Only `api` is visible to Dart; the core stays free of bridge code (ADR-0192).

pub mod api;
mod convert;
mod frb_generated;

#[cfg(test)]
mod tests;
