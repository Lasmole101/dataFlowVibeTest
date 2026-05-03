//! Top-level application state and the [`eframe::App`] implementation.
//!
//! [`AppState`] is the single source of truth for the entire application.
//! Every UI module receives `&mut AppState` and dispatches
//! [`crate::undo::history::Command`]s through it rather than mutating
//! `project` directly.
//!
//! # Frame layout
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  TopBottomPanel  (toolbar)                          │
//! ├──────────────┬──────────────────────────────────────┤
//! │ SidePanel    │  CentralPanel  (canvas)              │
//! │ (tree pane)  │                                      │
//! │              │                                      │
//! ├──────────────┴──────────────────────────────────────┤
//! │  TopBottomPanel  (status bar)                       │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! egui requires panels to be registered in the order: top/bottom first,
//! then side panels, then the central panel (which takes the remainder).
//!
//! # Dialog ownership
//!
//! All modal dialog states live on [`AppState`] so their text buffers and
//! open-flags survive across frames.  The dialog `show()` methods are called
//! from `update()` and return their result (confirmed value or `None`) each
//! frame.

use eframe::CreationContext;
use egui::{Context, FontId, RichText, Ui};
use uuid::Uuid;

use crate::{
    model::project::Project,
    ui::dialogs::{ConfirmDeleteDialog, CreateGroupDialog, RenameDialog, SetSubDiagramDialog},
    ui::dictionary_panel::DictionaryPanelState,
    ui::spec_panel::SpecPanelState,
    undo::history::UndoStack,
    validation::validator::ValidationCache,
};

// ---------------------------------------------------------------------------
// ActivePanel
// ---------------------------------------------------------------------------

/// Which right-hand panel is currently visible.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum ActivePanel {
    /// No right-hand panel open; the canvas takes full width.
    #[default]
    None,
    /// Type specification editor for the given component (node or stream).
    Spec(Uuid),
    /// Data dictionary editor.
    Dictionary,
}

// ---------------------------------------------------------------------------
// DragState
// ---------------------------------------------------------------------------

/// Persistent canvas drag state.
///
/// Isolated from the rest of `AppState` so the immediate-mode loop cannot
/// accidentally read stale drag progress from widget positions.
#[derive(Debug, Default)]
pub struct DragState {
    /// Node being dragged: `(node_id, pointer_offset_from_node_origin_in_canvas_space)`.
    pub dragging_node: Option<(Uuid, egui::Vec2)>,
    /// Stream creation in progress: `(source_node_id, current_pointer_in_canvas_space)`.
    pub creating_stream: Option<(Uuid, egui::Pos2)>,
}

// ---------------------------------------------------------------------------
// DialogState
// ---------------------------------------------------------------------------

/// All modal dialog states bundled together.
///
/// Owned by `AppState` so dialog text buffers survive between frames.
#[derive(Debug, Default)]
pub struct DialogState {
    pub rename: RenameDialog,
    pub create_group: CreateGroupDialog,
    pub set_subdiagram: SetSubDiagramDialog,
    pub confirm_delete: ConfirmDeleteDialog,
    /// Persistent state for the data dictionary editor panel.
    pub dictionary: DictionaryPanelState,
    /// Persistent state for the spec editor panel.
    pub spec: SpecPanelState,
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Root application state.  A single instance lives for the lifetime of the
/// process and is owned by the eframe runtime.
pub struct AppState {
    // ---- Core subsystems ----
    /// The loaded project (diagrams, data dictionary, etc.).
    pub project: Project,
    /// Global undo/redo stack covering all mutations.
    pub undo_stack: UndoStack,
    /// Cached validation results, updated incrementally on every change.
    pub validation: ValidationCache,

    // ---- Navigation ----
    /// UUID of the diagram currently visible on the canvas.
    pub active_diagram_id: Uuid,

    // ---- Selection ----
    /// The currently selected component (node or stream UUID), if any.
    pub selected_id: Option<Uuid>,

    // ---- Panel state ----
    /// Which right-hand side panel is open.
    pub active_panel: ActivePanel,
    /// Width of the left-hand tree pane in logical pixels.
    pub tree_pane_width: f32,
    /// Whether the tree pane is visible.
    pub tree_pane_visible: bool,

    // ---- Canvas interaction ----
    /// Persistent drag state for the canvas.
    pub drag_state: DragState,

    // ---- Dialogs ----
    /// All modal dialog states.
    pub dialogs: DialogState,

    // ---- File state ----
    /// Path of the currently open file, if any.
    pub current_file: Option<std::path::PathBuf>,
    /// Unsaved-changes flag.  Set by every undo command; cleared on save.
    pub dirty: bool,

    // ---- Status bar ----
    /// Short message shown in the status bar (cleared each frame after display).
    pub status_message: Option<String>,
}

impl AppState {
    /// Construct a fresh [`AppState`] with an empty project.
    pub fn new(_cc: &CreationContext<'_>) -> Self {
        let project = Project::new();
        let root_id = project.root_diagram_id;
        Self {
            project,
            undo_stack: UndoStack::default(),
            validation: ValidationCache::default(),
            active_diagram_id: root_id,
            selected_id: None,
            active_panel: ActivePanel::None,
            tree_pane_width: 220.0,
            tree_pane_visible: true,
            drag_state: DragState::default(),
            dialogs: DialogState::default(),
            current_file: None,
            dirty: false,
            status_message: None,
        }
    }

    // -----------------------------------------------------------------------
    // Helpers called by UI modules
    // -----------------------------------------------------------------------

    /// Returns the name of the currently active diagram (for the window title).
    pub fn active_diagram_name(&self) -> &str {
        self.project
            .diagram(self.active_diagram_id)
            .map(|d| d.name.as_str())
            .unwrap_or("(unknown diagram)")
    }

    /// Post a status-bar message for one frame.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
    }

    /// Window title: "DataFlow — <diagram name> [*]"
    fn window_title(&self) -> String {
        let dirty_marker = if self.dirty { " [*]" } else { "" };
        let file_name = self
            .current_file
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("Untitled");
        format!("DataFlow — {file_name}{dirty_marker}")
    }
}

// ---------------------------------------------------------------------------
// eframe::App implementation
// ---------------------------------------------------------------------------

impl eframe::App for AppState {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Update the window title each frame.
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.window_title()));

        // ── Top: toolbar ──────────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            render_toolbar(self, ui);
        });

        // ── Bottom: status bar ────────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            render_status_bar(self, ui);
        });

        // ── Left: tree pane ───────────────────────────────────────────────
        if self.tree_pane_visible {
            egui::SidePanel::left("tree_pane")
                .resizable(true)
                .default_width(self.tree_pane_width)
                .width_range(140.0..=480.0)
                .show(ctx, |ui| {
                    // Store the current width so it survives panel hide/show.
                    self.tree_pane_width = ui.available_width();
                    render_tree_pane(self, ui);
                });
        }

        // ── Right: spec / dictionary panel ────────────────────────────────
        let panel_open = self.active_panel != ActivePanel::None;
        if panel_open {
            egui::SidePanel::right("right_panel")
                .resizable(true)
                .default_width(280.0)
                .width_range(200.0..=480.0)
                .show(ctx, |ui| {
                    render_right_panel(self, ui);
                });
        }

        // ── Centre: canvas (placeholder) ──────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            render_canvas_placeholder(self, ui);
        });

        // ── Floating dialogs (rendered over everything) ───────────────────
        // The tree pane may open the set-subdiagram dialog; handle it here
        // so it floats over the full window.
        render_set_subdiagram_dialog(self, ctx);
    }
}

// ---------------------------------------------------------------------------
// Panel render functions (inline stubs — replaced by ui/* modules in Phase 7)
// ---------------------------------------------------------------------------

/// Renders the top toolbar strip — delegates to [`crate::ui::toolbar::show`].
fn render_toolbar(state: &mut AppState, ui: &mut Ui) {
    crate::ui::toolbar::show(state, ui);
}

/// Renders the bottom status bar.
fn render_status_bar(state: &mut AppState, ui: &mut Ui) {
    ui.horizontal(|ui| {
        // Left: diagram name.
        ui.label(RichText::new(state.active_diagram_name()).font(FontId::proportional(12.0)));

        ui.separator();

        // Validation summary.
        if state.validation.has_errors() {
            ui.label(RichText::new("⚠ Errors").color(egui::Color32::RED));
        } else if state.validation.has_warnings() {
            ui.label(RichText::new("⚠ Warnings").color(egui::Color32::YELLOW));
        } else {
            ui.label(RichText::new("✓ Valid").color(egui::Color32::GREEN));
        }

        ui.separator();

        // Right: status message (shown for one frame then cleared).
        if let Some(msg) = state.status_message.take() {
            ui.label(RichText::new(msg).font(FontId::proportional(12.0)));
        }

        // File path.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(path) = &state.current_file {
                ui.label(
                    RichText::new(path.display().to_string())
                        .font(FontId::proportional(11.0))
                        .color(egui::Color32::GRAY),
                );
            }
        });
    });
}

/// Renders the left tree pane — delegates to [`crate::ui::tree_pane::show`].
fn render_tree_pane(state: &mut AppState, ui: &mut Ui) {
    crate::ui::tree_pane::show(state, ui);
}

/// Renders the right-hand panel (spec editor or dictionary).
///
/// Phase 6a stub: shows a placeholder.  Full implementation in Phase 7.
fn render_right_panel(state: &mut AppState, ui: &mut Ui) {
    match &state.active_panel {
        ActivePanel::None => {}
        ActivePanel::Dictionary => {
            crate::ui::dictionary_panel::show(state, ui);
        }
        ActivePanel::Spec(id) => {
            let id = *id;
            crate::ui::spec_panel::show(state, id, ui);
        }
    }
}

/// Renders the central canvas area — delegates to [`crate::ui::canvas::show`].
fn render_canvas_placeholder(state: &mut AppState, ui: &mut Ui) {
    crate::ui::canvas::show(state, ui);
}

/// "Set Sub-diagram" dialog — delegates to [`crate::ui::dialogs::SetSubDiagramDialog::show`].
fn render_set_subdiagram_dialog(state: &mut AppState, ctx: &Context) {
    // Snapshot process_id before the mutable borrow of dialogs.
    let process_id = state.dialogs.set_subdiagram.target_process_id;

    match state.dialogs.set_subdiagram.show(ctx) {
        crate::ui::dialogs::DialogResult::Confirmed(orphan_id) => {
            if let Some(pid) = process_id {
                let cmd = crate::undo::commands::AttachOrphan {
                    orphan_diagram_id: orphan_id,
                    process_node_id: pid,
                };
                if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                    state.set_status(format!("Attach orphan failed: {e}"));
                } else {
                    state.dirty = true;
                    state.validation.validate_full(&state.project);
                }
            }
        }
        crate::ui::dialogs::DialogResult::Cancelled | crate::ui::dialogs::DialogResult::Pending => {
        }
    }
}
