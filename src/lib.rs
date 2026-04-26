//! Data Flow Diagramming Tool — library root.
//!
//! Exposes all modules publicly so that integration tests in `tests/` can
//! import from the `dataflow` crate.  The binary entry point is in `main.rs`.

pub mod app;
pub mod model;
pub mod persistence;
pub mod ui;
pub mod undo;
pub mod validation;
