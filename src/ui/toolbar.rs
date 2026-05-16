//! Top menu bar and toolbar strip (spec §5.5, §5.2, §4.4).
//!
//! # Layout
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ File  Edit  View  │  ⟲ Undo: …  ⟳ Redo: …  │  path > diagram  │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The left side holds drop-down menus.  The right side (right-to-left
//! layout) shows undo/redo icon buttons, the dirty-state indicator, and
//! the diagram breadcrumb.
//!
//! # File actions
//!
//! Open / Save / Save As use [`rfd`] for native file dialogs and delegate to
//! [`crate::persistence::xml`] for serialisation / deserialisation.
//!
//! # Undo / Redo
//!
//! Both menu items and icon buttons call [`crate::undo::history::UndoStack`]
//! directly.  Menu items are disabled (greyed out) when the respective stack
//! is empty.  The menu item labels include the command description
//! ("Undo: Move node") so the user knows what will be reversed.
//!
//! # Full Validate
//!
//! Calls [`crate::validation::validator::ValidationCache::validate_full`]
//! immediately and posts the result count to the status bar.

use egui::{Key, KeyboardShortcut, Modifiers, RichText, Ui};

use crate::{
    app::{ActivePanel, AppState},
    model::project::Project,
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render the top menu bar + toolbar strip into `ui`.
///
/// Called from [`crate::app::AppState::update`] inside a
/// `TopBottomPanel::top`.
pub fn show(state: &mut AppState, ui: &mut Ui) {
    egui::menu::bar(ui, |ui| {
        file_menu(state, ui);
        edit_menu(state, ui);
        view_menu(state, ui);
        right_side(state, ui);
    });

    // Process keyboard shortcuts outside the menu bar so they fire even
    // when no menu is open.
    handle_shortcuts(state, ui);
}

// ---------------------------------------------------------------------------
// File menu
// ---------------------------------------------------------------------------

fn file_menu(state: &mut AppState, ui: &mut Ui) {
    ui.menu_button("File", |ui| {
        // ── New ──────────────────────────────────────────────────────────
        if ui.add(menu_item("New Project", "Ctrl+N")).clicked() {
            action_new(state);
            ui.close_menu();
        }

        ui.separator();

        // ── Open ─────────────────────────────────────────────────────────
        if ui.add(menu_item("Open…", "Ctrl+O")).clicked() {
            action_open(state);
            ui.close_menu();
        }

        ui.separator();

        // ── Save ─────────────────────────────────────────────────────────
        let save_enabled = state.dirty || state.current_file.is_some();
        if ui
            .add_enabled(save_enabled, menu_item("Save", "Ctrl+S"))
            .clicked()
        {
            action_save(state);
            ui.close_menu();
        }

        if ui.add(menu_item("Save As…", "Ctrl+Shift+S")).clicked() {
            action_save_as(state);
            ui.close_menu();
        }

        ui.separator();

        // ── Quit ─────────────────────────────────────────────────────────
        if ui.button("Quit").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

// ---------------------------------------------------------------------------
// Edit menu
// ---------------------------------------------------------------------------

fn edit_menu(state: &mut AppState, ui: &mut Ui) {
    ui.menu_button("Edit", |ui| {
        let can_undo = state.undo_stack.can_undo();
        let can_redo = state.undo_stack.can_redo();

        // Build labels before borrowing state mutably.
        let undo_label = state
            .undo_stack
            .undo_description()
            .map(|d| format!("Undo: {d}\tCtrl+Z"))
            .unwrap_or_else(|| "Undo\tCtrl+Z".into());
        let redo_label = state
            .undo_stack
            .redo_description()
            .map(|d| format!("Redo: {d}\tCtrl+Shift+Z"))
            .unwrap_or_else(|| "Redo\tCtrl+Shift+Z".into());

        if ui
            .add_enabled(can_undo, egui::Button::new(undo_label))
            .clicked()
        {
            action_undo(state);
            ui.close_menu();
        }
        if ui
            .add_enabled(can_redo, egui::Button::new(redo_label))
            .clicked()
        {
            action_redo(state);
            ui.close_menu();
        }
    });
}

// ---------------------------------------------------------------------------
// View menu
// ---------------------------------------------------------------------------

fn view_menu(state: &mut AppState, ui: &mut Ui) {
    ui.menu_button("View", |ui| {
        // ── Panels ───────────────────────────────────────────────────────
        if ui
            .checkbox(&mut state.tree_pane_visible, "Show Tree Pane")
            .clicked()
        {
            ui.close_menu();
        }

        let dict_open = state.active_panel == ActivePanel::Dictionary;
        if ui.selectable_label(dict_open, "Data Dictionary").clicked() {
            state.active_panel = if dict_open {
                ActivePanel::None
            } else {
                ActivePanel::Dictionary
            };
            ui.close_menu();
        }

        ui.separator();

        // ── Auto Layout ──────────────────────────────────────────────────
        if ui.add(menu_item("Auto Layout", "")).clicked() {
            action_auto_layout(state);
            ui.close_menu();
        }

        ui.separator();

        // ── Full Validate ────────────────────────────────────────────────
        if ui.add(menu_item("Full Validate", "F5")).clicked() {
            action_full_validate(state);
            ui.close_menu();
        }
    });
}

// ---------------------------------------------------------------------------
// Right side: breadcrumb + undo/redo icon buttons + dirty indicator
// ---------------------------------------------------------------------------

fn right_side(state: &mut AppState, ui: &mut Ui) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // ── Undo / Redo icon buttons ──────────────────────────────────────
        let can_redo = state.undo_stack.can_redo();
        let can_undo = state.undo_stack.can_undo();

        let redo_tip = state
            .undo_stack
            .redo_description()
            .map(|d| format!("Redo: {d}  (Ctrl+Shift+Z)"))
            .unwrap_or_else(|| "Nothing to redo".into());
        let undo_tip = state
            .undo_stack
            .undo_description()
            .map(|d| format!("Undo: {d}  (Ctrl+Z)"))
            .unwrap_or_else(|| "Nothing to undo".into());

        if ui
            .add_enabled(
                can_redo,
                egui::Button::new("⟳").min_size([28.0, 0.0].into()),
            )
            .on_hover_text(redo_tip)
            .clicked()
        {
            action_redo(state);
        }
        if ui
            .add_enabled(
                can_undo,
                egui::Button::new("⟲").min_size([28.0, 0.0].into()),
            )
            .on_hover_text(undo_tip)
            .clicked()
        {
            action_undo(state);
        }

        ui.separator();

        // ── Dirty indicator ───────────────────────────────────────────────
        if state.dirty {
            ui.label(RichText::new("●").color(egui::Color32::from_rgb(255, 200, 0)))
                .on_hover_text("Unsaved changes");
            ui.separator();
        }

        // ── Breadcrumb: file name > diagram name ──────────────────────────
        // Shown right-to-left so the diagram name is closest to the buttons.
        let diagram_name = state
            .project
            .diagram(state.active_diagram_id)
            .map(|d| d.name.as_str())
            .unwrap_or("(no diagram)");

        ui.label(RichText::new(diagram_name).color(ui.visuals().strong_text_color()));

        if let Some(path) = &state.current_file {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            ui.label(
                RichText::new(format!("{file_name}  ›")).color(ui.visuals().weak_text_color()),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Keyboard shortcut handling
// ---------------------------------------------------------------------------

fn handle_shortcuts(state: &mut AppState, ui: &mut Ui) {
    let ctx = ui.ctx();

    if ctx.input_mut(|i| i.consume_shortcut(&shortcut(Modifiers::CTRL, Key::N))) {
        action_new(state);
    }
    if ctx.input_mut(|i| i.consume_shortcut(&shortcut(Modifiers::CTRL, Key::O))) {
        action_open(state);
    }
    if ctx.input_mut(|i| i.consume_shortcut(&shortcut(Modifiers::CTRL, Key::S))) {
        if state.current_file.is_some() {
            action_save(state);
        } else {
            action_save_as(state);
        }
    }
    if ctx.input_mut(|i| i.consume_shortcut(&shortcut(Modifiers::CTRL | Modifiers::SHIFT, Key::S)))
    {
        action_save_as(state);
    }
    if ctx.input_mut(|i| i.consume_shortcut(&shortcut(Modifiers::CTRL, Key::Z))) {
        action_undo(state);
    }
    // Ctrl+Shift+Z for redo (also Ctrl+Y on some platforms — add both).
    if ctx.input_mut(|i| i.consume_shortcut(&shortcut(Modifiers::CTRL | Modifiers::SHIFT, Key::Z)))
    {
        action_redo(state);
    }
    if ctx.input_mut(|i| i.consume_shortcut(&shortcut(Modifiers::CTRL, Key::Y))) {
        action_redo(state);
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F5)) {
        action_full_validate(state);
    }
}

fn shortcut(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(modifiers, key)
}

// ---------------------------------------------------------------------------
// Actions — pure logic, no UI dependencies
// ---------------------------------------------------------------------------

/// Reset to a fresh empty project.
fn action_new(state: &mut AppState) {
    state.project = Project::new();
    state.active_diagram_id = state.project.root_diagram_id;
    state.undo_stack.clear();
    state.validation.clear_all();
    state.selected_id = None;
    state.active_panel = ActivePanel::None;
    state.current_file = None;
    state.dirty = false;
    state.set_status("New project created.");
}

/// Open a project from a file chosen via a native dialog.
fn action_open(state: &mut AppState) {
    let path = rfd::FileDialog::new()
        .add_filter("DataFlow Project", &["xml"])
        .pick_file();

    let Some(path) = path else { return };

    match crate::persistence::xml::load_project_from_file(&path) {
        Ok(project) => {
            let root_id = project.root_diagram_id;
            state.project = project;
            state.active_diagram_id = root_id;
            state.undo_stack.clear();
            state.validation.validate_full(&state.project);
            state.selected_id = None;
            state.active_panel = ActivePanel::None;
            state.current_file = Some(path);
            state.dirty = false;
            state.set_status("Project opened.");
        }
        Err(e) => state.set_status(format!("Open failed: {e}")),
    }
}

/// Save to the current file path, or delegate to Save As if none is set.
fn action_save(state: &mut AppState) {
    // Clone the path to avoid a split borrow of `state`.
    let path = state.current_file.clone();
    if let Some(path) = path {
        do_save(state, &path);
    } else {
        action_save_as(state);
    }
}

/// Save to a new file path chosen via a native dialog.
fn action_save_as(state: &mut AppState) {
    let path = rfd::FileDialog::new()
        .add_filter("DataFlow Project", &["xml"])
        .set_file_name("project.xml")
        .save_file();

    let Some(path) = path else { return };

    // Remember the path first so `do_save` can use it.
    state.current_file = Some(path.clone());
    do_save(state, &path);
}

/// Write the project to `path` and update dirty/status accordingly.
fn do_save(state: &mut AppState, path: &std::path::Path) {
    match crate::persistence::xml::save_project_to_file(&state.project, path) {
        Ok(()) => {
            state.dirty = false;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            state.set_status(format!("Saved to {name}."));
        }
        Err(e) => state.set_status(format!("Save failed: {e}")),
    }
}

/// Undo the most recent command.
fn action_undo(state: &mut AppState) {
    match state.undo_stack.undo(&mut state.project) {
        Ok(()) => {
            // Re-validate the parts of the project that changed.
            state.validation.validate_full(&state.project);
        }
        Err(e) => state.set_status(format!("Undo failed: {e}")),
    }
}

/// Redo the most recently undone command.
fn action_redo(state: &mut AppState) {
    match state.undo_stack.redo(&mut state.project) {
        Ok(()) => {
            state.validation.validate_full(&state.project);
        }
        Err(e) => state.set_status(format!("Redo failed: {e}")),
    }
}

/// Run full validation across the entire project and post a status summary.
fn action_full_validate(state: &mut AppState) {
    state.validation.validate_full(&state.project);
    let has_errors = state.validation.has_errors();
    let has_warnings = state.validation.has_warnings();
    let msg = if has_errors {
        "Validation complete — errors found."
    } else if has_warnings {
        "Validation complete — warnings found."
    } else {
        "Validation complete — no issues."
    };
    state.set_status(msg);
}

/// Run auto-layout on the active diagram and push the result as a single
/// undoable compound command.
fn action_auto_layout(state: &mut AppState) {
    let cmd = {
        let Some(diagram) = state.project.diagram(state.active_diagram_id) else {
            state.set_status("No active diagram.");
            return;
        };
        crate::ui::layout::auto_layout_command(diagram)
    };

    if cmd.commands.is_empty() {
        state.set_status("Nothing to layout.");
        return;
    }

    match state.undo_stack.execute(Box::new(cmd), &mut state.project) {
        Ok(()) => {
            state.dirty = true;
            // Centre the viewport on canvas (0, 0) — where the layout is centred.
            // Transform: screen = (canvas - scroll) * zoom + origin
            // For canvas (0,0) at panel centre: scroll = -(panel_size / 2) / zoom
            if let Some(diagram) = state.project.diagram_mut(state.active_diagram_id) {
                let zoom = diagram.viewport.zoom;
                let half_w = state.last_canvas_size.x / 2.0;
                let half_h = state.last_canvas_size.y / 2.0;
                diagram.viewport.scroll = (-half_w / zoom, -half_h / zoom);
            }
            state.set_status("Auto Layout applied.");
        }
        Err(e) => state.set_status(format!("Auto Layout failed: {e}")),
    }
}

// ---------------------------------------------------------------------------
// UI helper
// ---------------------------------------------------------------------------

/// A `Button` styled as a menu item with a right-aligned shortcut hint.
///
/// Pass an empty string for `shortcut` when there is no shortcut.
fn menu_item(label: &str, shortcut: &str) -> egui::Button<'static> {
    if shortcut.is_empty() {
        egui::Button::new(label.to_owned())
    } else {
        egui::Button::new(format!("{label}\t{shortcut}"))
    }
}
