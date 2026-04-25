//! Main canvas panel (spec §5.1, §8).
//!
//! Renders the active diagram and handles all direct-manipulation interaction:
//! zoom/pan, node selection, drag/drop node placement, and stream creation.
//!
//! The canvas maintains a `transform` mapping canvas-space coordinates
//! (stored in [`crate::model::diagram::Diagram::positions`]) to screen-space
//! pixels.  All hit-testing is performed in canvas space.

use egui::{Context, Ui};

use crate::app::AppState;

/// Render the main canvas area for the currently active diagram.
///
/// Reads from `state.project.diagrams[state.active_diagram_id]` and
/// dispatches commands through `state.undo_stack`.
pub fn show(state: &mut AppState, ui: &mut Ui) {
    todo!("Phase 8 — canvas::show")
}
