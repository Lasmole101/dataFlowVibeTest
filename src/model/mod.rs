//! Domain model for the diagramming tool.
//!
//! Modules in this crate layer are pure data — no UI, no I/O.
//! Validation lives in [`crate::validation`]; persistence in
//! [`crate::persistence`].

pub mod diagram;
pub mod dictionary;
pub mod node;
pub mod project;
pub mod stream;
pub mod types;
