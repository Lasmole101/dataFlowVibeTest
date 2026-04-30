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
    }
}

// ---------------------------------------------------------------------------
// Panel render functions (inline stubs — replaced by ui/* modules in Phase 7)
// ---------------------------------------------------------------------------

/// Renders the top toolbar strip.
///
/// Phase 6a: menu bar with File / Edit / View menus and basic undo/redo
/// buttons that are already wired up.  Full toolbar implemented in Phase 7.
fn render_toolbar(state: &mut AppState, ui: &mut Ui) {
    egui::menu::bar(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("New").clicked() {
                state.project = Project::new();
                state.active_diagram_id = state.project.root_diagram_id;
                state.undo_stack.clear();
                state.validation.clear_all();
                state.selected_id = None;
                state.active_panel = ActivePanel::None;
                state.current_file = None;
                state.dirty = false;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });

        ui.menu_button("Edit", |ui| {
            let can_undo = state.undo_stack.can_undo();
            let can_redo = state.undo_stack.can_redo();
            let undo_label = state
                .undo_stack
                .undo_description()
                .map(|d| format!("Undo: {d}"))
                .unwrap_or_else(|| "Undo".into());
            let redo_label = state
                .undo_stack
                .redo_description()
                .map(|d| format!("Redo: {d}"))
                .unwrap_or_else(|| "Redo".into());

            if ui
                .add_enabled(can_undo, egui::Button::new(undo_label))
                .clicked()
            {
                if let Err(e) = state.undo_stack.undo(&mut state.project) {
                    state.set_status(format!("Undo failed: {e}"));
                }
                ui.close_menu();
            }
            if ui
                .add_enabled(can_redo, egui::Button::new(redo_label))
                .clicked()
            {
                if let Err(e) = state.undo_stack.redo(&mut state.project) {
                    state.set_status(format!("Redo failed: {e}"));
                }
                ui.close_menu();
            }
        });

        ui.menu_button("View", |ui| {
            if ui
                .checkbox(&mut state.tree_pane_visible, "Show tree pane")
                .clicked()
            {
                ui.close_menu();
            }
            if ui.button("Dictionary").clicked() {
                state.active_panel = if state.active_panel == ActivePanel::Dictionary {
                    ActivePanel::None
                } else {
                    ActivePanel::Dictionary
                };
                ui.close_menu();
            }
        });

        // Quick undo/redo buttons to the right of the menus.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_redo = state.undo_stack.can_redo();
            let can_undo = state.undo_stack.can_undo();
            if ui.add_enabled(can_redo, egui::Button::new("⟳")).clicked() {
                let _ = state.undo_stack.redo(&mut state.project);
            }
            if ui.add_enabled(can_undo, egui::Button::new("⟲")).clicked() {
                let _ = state.undo_stack.undo(&mut state.project);
            }
            ui.separator();
            // Dirty indicator.
            if state.dirty {
                ui.label(RichText::new("●").color(egui::Color32::YELLOW))
                    .on_hover_text("Unsaved changes");
            }
        });
    });
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

/// Renders the left tree pane (project hierarchy).
///
/// Phase 6a stub: shows project structure as a label list.
/// Full implementation in Phase 7 (`ui::tree_pane::show`).
fn render_tree_pane(state: &mut AppState, ui: &mut Ui) {
    ui.vertical(|ui| {
        ui.heading("Project");
        ui.separator();

        // Root diagram.
        let root_id = state.project.root_diagram_id;
        let root_name = state
            .project
            .diagram(root_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| "Root".into());

        let root_selected = state.active_diagram_id == root_id;
        if ui
            .selectable_label(root_selected, format!("📄 {root_name}"))
            .clicked()
        {
            state.active_diagram_id = root_id;
        }

        // Sub-diagrams (non-root, non-orphaned).
        let sub_ids: Vec<(Uuid, String)> = state
            .project
            .diagrams
            .iter()
            .filter(|(&id, _)| id != root_id && !state.project.is_orphaned(id))
            .map(|(&id, d)| (id, d.name.clone()))
            .collect();

        if !sub_ids.is_empty() {
            ui.indent("sub_diagrams", |ui| {
                for (id, name) in &sub_ids {
                    let selected = state.active_diagram_id == *id;
                    if ui
                        .selectable_label(selected, format!("  📄 {name}"))
                        .clicked()
                    {
                        state.active_diagram_id = *id;
                    }
                }
            });
        }

        // Orphaned diagrams.
        if !state.project.orphaned_diagram_ids.is_empty() {
            ui.separator();
            ui.label(RichText::new("Orphaned").color(egui::Color32::GRAY));
            let orphan_ids: Vec<(Uuid, String)> = state
                .project
                .orphaned_diagram_ids
                .iter()
                .filter_map(|&id| state.project.diagram(id).map(|d| (id, d.name.clone())))
                .collect();
            ui.indent("orphans", |ui| {
                for (id, name) in &orphan_ids {
                    let selected = state.active_diagram_id == *id;
                    if ui
                        .selectable_label(selected, format!("  ⚠ {name}"))
                        .clicked()
                    {
                        state.active_diagram_id = *id;
                    }
                }
            });
        }

        ui.separator();

        // Dictionary shortcut.
        let dict_selected = state.active_panel == ActivePanel::Dictionary;
        if ui
            .selectable_label(dict_selected, "📚 Data Dictionary")
            .clicked()
        {
            state.active_panel = if dict_selected {
                ActivePanel::None
            } else {
                ActivePanel::Dictionary
            };
        }
    });
}

/// Renders the right-hand panel (spec editor or dictionary).
///
/// Phase 6a stub: shows a placeholder.  Full implementation in Phase 7.
fn render_right_panel(state: &mut AppState, ui: &mut Ui) {
    match &state.active_panel {
        ActivePanel::None => {}
        ActivePanel::Dictionary => {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.heading("Data Dictionary");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            state.active_panel = ActivePanel::None;
                        }
                    });
                });
                ui.separator();
                ui.label("Dictionary editor — Phase 7.");
                ui.separator();
                let prim_count = state.project.data_dictionary.primitives.len();
                let comp_count = state.project.data_dictionary.composites.len();
                ui.label(format!("{prim_count} primitive type(s)"));
                ui.label(format!("{comp_count} composite type(s)"));
            });
        }
        ActivePanel::Spec(id) => {
            let id = *id; // copy before borrowing state
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.heading("Spec Editor");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            state.active_panel = ActivePanel::None;
                        }
                    });
                });
                ui.separator();
                ui.label(format!("Component: {}", id.hyphenated()));
                ui.label("Spec editor — Phase 7.");
            });
        }
    }
}

/// Renders the central canvas area.
///
/// Phase 6a: placeholder with diagram info.  Canvas drawing implemented in
/// Phase 8 (`ui::canvas::show`).
fn render_canvas_placeholder(state: &mut AppState, ui: &mut Ui) {
    let diagram = state.project.diagram(state.active_diagram_id);

    ui.vertical_centered(|ui| {
        ui.add_space(40.0);

        ui.heading(
            state
                .project
                .diagram(state.active_diagram_id)
                .map(|d| d.name.as_str())
                .unwrap_or("(no diagram)"),
        );

        ui.add_space(12.0);

        if let Some(diag) = diagram {
            let node_count = diag.nodes.len();
            let stream_count = diag.streams.len();
            ui.label(format!("{node_count} node(s)   {stream_count} stream(s)"));
        }

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(12.0);
        ui.label(
            RichText::new("Canvas rendering implemented in Phase 8.").color(egui::Color32::GRAY),
        );
    });
}
