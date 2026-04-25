//! Type specification panel (spec §5.3).
//!
//! Opened on demand for the selected node or stream.  Displays the current
//! [`crate::model::types::TypeSpec`] and provides controls to add, remove, and
//! adjust counts of type entries.
//!
//! Boundary nodes are displayed in read-only mode (spec §6.2).

use egui::Ui;
use uuid::Uuid;

use crate::app::AppState;

/// Render the specification editor for component `target_id`.
///
/// `target_id` may refer to either a node or a stream; the panel queries
/// `state.project` to determine which.
pub fn show(state: &mut AppState, target_id: Uuid, ui: &mut Ui) {
    todo!("Phase 7 — spec_panel::show")
}
