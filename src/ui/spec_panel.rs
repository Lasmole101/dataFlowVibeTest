//! Type specification panel (spec §5.3).
//!
//! Opens in the right-hand `SidePanel` when the user selects a node or stream
//! on the canvas.  Shows the component's name, kind, and type specification
//! entries with controls to increment, decrement, and add types.
//!
//! # Spec sections shown per component type
//!
//! | Component      | Input section | Output section |
//! |----------------|---------------|----------------|
//! | Source node    | Hidden        | Shown          |
//! | Sink node      | Shown         | Hidden         |
//! | Process node   | Shown         | Shown          |
//! | Stream         | —             | Single section |
//! | Boundary node  | Read-only     | Read-only      |
//!
//! # Command dispatch
//!
//! * Rename → [`RenameNode`] (on Enter in the name field)
//! * `+` / `−` on an entry → [`EditNodeSpec`] or [`EditStreamSpec`]
//! * Add type → same commands, with merged count if type already present
//!
//! Each `+` / `−` click dispatches a command immediately so every count change
//! is individually undoable (spec §5.5).

use egui::{Color32, RichText, Ui};
use uuid::Uuid;

use crate::{
    app::{ActivePanel, AppState},
    model::{node::NodeKind, types::TypeSpec},
    undo::commands::{EditNodeSpec, EditStreamSpec, NodeSpecSide, RenameNode},
};

// ---------------------------------------------------------------------------
// Panel state  (stored on AppState via DialogState::spec)
// ---------------------------------------------------------------------------

/// Persistent state for the spec panel across frames.
#[derive(Debug, Default)]
pub struct SpecPanelState {
    /// The UUID this state belongs to. When the selection changes we reset.
    pub current_id: Option<Uuid>,
    /// Editable buffer for the component name.
    pub name_buf: String,
    /// Whether the name field is in edit mode.
    pub name_editing: bool,
    /// Error shown below the name field.
    pub name_error: Option<String>,
    /// Whether the "Add type" picker for the input spec is open.
    pub input_picker_open: bool,
    /// Whether the "Add type" picker for the output / stream spec is open.
    pub output_picker_open: bool,
}

impl SpecPanelState {
    /// Reset state when the selected component changes.
    pub fn reset(&mut self, new_id: Uuid, new_name: &str) {
        self.current_id = Some(new_id);
        self.name_buf = new_name.to_owned();
        self.name_editing = false;
        self.name_error = None;
        self.input_picker_open = false;
        self.output_picker_open = false;
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render the spec editor for `target_id` (a node or stream UUID).
pub fn show(state: &mut AppState, target_id: Uuid, ui: &mut Ui) {
    // ── Header ────────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.strong("Spec Editor");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").clicked() {
                state.active_panel = ActivePanel::None;
            }
        });
    });
    ui.separator();

    // ── Locate the component ─────────────────────────────────────────────
    // Find the diagram and determine whether it's a node or stream.
    let locate = locate_component(state, target_id);

    match locate {
        None => {
            ui.label(RichText::new("Component not found.").color(Color32::from_rgb(220, 70, 70)));
            return;
        }
        Some(Location::Node { diagram_id, .. }) => {
            show_node_panel(state, ui, target_id, diagram_id);
        }
        Some(Location::Stream { diagram_id }) => {
            show_stream_panel(state, ui, target_id, diagram_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Location helper
// ---------------------------------------------------------------------------

enum Location {
    Node { diagram_id: Uuid },
    Stream { diagram_id: Uuid },
}

fn locate_component(state: &AppState, target_id: Uuid) -> Option<Location> {
    for (&did, diagram) in &state.project.diagrams {
        if diagram.nodes.contains_key(&target_id) {
            return Some(Location::Node { diagram_id: did });
        }
        if diagram.streams.contains_key(&target_id) {
            return Some(Location::Stream { diagram_id: did });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Node panel
// ---------------------------------------------------------------------------

fn show_node_panel(state: &mut AppState, ui: &mut Ui, node_id: Uuid, diagram_id: Uuid) {
    // Snapshot node data before any mutable borrows.
    let (name, kind, is_boundary, input_spec, output_spec) = {
        let node = &state.project.diagrams[&diagram_id].nodes[&node_id];
        (
            node.name.clone(),
            node.kind,
            node.is_boundary,
            node.input_spec.clone(),
            node.output_spec.clone(),
        )
    };

    // ── Reset state when selection changes ───────────────────────────────
    let needs_reset = state.dialogs.spec.current_id != Some(node_id);
    if needs_reset {
        state.dialogs.spec.reset(node_id, &name);
    }

    // ── Name row ─────────────────────────────────────────────────────────
    show_name_row(state, ui, node_id, diagram_id, &name, is_boundary, true);

    // ── Kind badge ───────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label("Kind:");
        let kind_color = match kind {
            NodeKind::Source => Color32::from_rgb(80, 160, 80),
            NodeKind::Sink => Color32::from_rgb(160, 80, 80),
            NodeKind::Process => Color32::from_rgb(80, 120, 200),
        };
        ui.label(RichText::new(kind.label()).color(kind_color).strong());
        if is_boundary {
            ui.label(RichText::new("(boundary)").color(Color32::GRAY).small());
        }
    });

    ui.separator();

    // ── Input spec section ────────────────────────────────────────────────
    // Shown for Sink and Process nodes (not Source).
    let show_input = matches!(kind, NodeKind::Sink | NodeKind::Process);
    if show_input {
        ui.strong("Input spec");
        show_spec_entries(
            state,
            ui,
            SpecTarget::NodeInput {
                node_id,
                diagram_id,
            },
            &input_spec,
            is_boundary,
        );
        ui.add_space(6.0);
    }

    // ── Output spec section ───────────────────────────────────────────────
    // Shown for Source and Process nodes (not Sink).
    let show_output = matches!(kind, NodeKind::Source | NodeKind::Process);
    if show_output {
        ui.strong("Output spec");
        show_spec_entries(
            state,
            ui,
            SpecTarget::NodeOutput {
                node_id,
                diagram_id,
            },
            &output_spec,
            is_boundary,
        );
    }
}

// ---------------------------------------------------------------------------
// Stream panel
// ---------------------------------------------------------------------------

fn show_stream_panel(state: &mut AppState, ui: &mut Ui, stream_id: Uuid, diagram_id: Uuid) {
    let type_spec = state.project.diagrams[&diagram_id].streams[&stream_id]
        .type_spec
        .clone();

    // Streams don't have a name — show their UUID as a read-only label.
    let needs_reset = state.dialogs.spec.current_id != Some(stream_id);
    if needs_reset {
        state.dialogs.spec.reset(stream_id, "");
    }

    ui.horizontal(|ui| {
        ui.label("Stream");
        ui.label(
            RichText::new(format!("({}…)", &stream_id.hyphenated().to_string()[..8]))
                .color(Color32::GRAY)
                .small(),
        );
    });
    ui.separator();

    ui.strong("Type spec");
    show_spec_entries(
        state,
        ui,
        SpecTarget::Stream {
            stream_id,
            diagram_id,
        },
        &type_spec,
        false, // streams are never read-only
    );
}

// ---------------------------------------------------------------------------
// Name row (editable for non-boundary nodes)
// ---------------------------------------------------------------------------

fn show_name_row(
    state: &mut AppState,
    ui: &mut Ui,
    node_id: Uuid,
    diagram_id: Uuid,
    current_name: &str,
    read_only: bool,
    is_node: bool,
) {
    ui.horizontal(|ui| {
        ui.label("Name:");

        if read_only {
            ui.label(current_name);
            return;
        }

        if state.dialogs.spec.name_editing {
            // ── Edit mode ─────────────────────────────────────────────────
            let resp = ui.text_edit_singleline(&mut state.dialogs.spec.name_buf);
            resp.request_focus();

            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));

            if enter {
                let new_name = state.dialogs.spec.name_buf.trim().to_owned();
                if new_name.is_empty() {
                    state.dialogs.spec.name_error = Some("Name cannot be empty.".into());
                } else if new_name != current_name && is_node {
                    let cmd = RenameNode {
                        diagram_id,
                        node_id,
                        old_name: current_name.to_owned(),
                        new_name,
                    };
                    if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                        state.dialogs.spec.name_error = Some(e.to_string());
                    } else {
                        state.dirty = true;
                        state.dialogs.spec.name_editing = false;
                        state.dialogs.spec.name_error = None;
                    }
                } else {
                    state.dialogs.spec.name_editing = false;
                    state.dialogs.spec.name_error = None;
                }
            }
            if esc {
                state.dialogs.spec.name_buf = current_name.to_owned();
                state.dialogs.spec.name_editing = false;
                state.dialogs.spec.name_error = None;
            }
        } else {
            // ── Display mode ──────────────────────────────────────────────
            ui.label(current_name);
            if ui.small_button("✎").on_hover_text("Rename").clicked() {
                state.dialogs.spec.name_buf = current_name.to_owned();
                state.dialogs.spec.name_editing = true;
                state.dialogs.spec.name_error = None;
            }
        }
    });

    // Inline name error.
    if let Some(ref err) = state.dialogs.spec.name_error.clone() {
        ui.label(
            RichText::new(err)
                .color(Color32::from_rgb(220, 70, 70))
                .small(),
        );
    }
}

// ---------------------------------------------------------------------------
// Spec entry list
// ---------------------------------------------------------------------------

/// Identifies which spec is being edited — drives command construction.
#[derive(Clone, Copy)]
enum SpecTarget {
    NodeInput { node_id: Uuid, diagram_id: Uuid },
    NodeOutput { node_id: Uuid, diagram_id: Uuid },
    Stream { stream_id: Uuid, diagram_id: Uuid },
}

/// Render the list of `(count, type_name)` entries for one spec section.
///
/// `read_only` suppresses all +/− and Add controls for boundary nodes.
fn show_spec_entries(
    state: &mut AppState,
    ui: &mut Ui,
    target: SpecTarget,
    spec: &TypeSpec,
    read_only: bool,
) {
    if spec.is_empty() && read_only {
        ui.label(RichText::new("(empty)").color(Color32::GRAY).small());
        return;
    }

    // Collect all type names available for the picker.
    let all_names: Vec<String> = state
        .project
        .data_dictionary
        .all_names()
        .map(|s| s.to_owned())
        .collect();

    // Track which entry index to remove (−) or increment (+).
    let mut action: Option<SpecAction> = None;

    for (i, entry) in spec.entries.iter().enumerate() {
        ui.horizontal(|ui| {
            // Type name label.
            ui.label(&entry.type_ref);

            if !read_only {
                // − button.
                if ui.small_button("−").clicked() {
                    action = Some(SpecAction::Decrement(i));
                }

                // Count display.
                ui.label(format!("{}", entry.count));

                // + button.
                if ui.small_button("+").clicked() {
                    action = Some(SpecAction::Increment(i));
                }
            } else {
                // Read-only: show count without controls.
                ui.label(RichText::new(format!("× {}", entry.count)).color(Color32::GRAY));
            }
        });
    }

    if read_only {
        return;
    }

    // ── Add type picker ───────────────────────────────────────────────────
    // The picker-open state is stored by section (input vs output/stream).
    let picker_open: &mut bool = match target {
        SpecTarget::NodeInput { .. } => &mut state.dialogs.spec.input_picker_open,
        _ => &mut state.dialogs.spec.output_picker_open,
    };

    if all_names.is_empty() {
        ui.label(
            RichText::new("(no types in dictionary)")
                .color(Color32::GRAY)
                .small(),
        );
    } else {
        ui.horizontal(|ui| {
            if ui.small_button("Add type…").clicked() {
                *picker_open = !*picker_open;
            }
        });

        if *picker_open {
            ui.indent("type_picker", |ui| {
                for name in &all_names {
                    if ui.small_button(name).clicked() {
                        action = Some(SpecAction::Add(name.clone()));
                        *picker_open = false;
                    }
                }
                if ui.small_button("✕ Close").clicked() {
                    *picker_open = false;
                }
            });
        }
    }

    // ── Apply action ──────────────────────────────────────────────────────
    if let Some(act) = action {
        apply_spec_action(state, target, spec, act);
    }
}

// ---------------------------------------------------------------------------
// Spec action application
// ---------------------------------------------------------------------------

enum SpecAction {
    Increment(usize),
    Decrement(usize),
    Add(String),
}

/// Build the "after" spec from the "before" spec and the action, then dispatch
/// the appropriate command.
fn apply_spec_action(
    state: &mut AppState,
    target: SpecTarget,
    before_spec: &TypeSpec,
    action: SpecAction,
) {
    let mut after = before_spec.clone();

    match action {
        SpecAction::Increment(i) => {
            if let Some(entry) = after.entries.get_mut(i) {
                entry.count += 1;
            }
        }
        SpecAction::Decrement(i) => {
            if let Some(entry) = after.entries.get(i).cloned() {
                if entry.count <= 1 {
                    after.entries.remove(i);
                } else {
                    after.entries[i].count -= 1;
                }
            }
        }
        SpecAction::Add(type_name) => {
            // Merge if the type already exists (spec §5.3: "same type added
            // twice, counts are merged into a single entry").
            after.add(1, &type_name);
        }
    }

    // Dispatch the appropriate command.
    let result = match target {
        SpecTarget::NodeInput {
            node_id,
            diagram_id,
        } => state.undo_stack.execute(
            Box::new(EditNodeSpec {
                diagram_id,
                node_id,
                side: NodeSpecSide::Input,
                before: before_spec.clone(),
                after,
            }),
            &mut state.project,
        ),
        SpecTarget::NodeOutput {
            node_id,
            diagram_id,
        } => state.undo_stack.execute(
            Box::new(EditNodeSpec {
                diagram_id,
                node_id,
                side: NodeSpecSide::Output,
                before: before_spec.clone(),
                after,
            }),
            &mut state.project,
        ),
        SpecTarget::Stream {
            stream_id,
            diagram_id,
        } => state.undo_stack.execute(
            Box::new(EditStreamSpec {
                diagram_id,
                stream_id,
                before: before_spec.clone(),
                after,
            }),
            &mut state.project,
        ),
    };

    match result {
        Ok(()) => {
            state.dirty = true;
            // A node-spec edit may cascade into a sub-diagram's boundary
            // nodes (see EditNodeSpec::apply), which live in a different
            // diagram than the one incremental validation scopes to — so
            // fall back to a full re-validation for those.
            let cascades_to_subdiagram = match target {
                SpecTarget::NodeInput { node_id, diagram_id }
                | SpecTarget::NodeOutput { node_id, diagram_id } => state
                    .project
                    .diagram(diagram_id)
                    .and_then(|d| d.nodes.get(&node_id))
                    .is_some_and(|n| n.sub_diagram_id.is_some()),
                SpecTarget::Stream { .. } => false,
            };
            if cascades_to_subdiagram {
                state.validation.validate_full(&state.project);
            } else {
                // Incremental validation: re-validate the changed component
                // and its neighbours.
                state.validation.validate_incremental(
                    match target {
                        SpecTarget::NodeInput { node_id, .. } => node_id,
                        SpecTarget::NodeOutput { node_id, .. } => node_id,
                        SpecTarget::Stream { stream_id, .. } => stream_id,
                    },
                    &state.project,
                );
            }
        }
        Err(e) => {
            state.set_status(format!("Spec edit failed: {e}"));
        }
    }
}
