//! User interface modules built on [`egui`].
//!
//! Each sub-module is responsible for a distinct area of the window.  The UI
//! modules do not mutate [`crate::model`] directly; instead they push
//! [`crate::undo::history::Command`]s through [`crate::app::AppState`].

pub mod canvas;
pub mod dialogs;
pub mod dictionary_panel;
pub mod spec_panel;
pub mod toolbar;
pub mod tree_pane;
