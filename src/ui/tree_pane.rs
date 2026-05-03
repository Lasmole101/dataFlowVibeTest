//! Left-hand project tree pane (spec §5.4).
//!
//! # Layout
//!
//! ```text
//! ┌─────────────────────┐
//! │ Project             │
//! ├─────────────────────┤
//! │ 📄 Root             │  ← root diagram (always present)
//! │   ⚙ Filter         │  ← process node with sub-diagram
//! │     📄 Filter (sub) │  ← sub-diagram, indented under its process
//! │   ⚙ Logger         │  ← process node without sub-diagram
//! ├─────────────────────┤
//! │ ⚠ Orphaned          │  ← section only when non-empty
//! │   📄 Old (sub)      │
//! ├─────────────────────┤
//! │ 📚 Data Dictionary  │
//! └─────────────────────┘
//! ```
//!
//! # Interaction
//!
//! * **Click** a diagram row → sets `AppState::active_diagram_id`.
//! * **Right-click** a diagram row → "Rename" context menu.
//! * **Right-click** a process node row → "Create Sub-diagram" and, when
//!   orphaned diagrams exist, "Set Sub-diagram…".
//!
//! # Rename
//!
//! Opens an inline egui `Window` (not the `dialogs::RenameDialog` stub, which
//! is still `todo!()` pending Phase 7).  The rename state is stored on
//! `AppState::dialogs.rename` — the same fields the full dialog will use —
//! so the transition to the Phase 7 implementation is a drop-in swap.
//!
//! # Badges
//!
//! A diagram badge is the worst `ValidationState` across all its nodes and
//! streams.  Computed locally here rather than in `ValidationCache` because
//! per-diagram aggregation is a UI concern.

use egui::{Color32, RichText, Ui};
use uuid::Uuid;

use crate::{
    app::{ActivePanel, AppState},
    model::node::NodeKind,
    undo::commands::{CreateSubDiagram, RenameNode},
    validation::validator::ValidationState,
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render the tree pane into `ui`.
///
/// Called from [`crate::app::AppState::update`] inside a
/// `SidePanel::left`.
pub fn show(state: &mut AppState, ui: &mut Ui) {
    // Collect all data we need before any mutable borrows of `state`.
    let tree = build_tree(state);

    ui.vertical(|ui| {
        // ── Header ──────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.strong("Project");
        });
        ui.separator();

        // ── Main tree ───────────────────────────────────────────────────
        render_diagram_row(state, ui, &tree.root, 0);

        // ── Orphaned section ─────────────────────────────────────────────
        if !tree.orphaned.is_empty() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠ Orphaned").color(Color32::from_rgb(200, 150, 50)));
            });
            for entry in &tree.orphaned {
                render_orphan_row(state, ui, entry);
            }
        }

        ui.separator();

        // ── Dictionary shortcut ──────────────────────────────────────────
        let dict_open = state.active_panel == ActivePanel::Dictionary;
        let dict_resp = ui.selectable_label(dict_open, "📚 Data Dictionary");
        if dict_resp.clicked() {
            state.active_panel = if dict_open {
                ActivePanel::None
            } else {
                ActivePanel::Dictionary
            };
        }
    });

    // ── Rename dialog (inline — bypasses the todo!() stub) ──────────────
    render_rename_dialog(state, ui);
}

// ---------------------------------------------------------------------------
// Tree data structure
// ---------------------------------------------------------------------------

/// A resolved tree node ready for rendering.
struct DiagramEntry {
    diagram_id: Uuid,
    diagram_name: String,
    badge: ValidationState,
    /// Process node children: `(node_id, node_name, optional sub-diagram)`.
    process_children: Vec<ProcessEntry>,
}

struct ProcessEntry {
    node_id: Uuid,
    node_name: String,
    /// The sub-diagram owned by this process node, if any.
    sub_diagram: Option<Box<DiagramEntry>>,
}

struct OrphanEntry {
    diagram_id: Uuid,
    diagram_name: String,
    badge: ValidationState,
}

struct ProjectTree {
    root: DiagramEntry,
    orphaned: Vec<OrphanEntry>,
}

/// Build the tree from project state, resolving all names and badges up-front
/// so the render phase can be fully `&mut AppState` without nested borrows.
fn build_tree(state: &AppState) -> ProjectTree {
    let root_id = state.project.root_diagram_id;
    let root = build_diagram_entry(state, root_id);

    let orphaned: Vec<OrphanEntry> = state
        .project
        .orphaned_diagram_ids
        .iter()
        .filter_map(|&id| {
            state.project.diagram(id).map(|d| OrphanEntry {
                diagram_id: id,
                diagram_name: d.name.clone(),
                badge: diagram_badge(state, id),
            })
        })
        .collect();

    ProjectTree { root, orphaned }
}

/// Recursively build a `DiagramEntry` for `diagram_id`.
fn build_diagram_entry(state: &AppState, diagram_id: Uuid) -> DiagramEntry {
    let diagram = match state.project.diagram(diagram_id) {
        Some(d) => d,
        None => {
            return DiagramEntry {
                diagram_id,
                diagram_name: "(missing)".into(),
                badge: ValidationState::Error,
                process_children: vec![],
            }
        }
    };

    let diagram_name = diagram.name.clone();
    let badge = diagram_badge(state, diagram_id);

    // Collect process nodes that either have a sub-diagram or are plain
    // process nodes, sorted by name for stability.
    let mut processes: Vec<&crate::model::node::Node> = diagram
        .nodes
        .values()
        .filter(|n| n.kind == NodeKind::Process)
        .collect();
    processes.sort_by(|a, b| a.name.cmp(&b.name));

    let process_children = processes
        .iter()
        .map(|n| {
            let sub_diagram = n
                .sub_diagram_id
                .map(|sub_id| Box::new(build_diagram_entry(state, sub_id)));
            ProcessEntry {
                node_id: n.id,
                node_name: n.name.clone(),
                sub_diagram,
            }
        })
        .collect();

    DiagramEntry {
        diagram_id,
        diagram_name,
        badge,
        process_children,
    }
}

/// Compute the worst `ValidationState` across all nodes and streams in a diagram.
fn diagram_badge(state: &AppState, diagram_id: Uuid) -> ValidationState {
    let Some(diagram) = state.project.diagram(diagram_id) else {
        return ValidationState::Ok;
    };
    let mut worst = ValidationState::Ok;
    for &id in diagram.nodes.keys().chain(diagram.streams.keys()) {
        worst = worst.merge(state.validation.state(id));
        if worst == ValidationState::Error {
            break; // can't get worse
        }
    }
    worst
}

// ---------------------------------------------------------------------------
// Row renderers
// ---------------------------------------------------------------------------

/// Render a diagram row and all its process-node children recursively.
fn render_diagram_row(
    state: &mut AppState,
    ui: &mut Ui,
    entry: &DiagramEntry,
    indent_level: usize,
) {
    let label = diagram_label(&entry.diagram_name, entry.badge);
    let selected = state.active_diagram_id == entry.diagram_id;

    let indent_px = indent_level as f32 * 14.0;
    ui.horizontal(|ui| {
        ui.add_space(indent_px);
        let resp = ui.selectable_label(selected, label);

        // Click → navigate.
        if resp.clicked() {
            state.active_diagram_id = entry.diagram_id;
        }

        // Right-click → diagram context menu.
        resp.context_menu(|ui| {
            diagram_context_menu(state, ui, entry.diagram_id, &entry.diagram_name);
        });
    });

    // Render process node children.
    for proc in &entry.process_children {
        render_process_row(state, ui, proc, indent_level + 1, entry.diagram_id);
    }
}

/// Render a process node row, with optional sub-diagram nested beneath it.
fn render_process_row(
    state: &mut AppState,
    ui: &mut Ui,
    proc: &ProcessEntry,
    indent_level: usize,
    parent_diagram_id: Uuid,
) {
    let indent_px = indent_level as f32 * 14.0;

    // Badge from the sub-diagram (if any).
    let proc_badge = proc
        .sub_diagram
        .as_ref()
        .map(|sd| sd.badge)
        .unwrap_or(ValidationState::Ok);
    let icon = if proc.sub_diagram.is_some() {
        "⚙"
    } else {
        "○"
    };
    let label = node_label(icon, &proc.node_name, proc_badge);

    ui.horizontal(|ui| {
        ui.add_space(indent_px);
        let has_orphans = !state.project.orphaned_diagram_ids.is_empty();
        let resp = ui.label(label);

        // Right-click → process node context menu.
        resp.context_menu(|ui| {
            process_context_menu(
                state,
                ui,
                proc.node_id,
                parent_diagram_id,
                proc.sub_diagram.is_some(),
                has_orphans,
            );
        });
    });

    // Recurse into sub-diagram if present.
    if let Some(sub) = &proc.sub_diagram {
        render_diagram_row(state, ui, sub, indent_level + 1);
    }
}

/// Render an orphaned diagram row.
fn render_orphan_row(state: &mut AppState, ui: &mut Ui, entry: &OrphanEntry) {
    let label = diagram_label(&entry.diagram_name, entry.badge);
    let selected = state.active_diagram_id == entry.diagram_id;

    ui.horizontal(|ui| {
        ui.add_space(14.0); // one indent level
        let resp = ui.selectable_label(selected, label);
        if resp.clicked() {
            state.active_diagram_id = entry.diagram_id;
        }
        resp.context_menu(|ui| {
            diagram_context_menu(state, ui, entry.diagram_id, &entry.diagram_name);
        });
    });
}

// ---------------------------------------------------------------------------
// Context menus
// ---------------------------------------------------------------------------

/// Context menu for a diagram row.
fn diagram_context_menu(state: &mut AppState, ui: &mut Ui, diagram_id: Uuid, current_name: &str) {
    if ui.button("Rename…").clicked() {
        state.dialogs.rename.open_for(diagram_id, current_name);
        ui.close_menu();
    }
}

/// Context menu for a process node row.
fn process_context_menu(
    state: &mut AppState,
    ui: &mut Ui,
    node_id: Uuid,
    parent_diagram_id: Uuid,
    has_sub_diagram: bool,
    has_orphans: bool,
) {
    // "Create Sub-diagram" — only when none exists yet.
    if !has_sub_diagram && ui.button("Create Sub-diagram").clicked() {
        let cmd = CreateSubDiagram {
            parent_diagram_id,
            process_node_id: node_id,
            sub_diagram_id: Uuid::new_v4(),
        };
        if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
            state.set_status(format!("Create sub-diagram failed: {e}"));
        } else {
            state.dirty = true;
            state.validation.validate_full(&state.project);
        }
        ui.close_menu();
    }

    // "Set Sub-diagram…" — only when orphaned diagrams exist and node has no sub-diagram.
    if !has_sub_diagram && has_orphans && ui.button("Set Sub-diagram…").clicked() {
        let orphan_list: Vec<(Uuid, String)> = state
            .project
            .orphaned_diagram_ids
            .iter()
            .filter_map(|&id| state.project.diagram(id).map(|d| (id, d.name.clone())))
            .collect();
        state.dialogs.set_subdiagram.open_for(node_id, orphan_list);
        ui.close_menu();
    }

    // Show a note when a sub-diagram already exists.
    if has_sub_diagram {
        ui.label(
            RichText::new("Already has a sub-diagram")
                .color(Color32::GRAY)
                .italics(),
        );
    }
}

// ---------------------------------------------------------------------------
// Inline rename dialog
// ---------------------------------------------------------------------------

/// Render the inline rename dialog.
/// Delegates to [`crate::ui::dialogs::RenameDialog::show`].
fn render_rename_dialog(state: &mut AppState, ui: &mut Ui) {
    // Snapshot target_id before the mutable borrow of dialogs.
    let target_id = state.dialogs.rename.target_id;

    match state.dialogs.rename.show(ui.ctx()) {
        crate::ui::dialogs::DialogResult::Confirmed(new_name) => {
            if let Some(id) = target_id {
                apply_rename(state, id, new_name);
            }
        }
        crate::ui::dialogs::DialogResult::Cancelled | crate::ui::dialogs::DialogResult::Pending => {
        }
    }
}

/// Apply a rename to either a diagram or a node, whichever `target_id` refers to.
fn apply_rename(state: &mut AppState, target_id: Uuid, new_name: String) {
    // Is it a diagram?
    if state.project.diagrams.contains_key(&target_id) {
        if let Some(diagram) = state.project.diagrams.get_mut(&target_id) {
            diagram.name = new_name;
            state.dirty = true;
        }
        return;
    }

    // Is it a node in any diagram?
    // Find which diagram contains the node.
    let diagram_id = state
        .project
        .diagrams
        .iter()
        .find(|(_, d)| d.nodes.contains_key(&target_id))
        .map(|(&id, _)| id);

    if let Some(diagram_id) = diagram_id {
        let old_name = state
            .project
            .diagram(diagram_id)
            .and_then(|d| d.nodes.get(&target_id))
            .map(|n| n.name.clone())
            .unwrap_or_default();

        let cmd = RenameNode {
            diagram_id,
            node_id: target_id,
            old_name,
            new_name,
        };
        if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
            state.set_status(format!("Rename failed: {e}"));
        } else {
            state.dirty = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Label builders
// ---------------------------------------------------------------------------

/// Build a styled label for a diagram row.
fn diagram_label(name: &str, badge: ValidationState) -> RichText {
    let text = format!("📄 {name}{}", badge_icon(badge));
    apply_badge_color(RichText::new(text), badge)
}

/// Build a styled label for a process node row.
fn node_label(icon: &str, name: &str, badge: ValidationState) -> RichText {
    let text = format!("{icon} {name}{}", badge_icon(badge));
    apply_badge_color(RichText::new(text), badge)
}

/// Return the badge suffix icon for the given `ValidationState`.
fn badge_icon(state: ValidationState) -> &'static str {
    match state {
        ValidationState::Ok => "",
        ValidationState::Warning => " ⚠",
        ValidationState::Error => " ✕",
    }
}

/// Apply a colour to `text` only when the validation state is not `Ok`.
/// `Ok` items render in the UI's default text colour.
fn apply_badge_color(text: RichText, state: ValidationState) -> RichText {
    match state {
        ValidationState::Ok => text,
        ValidationState::Warning => text.color(Color32::from_rgb(220, 170, 50)),
        ValidationState::Error => text.color(Color32::from_rgb(220, 70, 70)),
    }
}
