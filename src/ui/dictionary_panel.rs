//! Data dictionary editor panel.
//!
//! Lists all primitive and composite types in the project's
//! [`crate::model::dictionary::DataDictionary`] and provides controls to
//! create, rename, and delete types.
//!
//! Rename and delete operations propagate automatically through the dictionary;
//! referencing nodes and streams are updated or placed into an error state as
//! described in spec §2.

use egui::Ui;

use crate::app::AppState;

/// Render the dictionary editor panel into `ui`.
pub fn show(state: &mut AppState, ui: &mut Ui) {
    todo!("Phase 7 — dictionary_panel::show")
}
