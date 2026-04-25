//! Top-level application state.
//!
//! [`AppState`] owns every subsystem and acts as the single source of truth
//! threaded through each egui frame.  The UI modules receive a mutable
//! reference to this struct and dispatch [`undo::history::Command`]s through
//! it rather than mutating the project directly.

use eframe::CreationContext;
use egui::Context;
use uuid::Uuid;

use crate::{
    model::project::Project,
    undo::history::UndoStack,
    validation::validator::ValidationCache,
};

/// Identifies which panel (if any) is currently open in the right-hand side.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum ActivePanel {
    /// No panel open.
    #[default]
    None,
    /// Type specification editor for the given component.
    Spec(Uuid),
    /// Data dictionary editor.
    Dictionary,
}

/// Persistent drag state, isolated so egui's immediate-mode loop cannot
/// accidentally infer drag progress from widget positions.
#[derive(Debug, Default)]
pub struct DragState {
    /// Node being dragged, and the canvas-space offset from its origin to the
    /// pointer at drag-start.
    pub dragging_node: Option<(Uuid, egui::Vec2)>,
    /// Stream-creation in progress: the source node and current pointer
    /// position in canvas space.
    pub creating_stream: Option<(Uuid, egui::Pos2)>,
}

/// Root application state.  A single instance lives for the lifetime of the
/// process.
pub struct AppState {
    /// The loaded project (diagrams, data dictionary, etc.).
    pub project: Project,
    /// Global undo/redo stack covering all mutations.
    pub undo_stack: UndoStack,
    /// Cached validation results, updated incrementally on every change.
    pub validation: ValidationCache,
    /// The diagram currently visible on the canvas.
    pub active_diagram_id: Uuid,
    /// The currently selected component (node or stream), if any.
    pub selected_id: Option<Uuid>,
    /// Which side panel is open.
    pub active_panel: ActivePanel,
    /// Drag interaction state.
    pub drag_state: DragState,
    /// Path of the currently open file, if any.
    pub current_file: Option<std::path::PathBuf>,
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
            drag_state: DragState::default(),
            current_file: None,
        }
    }
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Phase 0: render a placeholder so the window is not blank.
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Data Flow Diagramming Tool");
            ui.label("Phase 0 scaffold — implementation in progress.");
        });
    }
}
