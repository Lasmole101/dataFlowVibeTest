//! Modal dialog windows (spec §5, phase 7c).
//!
//! Each dialog is a floating [`egui::Window`] displayed over the canvas.
//! Dialog state lives in [`crate::app::DialogState`] so text buffers survive
//! across frames.
//!
//! # Usage pattern
//!
//! Open a dialog by setting its `.open = true` and populating its fields.
//! Call `dialog.show(ctx)` every frame; it returns a [`DialogResult`].
//! Match on the result to act on confirmation or cancellation.
//!
//! ```ignore
//! // Open:
//! state.dialogs.rename.open = true;
//! state.dialogs.rename.buffer = current_name.clone();
//! state.dialogs.rename.target_id = Some(id);
//!
//! // Each frame:
//! match state.dialogs.rename.show(ctx) {
//!     DialogResult::Confirmed(new_name) => { /* dispatch RenameType */ }
//!     DialogResult::Cancelled           => { /* nothing */ }
//!     DialogResult::Pending             => { /* still open */ }
//! }
//! ```
//!
//! # Common UX conventions
//!
//! * **Enter** confirms (equivalent to clicking the primary button).
//! * **Escape** cancels.
//! * The primary action button is disabled when input is empty or invalid.
//! * The text field receives focus on the first frame the dialog is open.
//! * All dialog windows are `collapsible(false)` and `resizable(false)` and
//!   anchored to the screen centre.

use egui::{Color32, Context, Key, RichText};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// DialogResult
// ---------------------------------------------------------------------------

/// The outcome of rendering a dialog for one frame.
///
/// Callers match on this each frame and act only on `Confirmed` or `Cancelled`.
/// `Pending` means the dialog is still open and the user has not yet acted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogResult<T> {
    /// The dialog is open and the user has not yet confirmed or cancelled.
    Pending,
    /// The user confirmed the dialog this frame.  The inner value is the result.
    Confirmed(T),
    /// The user cancelled or dismissed the dialog this frame.
    Cancelled,
}

impl<T> DialogResult<T> {
    /// Returns `true` if the dialog is still open.
    pub fn is_pending(&self) -> bool {
        matches!(self, DialogResult::Pending)
    }

    /// Returns `true` if the user just confirmed.
    pub fn is_confirmed(&self) -> bool {
        matches!(self, DialogResult::Confirmed(_))
    }

    /// Returns `true` if the user just cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, DialogResult::Cancelled)
    }
}

// ---------------------------------------------------------------------------
// Shared anchor helper
// ---------------------------------------------------------------------------

/// Build a centred, non-collapsible, non-resizable [`egui::Window`] with the
/// given title.  Apply further settings before calling `.show()`.
fn modal_window(title: &str) -> egui::Window<'_> {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
}

// ---------------------------------------------------------------------------
// RenameDialog
// ---------------------------------------------------------------------------

/// State for an in-progress rename operation.
///
/// Works for renaming diagrams and nodes (anything with a `String` name).
#[derive(Debug, Default)]
pub struct RenameDialog {
    /// Buffer for the text field.
    pub buffer: String,
    /// Whether the dialog is currently open.
    pub open: bool,
    /// The UUID of the component being renamed (used by the caller).
    pub target_id: Option<Uuid>,
    /// Whether this is the first frame the dialog has been shown
    /// (used to auto-focus the text field).
    first_frame: bool,
}

impl RenameDialog {
    /// Open the dialog, pre-filling `buffer` with `current_name`.
    pub fn open_for(&mut self, target_id: Uuid, current_name: &str) {
        self.open = true;
        self.target_id = Some(target_id);
        self.buffer = current_name.to_owned();
        self.first_frame = true;
    }

    /// Render the dialog.
    ///
    /// Returns:
    /// * [`DialogResult::Pending`] while the dialog is open and undecided.
    /// * [`DialogResult::Confirmed`]`(new_name)` on confirmation.
    /// * [`DialogResult::Cancelled`] on dismissal.
    ///
    /// Sets `self.open = false` on both `Confirmed` and `Cancelled`.
    pub fn show(&mut self, ctx: &Context) -> DialogResult<String> {
        if !self.open {
            return DialogResult::Pending;
        }

        let mut result = DialogResult::Pending;

        egui::Window::new("Rename")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("New name:");
                let resp = ui.text_edit_singleline(&mut self.buffer);

                if self.first_frame {
                    resp.request_focus();
                    self.first_frame = false;
                }

                let enter = ui.input(|i| i.key_pressed(Key::Enter));
                let esc = ui.input(|i| i.key_pressed(Key::Escape));
                let can_confirm = !self.buffer.trim().is_empty();

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_confirm, egui::Button::new("OK"))
                        .clicked()
                        || (enter && can_confirm)
                    {
                        result = DialogResult::Confirmed(self.buffer.trim().to_owned());
                    }
                    if ui.button("Cancel").clicked() || esc {
                        result = DialogResult::Cancelled;
                    }
                });
            });

        if result != DialogResult::Pending {
            self.open = false;
            self.target_id = None;
        }
        result
    }
}

// ---------------------------------------------------------------------------
// CreateGroupDialog
// ---------------------------------------------------------------------------

/// State for the "create stream group" dialog.
///
/// The caller pre-populates `candidate_stream_ids` with the streams that will
/// join the new group.  The dialog only collects the group name.
#[derive(Debug, Default)]
pub struct CreateGroupDialog {
    /// Buffer for the group name text field.
    pub buffer: String,
    /// Whether the dialog is open.
    pub open: bool,
    /// Stream IDs that will form the new group (read by the caller on confirm).
    pub candidate_stream_ids: Vec<Uuid>,
    first_frame: bool,
}

impl CreateGroupDialog {
    /// Open the dialog for the given set of stream IDs.
    pub fn open_for(&mut self, stream_ids: Vec<Uuid>) {
        self.open = true;
        self.candidate_stream_ids = stream_ids;
        self.buffer.clear();
        self.first_frame = true;
    }

    /// Render the dialog.
    ///
    /// Returns:
    /// * [`DialogResult::Pending`] while open.
    /// * [`DialogResult::Confirmed`]`(group_name)` on confirmation.
    /// * [`DialogResult::Cancelled`] on dismissal.
    pub fn show(&mut self, ctx: &Context) -> DialogResult<String> {
        if !self.open {
            return DialogResult::Pending;
        }

        let mut result = DialogResult::Pending;

        modal_window("Create Stream Group").show(ctx, |ui| {
            let stream_count = self.candidate_stream_ids.len();
            ui.label(format!(
                "Creating a group for {stream_count} stream{}.",
                if stream_count == 1 { "" } else { "s" }
            ));
            ui.add_space(4.0);
            ui.label("Group name:");
            let resp = ui.text_edit_singleline(&mut self.buffer);

            if self.first_frame {
                resp.request_focus();
                self.first_frame = false;
            }

            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            let esc = ui.input(|i| i.key_pressed(Key::Escape));
            let can_confirm = !self.buffer.trim().is_empty();

            if !can_confirm && !self.buffer.is_empty() {
                ui.label(
                    RichText::new("Name cannot be blank.")
                        .color(Color32::from_rgb(220, 70, 70))
                        .small(),
                );
            }

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_confirm, egui::Button::new("Create"))
                    .clicked()
                    || (enter && can_confirm)
                {
                    result = DialogResult::Confirmed(self.buffer.trim().to_owned());
                }
                if ui.button("Cancel").clicked() || esc {
                    result = DialogResult::Cancelled;
                }
            });
        });

        if result != DialogResult::Pending {
            self.open = false;
        }
        result
    }
}

// ---------------------------------------------------------------------------
// SetSubDiagramDialog
// ---------------------------------------------------------------------------

/// State for the "set sub-diagram" picker dialog.
///
/// Shows a list of orphaned diagrams; the user picks one to attach to the
/// target process node.
#[derive(Debug, Default)]
pub struct SetSubDiagramDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// The process node that will receive the chosen sub-diagram.
    pub target_process_id: Option<Uuid>,
    /// Available orphaned diagrams `(id, name)`.
    pub orphans: Vec<(Uuid, String)>,
    /// The UUID hovered/selected in the list this frame.
    selected: Option<Uuid>,
}

impl SetSubDiagramDialog {
    /// Open the dialog for the given process node and orphan list.
    pub fn open_for(&mut self, process_id: Uuid, orphans: Vec<(Uuid, String)>) {
        self.open = true;
        self.target_process_id = Some(process_id);
        self.orphans = orphans;
        self.selected = None;
    }

    /// Render the dialog.
    ///
    /// Returns:
    /// * [`DialogResult::Pending`] while open.
    /// * [`DialogResult::Confirmed`]`(orphan_uuid)` when the user picks a diagram.
    /// * [`DialogResult::Cancelled`] on dismissal.
    pub fn show(&mut self, ctx: &Context) -> DialogResult<Uuid> {
        if !self.open {
            return DialogResult::Pending;
        }

        let mut result = DialogResult::Pending;

        modal_window("Set Sub-diagram")
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label("Select an orphaned diagram to attach to this process:");
                ui.separator();

                if self.orphans.is_empty() {
                    ui.label(RichText::new("No orphaned diagrams available.").color(Color32::GRAY));
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for &(id, ref name) in &self.orphans {
                                let is_selected = self.selected == Some(id);
                                if ui
                                    .selectable_label(is_selected, format!("📄 {name}"))
                                    .clicked()
                                {
                                    self.selected = Some(id);
                                }
                            }
                        });
                }

                ui.separator();

                let esc = ui.input(|i| i.key_pressed(Key::Escape));
                let can_confirm = self.selected.is_some() && !self.orphans.is_empty();

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_confirm, egui::Button::new("Attach"))
                        .clicked()
                    {
                        if let Some(chosen) = self.selected {
                            result = DialogResult::Confirmed(chosen);
                        }
                    }
                    if ui.button("Cancel").clicked() || esc {
                        result = DialogResult::Cancelled;
                    }
                });
            });

        if result != DialogResult::Pending {
            self.open = false;
            self.target_process_id = None;
            self.selected = None;
        }
        result
    }
}

// ---------------------------------------------------------------------------
// ConfirmDeleteDialog
// ---------------------------------------------------------------------------

/// State for a generic yes/no confirmation dialog.
///
/// Used when deleting a type that is in use, or before any destructive action
/// that cannot be immediately undone.
#[derive(Debug, Default)]
pub struct ConfirmDeleteDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// The message displayed to the user (describes what will be deleted).
    pub message: String,
}

impl ConfirmDeleteDialog {
    /// Open the dialog with the given warning message.
    pub fn open_with(&mut self, message: impl Into<String>) {
        self.open = true;
        self.message = message.into();
    }

    /// Render the dialog.
    ///
    /// Returns:
    /// * [`DialogResult::Pending`] while open.
    /// * [`DialogResult::Confirmed`]`(true)` when the user clicks "Delete".
    /// * [`DialogResult::Cancelled`] when the user clicks "Cancel" or presses Escape.
    pub fn show(&mut self, ctx: &Context) -> DialogResult<bool> {
        if !self.open {
            return DialogResult::Pending;
        }

        let mut result = DialogResult::Pending;

        modal_window("Confirm Delete").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("⚠")
                        .color(Color32::from_rgb(220, 130, 50))
                        .heading(),
                );
                ui.vertical(|ui| {
                    ui.label(&self.message);
                });
            });

            ui.add_space(4.0);
            ui.separator();

            let esc = ui.input(|i| i.key_pressed(Key::Escape));

            ui.horizontal(|ui| {
                // Primary (destructive) action on the left.
                if ui
                    .add(egui::Button::new(
                        RichText::new("Delete").color(Color32::from_rgb(220, 70, 70)),
                    ))
                    .clicked()
                {
                    result = DialogResult::Confirmed(true);
                }
                if ui.button("Cancel").clicked() || esc {
                    result = DialogResult::Cancelled;
                }
            });
        });

        if result != DialogResult::Pending {
            self.open = false;
        }
        result
    }
}
