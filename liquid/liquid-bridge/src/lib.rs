//! Bridge between the Liquid Flutter app and the embedded Meta-Harness core (ADR-0184,
//! ADR-0186). Only `api` is visible to Dart; the core stays free of bridge code.

pub mod api;
mod convert;
mod frb_generated;
mod ready;

#[cfg(test)]
mod tests;
