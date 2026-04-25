//! Modal dialogs (spec §5, Phase 7c).
//!
//! Each dialog is shown conditionally based on a flag in [`crate::app::AppState`].
//! Dialogs are implemented as floating egui [`egui::Window`]s displayed on top
//! of the canvas.

use egui::{Context, Ui};
use uuid::Uuid;

use crate::app::AppState;

// ---------------------------------------------------------------------------
// RenameDialog
// ---------------------------------------------------------------------------

/// State for an in-progress rename operation.
#[derive(Debug, Default)]
pub struct RenameDialog {
    /// The text currently typed in the name field.
    pub buffer: String,
    /// Whether the dialog is open.
    pub open: bool,
    /// The ID of the component being renamed.
    pub target_id: Option<Uuid>,
}

impl RenameDialog {
    /// Render the dialog.  Returns the confirmed new name when the user
    /// accepts, or `None` if the dialog is still open or was cancelled.
    pub fn show(&mut self, ctx: &Context) -> Option<String> {
        todo!("Phase 7 — RenameDialog::show")
    }
}

// ---------------------------------------------------------------------------
// CreateGroupDialog
// ---------------------------------------------------------------------------

/// State for the "create stream group" dialog.
#[derive(Debug, Default)]
pub struct CreateGroupDialog {
    /// The text currently typed in the name field.
    pub buffer: String,
    /// Whether the dialog is open.
    pub open: bool,
    /// The stream IDs that will form the new group.
    pub candidate_stream_ids: Vec<Uuid>,
}

impl CreateGroupDialog {
    /// Render the dialog.  Returns the confirmed group name when the user
    /// accepts, or `None` otherwise.
    pub fn show(&mut self, ctx: &Context) -> Option<String> {
        todo!("Phase 7 — CreateGroupDialog::show")
    }
}

// ---------------------------------------------------------------------------
// SetSubDiagramDialog
// ---------------------------------------------------------------------------

/// State for the "set sub-diagram" dialog.
#[derive(Debug, Default)]
pub struct SetSubDiagramDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// The process node that will receive the chosen sub-diagram.
    pub target_process_id: Option<Uuid>,
    /// Available orphaned diagrams shown in the picker.
    pub orphans: Vec<(Uuid, String)>,
}

impl SetSubDiagramDialog {
    /// Render the dialog.  Returns the chosen orphan [`Uuid`] when the user
    /// selects one, or `None` otherwise.
    pub fn show(&mut self, ctx: &Context) -> Option<Uuid> {
        todo!("Phase 7 — SetSubDiagramDialog::show")
    }
}

// ---------------------------------------------------------------------------
// ConfirmDeleteDialog
// ---------------------------------------------------------------------------

/// State for a generic yes/no confirmation dialog.
#[derive(Debug, Default)]
pub struct ConfirmDeleteDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// The message displayed to the user.
    pub message: String,
}

impl ConfirmDeleteDialog {
    /// Render the dialog.  Returns `true` if the user confirmed, `false` if
    /// they cancelled, or `None` if the dialog is still open.
    pub fn show(&mut self, ctx: &Context) -> Option<bool> {
        todo!("Phase 7 — ConfirmDeleteDialog::show")
    }
}
