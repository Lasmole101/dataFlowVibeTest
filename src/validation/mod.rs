//! Validation subsystem (spec §4).
//!
//! Validation runs in two modes:
//! * **Incremental** — triggered automatically on every change; checks only
//!   the changed component and its direct neighbours.
//! * **Full** — triggered by a UI button; checks the entire project.
//!
//! Results are stored in a [`validator::ValidationCache`] that the canvas and
//! tree pane read to overlay error / warning badges.

pub mod balance;
pub mod flatten;
pub mod validator;
