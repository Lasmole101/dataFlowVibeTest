//! Data dictionary editor panel (spec §2, §5.3).
//!
//! Rendered inside the right-hand `SidePanel` when
//! `AppState::active_panel == ActivePanel::Dictionary`.
//!
//! # Layout
//!
//! ```text
//! ┌─ Data Dictionary ────────────────── [✕] ─┐
//! │ Primitives                    [+ Primitive] │
//! │   uint32_t   [Rename]  [Delete]             │
//! │   char       [Rename]  [Delete]             │
//! │                                             │
//! │ Composites                   [+ Composite] │
//! │ ▶ MyPacket = (3×uint32_t, 2×char)          │
//! │   [Rename]  [Delete]                        │
//! │                                             │
//! │ ┌─ Add Composite ──────────────────────┐   │
//! │ │ Name: [__________]                   │   │
//! │ │ Entries:                             │   │
//! │ │   [3] [uint32_t ▾]  [−]             │   │
//! │ │   [2] [char ▾]      [−]             │   │
//! │ │   [+ Add entry]                     │   │
//! │ │   [Create]  [Cancel]                │   │
//! │ └──────────────────────────────────────┘   │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! # Command dispatch
//!
//! All mutations go through the undo stack:
//! * Add primitive → [`AddPrimitiveType`]
//! * Add composite → [`AddCompositeType`]
//! * Rename → [`RenameType`] (inline edit mode, Enter to confirm)
//! * Delete → [`DeleteType`] (preceded by inline "in-use" warning if applicable)

use egui::{Color32, RichText, Ui};

use crate::{
    app::{ActivePanel, AppState},
    model::types::{CompositeType, PrimitiveType, TypeEntry, TypeSpec},
    undo::commands::{AddCompositeType, AddPrimitiveType, DeleteType, RenameType},
};

// ---------------------------------------------------------------------------
// Panel state  (stored on AppState via DialogState::dictionary)
// ---------------------------------------------------------------------------

/// One entry in the composite-builder form.
#[derive(Debug, Clone)]
pub struct BuilderEntry {
    pub count: u32,
    pub type_ref: String,
}

/// The mode the "add primitive" row is in.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum AddPrimitiveMode {
    #[default]
    Idle,
    Editing,
}

/// The mode the composite builder form is in.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum CompositeBuilderMode {
    #[default]
    Idle,
    Open,
}

/// State that persists across frames for the dictionary panel.
#[derive(Debug, Default)]
pub struct DictionaryPanelState {
    // ── Add primitive ──────────────────────────────────────────────────────
    pub add_primitive_mode: AddPrimitiveMode,
    pub add_primitive_buf: String,
    pub add_primitive_error: Option<String>,

    // ── Add composite ──────────────────────────────────────────────────────
    pub composite_builder_mode: CompositeBuilderMode,
    pub composite_name_buf: String,
    pub composite_entries: Vec<BuilderEntry>,
    pub composite_error: Option<String>,

    // ── Inline rename ──────────────────────────────────────────────────────
    /// The type name currently being renamed in-place, and the edit buffer.
    pub rename_target: Option<String>,
    pub rename_buf: String,
    pub rename_error: Option<String>,

    // ── Delete confirmation ────────────────────────────────────────────────
    /// Name of the type for which a delete confirmation is pending.
    pub pending_delete: Option<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render the dictionary editor panel into `ui`.
pub fn show(state: &mut AppState, ui: &mut Ui) {
    // Header row.
    ui.horizontal(|ui| {
        ui.strong("Data Dictionary");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✕").clicked() {
                state.active_panel = ActivePanel::None;
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            show_primitives(state, ui);
            ui.add_space(8.0);
            show_composites(state, ui);
        });
}

// ---------------------------------------------------------------------------
// Primitives section
// ---------------------------------------------------------------------------

fn show_primitives(state: &mut AppState, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.strong("Primitives");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+ Primitive").clicked() {
                let ps = &mut state.dialogs.dictionary;
                ps.add_primitive_mode = AddPrimitiveMode::Editing;
                ps.add_primitive_buf.clear();
                ps.add_primitive_error = None;
            }
        });
    });

    // Snapshot names to avoid borrow conflicts while rendering rename/delete.
    let primitive_names: Vec<String> = state
        .project
        .data_dictionary
        .primitives
        .iter()
        .map(|p| p.name().to_owned())
        .collect();

    for name in &primitive_names {
        show_type_row(state, ui, name, true);
    }

    // "Add primitive" inline input.
    if state.dialogs.dictionary.add_primitive_mode == AddPrimitiveMode::Editing {
        ui.horizontal(|ui| {
            let resp = ui.text_edit_singleline(&mut state.dialogs.dictionary.add_primitive_buf);
            resp.request_focus();

            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));

            if ui.small_button("Add").clicked() || enter {
                let name = state.dialogs.dictionary.add_primitive_buf.trim().to_owned();
                if name.is_empty() {
                    state.dialogs.dictionary.add_primitive_error =
                        Some("Name cannot be empty.".into());
                } else if state.project.data_dictionary.contains(&name) {
                    state.dialogs.dictionary.add_primitive_error =
                        Some(format!("'{name}' is already in use."));
                } else {
                    let cmd = AddPrimitiveType {
                        primitive: PrimitiveType::new(&name),
                    };
                    if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                        state.dialogs.dictionary.add_primitive_error = Some(e.to_string());
                    } else {
                        state.dirty = true;
                        state.dialogs.dictionary.add_primitive_mode = AddPrimitiveMode::Idle;
                        state.dialogs.dictionary.add_primitive_buf.clear();
                        state.dialogs.dictionary.add_primitive_error = None;
                    }
                }
            }
            if ui.small_button("Cancel").clicked() || esc {
                state.dialogs.dictionary.add_primitive_mode = AddPrimitiveMode::Idle;
                state.dialogs.dictionary.add_primitive_error = None;
            }
        });
        if let Some(ref err) = state.dialogs.dictionary.add_primitive_error.clone() {
            ui.label(RichText::new(err).color(Color32::from_rgb(220, 70, 70)));
        }
    }
}

// ---------------------------------------------------------------------------
// Composites section
// ---------------------------------------------------------------------------

fn show_composites(state: &mut AppState, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.strong("Composites");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+ Composite").clicked() {
                let ps = &mut state.dialogs.dictionary;
                ps.composite_builder_mode = CompositeBuilderMode::Open;
                ps.composite_name_buf.clear();
                ps.composite_entries.clear();
                ps.composite_error = None;
            }
        });
    });

    // Snapshot composite names.
    let composite_names: Vec<String> = state
        .project
        .data_dictionary
        .composites
        .iter()
        .map(|c| c.name().to_owned())
        .collect();

    for name in &composite_names {
        // Show a one-line definition summary.
        let summary = composite_summary(state, name);
        ui.horizontal(|ui| {
            ui.label(format!("▶ {name}"));
            ui.label(
                RichText::new(format!("= {summary}"))
                    .color(Color32::GRAY)
                    .small(),
            );
        });
        show_type_row(state, ui, name, false);
        ui.add_space(2.0);
    }

    // Composite builder form.
    if state.dialogs.dictionary.composite_builder_mode == CompositeBuilderMode::Open {
        show_composite_builder(state, ui);
    }
}

/// Render the inline composite builder sub-form.
fn show_composite_builder(state: &mut AppState, ui: &mut Ui) {
    ui.separator();
    ui.strong("New Composite");

    // Name field.
    ui.horizontal(|ui| {
        ui.label("Name:");
        let resp = ui.text_edit_singleline(&mut state.dialogs.dictionary.composite_name_buf);
        if state.dialogs.dictionary.composite_entries.is_empty() {
            resp.request_focus();
        }
    });

    // Error message.
    if let Some(ref err) = state.dialogs.dictionary.composite_error.clone() {
        ui.label(RichText::new(err).color(Color32::from_rgb(220, 70, 70)));
    }

    // Entry list.
    ui.label("Entries:");

    // Collect all available type names for the picker.
    let all_names: Vec<String> = state
        .project
        .data_dictionary
        .all_names()
        .map(|s| s.to_owned())
        .collect();

    // We need indices for deletion — collect a list of indices to remove.
    let mut to_remove: Option<usize> = None;
    let entry_count = state.dialogs.dictionary.composite_entries.len();

    for i in 0..entry_count {
        ui.horizontal(|ui| {
            // Count control.
            let count = &mut state.dialogs.dictionary.composite_entries[i].count;
            if ui.small_button("−").clicked() && *count > 1 {
                *count -= 1;
            }
            ui.add(egui::DragValue::new(count).range(1..=9999).speed(0.5));
            if ui.small_button("+").clicked() {
                *count += 1;
            }

            // Type picker (combobox).
            let current = state.dialogs.dictionary.composite_entries[i]
                .type_ref
                .clone();
            egui::ComboBox::from_id_salt(format!("comp_entry_{i}"))
                .selected_text(&current)
                .width(120.0)
                .show_ui(ui, |ui| {
                    for name in &all_names {
                        ui.selectable_value(
                            &mut state.dialogs.dictionary.composite_entries[i].type_ref,
                            name.clone(),
                            name,
                        );
                    }
                });

            // Remove entry button.
            if ui.small_button("✕").clicked() {
                to_remove = Some(i);
            }
        });
    }

    if let Some(idx) = to_remove {
        state.dialogs.dictionary.composite_entries.remove(idx);
    }

    // Add entry button.
    if ui.small_button("+ Add entry").clicked() {
        let default_type = state
            .project
            .data_dictionary
            .all_names()
            .next()
            .unwrap_or("")
            .to_owned();
        state
            .dialogs
            .dictionary
            .composite_entries
            .push(BuilderEntry {
                count: 1,
                type_ref: default_type,
            });
    }

    ui.horizontal(|ui| {
        if ui.button("Create").clicked() {
            try_create_composite(state);
        }
        if ui.button("Cancel").clicked() {
            state.dialogs.dictionary.composite_builder_mode = CompositeBuilderMode::Idle;
            state.dialogs.dictionary.composite_error = None;
        }
    });
    ui.separator();
}

/// Attempt to submit the composite builder form as an `AddCompositeType` command.
fn try_create_composite(state: &mut AppState) {
    let name = state
        .dialogs
        .dictionary
        .composite_name_buf
        .trim()
        .to_owned();

    if name.is_empty() {
        state.dialogs.dictionary.composite_error = Some("Name cannot be empty.".into());
        return;
    }
    if state.project.data_dictionary.contains(&name) {
        state.dialogs.dictionary.composite_error = Some(format!("'{name}' is already in use."));
        return;
    }
    if state.dialogs.dictionary.composite_entries.is_empty() {
        state.dialogs.dictionary.composite_error =
            Some("A composite must have at least one entry.".into());
        return;
    }

    // Validate all type refs exist.
    for entry in &state.dialogs.dictionary.composite_entries {
        if entry.type_ref.is_empty() || !state.project.data_dictionary.contains(&entry.type_ref) {
            state.dialogs.dictionary.composite_error =
                Some(format!("Unknown type '{}'.", entry.type_ref));
            return;
        }
    }

    // Build the TypeSpec.
    let mut spec = TypeSpec::new();
    for entry in &state.dialogs.dictionary.composite_entries {
        spec.add(entry.count, &entry.type_ref);
    }

    let composite = CompositeType::new(&name, spec);
    let cmd = AddCompositeType { composite };

    match state.undo_stack.execute(Box::new(cmd), &mut state.project) {
        Ok(()) => {
            state.dirty = true;
            state.dialogs.dictionary.composite_builder_mode = CompositeBuilderMode::Idle;
            state.dialogs.dictionary.composite_error = None;
            state.dialogs.dictionary.composite_name_buf.clear();
            state.dialogs.dictionary.composite_entries.clear();
        }
        Err(e) => {
            state.dialogs.dictionary.composite_error = Some(e.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Shared type row (rename / delete controls)
// ---------------------------------------------------------------------------

/// Render the rename and delete controls for one type entry.
///
/// `is_primitive` controls which snapshot type is built for `DeleteType`.
fn show_type_row(state: &mut AppState, ui: &mut Ui, name: &str, is_primitive: bool) {
    // ── Inline rename mode ────────────────────────────────────────────────
    let is_renaming = state.dialogs.dictionary.rename_target.as_deref() == Some(name);

    if is_renaming {
        ui.horizontal(|ui| {
            let resp = ui.text_edit_singleline(&mut state.dialogs.dictionary.rename_buf);
            resp.request_focus();

            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));

            let can_confirm = !state.dialogs.dictionary.rename_buf.trim().is_empty();
            if (ui
                .add_enabled(can_confirm, egui::Button::new("OK"))
                .clicked()
                || enter)
                && can_confirm
            {
                let new_name = state.dialogs.dictionary.rename_buf.trim().to_owned();
                let old_name = name.to_owned();
                if new_name == old_name {
                    // No-op rename — just close.
                    state.dialogs.dictionary.rename_target = None;
                    state.dialogs.dictionary.rename_error = None;
                } else if state.project.data_dictionary.contains(&new_name) {
                    state.dialogs.dictionary.rename_error =
                        Some(format!("'{new_name}' is already in use."));
                } else {
                    let cmd = RenameType {
                        old_name: old_name.clone(),
                        new_name: new_name.clone(),
                        patched_composites: vec![],
                    };
                    match state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                        Ok(()) => {
                            state.dirty = true;
                            // Notify validation cache of the rename.
                            state
                                .validation
                                .on_type_renamed(&old_name, &new_name, &state.project);
                            state.dialogs.dictionary.rename_target = None;
                            state.dialogs.dictionary.rename_error = None;
                        }
                        Err(e) => {
                            state.dialogs.dictionary.rename_error = Some(e.to_string());
                        }
                    }
                }
            }
            if ui.small_button("Cancel").clicked() || esc {
                state.dialogs.dictionary.rename_target = None;
                state.dialogs.dictionary.rename_error = None;
            }
        });

        if let Some(ref err) = state.dialogs.dictionary.rename_error.clone() {
            ui.label(RichText::new(err).color(Color32::from_rgb(220, 70, 70)));
        }
        return;
    }

    // ── Normal display row ────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(format!("  {name}"));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // ── Delete button ────────────────────────────────────────────
            let is_pending_delete =
                state.dialogs.dictionary.pending_delete.as_deref() == Some(name);

            if is_pending_delete {
                // Show confirmation inline.
                if ui.small_button("Confirm").clicked() {
                    do_delete(state, name, is_primitive);
                    state.dialogs.dictionary.pending_delete = None;
                }
                if ui.small_button("Cancel").clicked() {
                    state.dialogs.dictionary.pending_delete = None;
                }
                ui.label(
                    RichText::new("In use — delete anyway?")
                        .color(Color32::from_rgb(220, 130, 50))
                        .small(),
                );
            } else if ui.small_button("Delete").clicked() {
                // Check if the type is in use anywhere.
                if is_type_in_use(&state.project, name) {
                    state.dialogs.dictionary.pending_delete = Some(name.to_owned());
                } else {
                    do_delete(state, name, is_primitive);
                }
            }

            // ── Rename button ─────────────────────────────────────────────
            if ui.small_button("Rename").clicked() {
                state.dialogs.dictionary.rename_target = Some(name.to_owned());
                state.dialogs.dictionary.rename_buf = name.to_owned();
                state.dialogs.dictionary.rename_error = None;
                // Cancel any pending delete for another type.
                state.dialogs.dictionary.pending_delete = None;
            }
        });
    });
}

/// Execute a `DeleteType` command and notify the validation cache.
fn do_delete(state: &mut AppState, name: &str, is_primitive: bool) {
    let cmd: Box<dyn crate::undo::history::Command> = if is_primitive {
        let prim = state
            .project
            .data_dictionary
            .primitive(name)
            .cloned()
            .unwrap_or_else(|| PrimitiveType::new(name));
        Box::new(DeleteType::primitive(prim))
    } else {
        let comp = state
            .project
            .data_dictionary
            .composite(name)
            .cloned()
            .unwrap_or_else(|| CompositeType::new(name, TypeSpec::new()));
        Box::new(DeleteType::composite(comp))
    };

    // Notify validation BEFORE deleting so the cache can still scan specs.
    state.validation.on_type_deleted(name, &state.project);

    match state.undo_stack.execute(cmd, &mut state.project) {
        Ok(()) => {
            state.dirty = true;
            state.set_status(format!("Deleted type '{name}'."));
        }
        Err(e) => {
            state.set_status(format!("Delete failed: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` if `name` appears in any node or stream spec in any diagram.
fn is_type_in_use(project: &crate::model::project::Project, name: &str) -> bool {
    for diagram in project.diagrams.values() {
        for node in diagram.nodes.values() {
            if node.input_spec.entries.iter().any(|e| e.type_ref == name)
                || node.output_spec.entries.iter().any(|e| e.type_ref == name)
            {
                return true;
            }
        }
        for stream in diagram.streams.values() {
            if stream.type_spec.entries.iter().any(|e| e.type_ref == name) {
                return true;
            }
        }
        // Also check composite definitions that reference this type.
        for comp in &project.data_dictionary.composites {
            if comp.spec.entries.iter().any(|e| e.type_ref == name) {
                return true;
            }
        }
    }
    false
}

/// Build a compact one-line summary of a composite's definition.
fn composite_summary(state: &AppState, name: &str) -> String {
    let Some(comp) = state.project.data_dictionary.composite(name) else {
        return "(missing)".into();
    };
    let parts: Vec<String> = comp
        .spec
        .entries
        .iter()
        .map(|e| format!("{}×{}", e.count, e.type_ref))
        .collect();
    if parts.is_empty() {
        "(empty)".into()
    } else {
        parts.join(", ")
    }
}
