//! Canvas drawing and interaction layer (spec §5.1, phases 8a–8d).
//!
//! Renders the active diagram's nodes, streams, and validation badges using
//! the egui [`Painter`] API.
//!
//! # Coordinate systems
//!
//! * **Canvas space** — the coordinate system stored in
//!   [`Diagram::positions`].  Origin is arbitrary; units are logical pixels
//!   at zoom 1.0.
//! * **Screen space** — pixels relative to the egui panel's top-left corner
//!   (the `Painter`'s clip rect origin).
//!
//! Transform: `screen = (canvas - scroll) * zoom + panel_origin`
//! Inverse:   `canvas = (screen - panel_origin) / zoom + scroll`
//!
//! # Interaction priority (phase 8d)
//!
//! Within a single `drag_started()` frame, the following checks run in order:
//! 1. Output handle hit → start stream creation
//! 2. Stream endpoint hit (if stream selected) → start reconnect
//! 3. Node body hit (not near a handle) → start node drag
//! Palette drag is detected separately in the floating palette window.

use std::collections::{HashMap, HashSet};

use egui::{Color32, CursorIcon, FontId, Painter, Pos2, Rect, Rounding, Stroke, Vec2};
use uuid::Uuid;

use crate::{
    app::{ActivePanel, AppState, ContextTarget},
    model::{
        diagram::Diagram,
        node::{Node, NodeKind},
        stream::Stream,
    },
    undo::{
        commands::{
            AddNode, AddStream, CreateSubDiagram, DeleteNode, DeleteStream, DeleteStreamGroup,
            EditStreamGroup, MoveNode,
        },
        history::CompoundCommand,
    },
    validation::validator::ValidationState,
};

// ---------------------------------------------------------------------------
// Layout constants (canvas units)
// ---------------------------------------------------------------------------

/// Width of a node rectangle in canvas units.
pub const NODE_W: f32 = 120.0;
/// Height of a node rectangle in canvas units.
pub const NODE_H: f32 = 60.0;
/// Corner radius of a node rectangle.
const NODE_ROUNDING: f32 = 6.0;
/// Thickness of the node border stroke.
const NODE_STROKE: f32 = 1.5;
/// Dashed segment length for boundary nodes.
const DASH_LEN: f32 = 6.0;
/// Gap between dashes.
const DASH_GAP: f32 = 4.0;

/// Stream line thickness.
const STREAM_STROKE: f32 = 1.5;
/// Arrowhead size (half-width of the arrowhead triangle).
const ARROW_HALF: f32 = 6.0;
/// Perpendicular offset per slot for grouped streams.
const GROUP_OFFSET: f32 = 7.5;

/// Badge circle radius.
const BADGE_R: f32 = 6.0;
/// Amber colour for warnings.
const AMBER: Color32 = Color32::from_rgb(220, 170, 50);
/// Red colour for errors.
const ERROR_RED: Color32 = Color32::from_rgb(220, 70, 70);
/// Selection highlight colour.
const SELECT_COLOR: Color32 = Color32::from_rgb(80, 180, 255);
/// Multi-selection highlight colour (amber, used when streams are Ctrl+clicked for grouping).
const MULTI_SELECT_COLOR: Color32 = Color32::from_rgb(255, 170, 50);
/// Tolerance (screen pixels) for clicking a stream polyline.
const STREAM_HIT_TOLERANCE: f32 = 6.0;
/// Zoom step per one scroll-wheel notch (120 scroll units).
const ZOOM_NOTCH: f32 = 120.0;
/// Multiplicative zoom change per scroll notch.
const ZOOM_FACTOR_PER_NOTCH: f32 = 1.15;
/// Hard zoom bounds.
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 20.0;
/// Grid snap size in canvas units (applied on node drop).
const SNAP_GRID: f32 = 16.0;
/// Radius of connection handle circles in screen pixels.
const HANDLE_R: f32 = 5.0;
/// Hit radius for connection handle detection (larger than visual for easier clicking).
const HANDLE_HIT_R: f32 = 8.0;

// ---------------------------------------------------------------------------
// Node colours
// ---------------------------------------------------------------------------

fn node_fill(kind: NodeKind, dark_mode: bool) -> Color32 {
    if dark_mode {
        match kind {
            NodeKind::Source => Color32::from_rgb(30, 80, 50),
            NodeKind::Sink => Color32::from_rgb(80, 30, 40),
            NodeKind::Process => Color32::from_rgb(30, 50, 90),
        }
    } else {
        match kind {
            NodeKind::Source => Color32::from_rgb(200, 235, 210),
            NodeKind::Sink => Color32::from_rgb(240, 210, 210),
            NodeKind::Process => Color32::from_rgb(210, 225, 245),
        }
    }
}

fn node_stroke_color(kind: NodeKind, dark_mode: bool) -> Color32 {
    if dark_mode {
        match kind {
            NodeKind::Source => Color32::from_rgb(70, 180, 100),
            NodeKind::Sink => Color32::from_rgb(200, 80, 80),
            NodeKind::Process => Color32::from_rgb(80, 140, 220),
        }
    } else {
        match kind {
            NodeKind::Source => Color32::from_rgb(40, 140, 80),
            NodeKind::Sink => Color32::from_rgb(160, 50, 50),
            NodeKind::Process => Color32::from_rgb(50, 100, 180),
        }
    }
}

fn stream_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(180, 180, 180)
    } else {
        Color32::from_rgb(80, 80, 80)
    }
}

// ---------------------------------------------------------------------------
// Connection handle position helpers
// ---------------------------------------------------------------------------

/// Screen-space position of the output (right-edge) connection handle, or `None`
/// if the node kind cannot produce outputs.
fn output_handle_pos(rect: Rect, kind: NodeKind) -> Option<Pos2> {
    if kind.produces_outputs() {
        Some(Pos2::new(rect.max.x, rect.center().y))
    } else {
        None
    }
}

/// Screen-space position of the input (left-edge) connection handle, or `None`
/// if the node kind cannot accept inputs.
fn input_handle_pos(rect: Rect, kind: NodeKind) -> Option<Pos2> {
    if kind.accepts_inputs() {
        Some(Pos2::new(rect.min.x, rect.center().y))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render the active diagram into the central panel and handle all interactions.
pub fn show(state: &mut AppState, ui: &mut egui::Ui) {
    let panel_rect = ui.available_rect_before_wrap();
    state.last_canvas_size = panel_rect.size();

    // Allocate the full panel as an interactive widget (primary-button click +
    // drag).  Middle-mouse events are read from global InputState directly.
    let response = ui.allocate_rect(panel_rect, egui::Sense::click_and_drag());

    // Clone the active diagram to release the project borrow before any code
    // that needs `&mut state`.
    let diagram = match state.project.diagrams.get(&state.active_diagram_id) {
        Some(d) => d.clone(),
        None => {
            ui.centered_and_justified(|ui| {
                ui.label("No diagram selected.");
            });
            return;
        }
    };

    let dark_mode = ui.visuals().dark_mode;
    let mut zoom = diagram.viewport.zoom;
    let mut scroll = Vec2::new(diagram.viewport.scroll.0, diagram.viewport.scroll.1);

    // -----------------------------------------------------------------------
    // Zoom (scroll wheel, only when pointer is over the canvas)
    // -----------------------------------------------------------------------
    if response.hovered() {
        let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
        if scroll_y != 0.0 {
            let factor = ZOOM_FACTOR_PER_NOTCH.powf(scroll_y / ZOOM_NOTCH);
            let new_zoom = (zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
            if let Some(mouse) = ui.input(|i| i.pointer.hover_pos()) {
                let cx = (mouse.x - panel_rect.min.x) / zoom + scroll.x;
                let cy = (mouse.y - panel_rect.min.y) / zoom + scroll.y;
                scroll.x = cx - (mouse.x - panel_rect.min.x) / new_zoom;
                scroll.y = cy - (mouse.y - panel_rect.min.y) / new_zoom;
            }
            zoom = new_zoom;
        }
    }

    // -----------------------------------------------------------------------
    // Pan — middle-mouse drag
    // -----------------------------------------------------------------------
    {
        let middle_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle));
        let middle_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
        if middle_pressed && response.hovered() {
            state.drag_state.mid_drag_active = true;
        }
        if !middle_down {
            state.drag_state.mid_drag_active = false;
        }
        if state.drag_state.mid_drag_active {
            let delta = ui.input(|i| i.pointer.delta());
            if delta != Vec2::ZERO {
                scroll.x -= delta.x / zoom;
                scroll.y -= delta.y / zoom;
            }
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
    }

    // -----------------------------------------------------------------------
    // Pan — Space + primary drag
    // -----------------------------------------------------------------------
    let space_down = ui.input(|i| i.key_down(egui::Key::Space));
    if space_down {
        if response.dragged() {
            let delta = response.drag_delta();
            scroll.x -= delta.x / zoom;
            scroll.y -= delta.y / zoom;
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }

    // -----------------------------------------------------------------------
    // Build transform
    // -----------------------------------------------------------------------
    let xform = Transform {
        zoom,
        scroll,
        origin: panel_rect.min,
    };

    let is_panning = space_down || state.drag_state.mid_drag_active;

    // -----------------------------------------------------------------------
    // Drag detection — all three types checked in priority order so only one
    // starts per drag_started() frame.
    // -----------------------------------------------------------------------
    let no_active_drag = state.drag_state.dragging_node.is_none()
        && state.drag_state.creating_stream.is_none()
        && state.drag_state.reconnecting_stream.is_none()
        && state.drag_state.palette_drag.is_none();

    if response.drag_started() && !is_panning && no_active_drag {
        let ptr = ui.input(|i| i.pointer.interact_pos());
        if let Some(ptr) = ptr {
            // 1. Output handle → stream creation
            if let Some(source_id) = quick_hit_output_handle(&diagram, &xform, ptr) {
                state.drag_state.creating_stream = Some((source_id, ptr));
            }
            // 2. Selected stream endpoint → reconnect
            else if let Some(rs) = detect_reconnect_start(&diagram, &xform, state.selected_id, ptr) {
                state.drag_state.reconnecting_stream = Some(rs);
            }
            // 3. Node body (only if not on a handle) → node drag
            else if let Some((node_id, orig_pos)) = quick_hit_test_nodes(&diagram, &xform, ptr) {
                let canvas_ptr = xform.to_canvas(ptr);
                let offset = Vec2::new(canvas_ptr.0 - orig_pos.0, canvas_ptr.1 - orig_pos.1);
                state.drag_state.dragging_node = Some((node_id, orig_pos, offset));
                state.selected_id = Some(node_id);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Update creating_stream pointer position each frame
    // -----------------------------------------------------------------------
    if let Some((_, ref mut cur_ptr)) = state.drag_state.creating_stream {
        if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
            *cur_ptr = ptr;
        }
    }

    // -----------------------------------------------------------------------
    // Compute live drag-position override for collect_draw_data
    // -----------------------------------------------------------------------
    let drag_pos_override: Option<(Uuid, (f32, f32))> =
        if let Some((node_id, _orig, offset)) = state.drag_state.dragging_node {
            ui.input(|i| i.pointer.hover_pos()).map(|ptr| {
                let cp = xform.to_canvas(ptr);
                (node_id, (cp.0 - offset.x, cp.1 - offset.y))
            })
        } else {
            None
        };

    // -----------------------------------------------------------------------
    // Collect draw data
    // -----------------------------------------------------------------------
    let mut draw_data = collect_draw_data(&diagram, &xform, drag_pos_override);
    apply_badges(&mut draw_data, state);

    // -----------------------------------------------------------------------
    // Node drag — END
    // -----------------------------------------------------------------------
    if response.drag_stopped() {
        if let Some((node_id, from_pos, offset)) = state.drag_state.dragging_node.take() {
            let to_pos = ui
                .input(|i| i.pointer.hover_pos())
                .map(|ptr| {
                    let cp = xform.to_canvas(ptr);
                    snap_to_grid((cp.0 - offset.x, cp.1 - offset.y))
                })
                .unwrap_or(from_pos);

            if to_pos != from_pos {
                let cmd = MoveNode {
                    diagram_id: state.active_diagram_id,
                    node_id,
                    from: from_pos,
                    to: to_pos,
                };
                if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                    state.set_status(format!("Move node failed: {e}"));
                } else {
                    state.dirty = true;
                    state.validation.validate_incremental(node_id, &state.project);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Stream creation — END
    // -----------------------------------------------------------------------
    if response.drag_stopped() {
        if let Some((source_id, _)) = state.drag_state.creating_stream.take() {
            let ptr = ui.input(|i| i.pointer.hover_pos());
            let target_id = ptr.and_then(|p| {
                quick_hit_test_nodes(&diagram, &xform, p)
                    .filter(|(nid, _)| *nid != source_id)
                    .map(|(nid, _)| nid)
            });

            let source_ok = diagram
                .nodes
                .get(&source_id)
                .map(|n| n.kind.produces_outputs())
                .unwrap_or(false);

            if let Some(tgt_id) = target_id {
                let target_ok = diagram
                    .nodes
                    .get(&tgt_id)
                    .map(|n| n.kind.accepts_inputs())
                    .unwrap_or(false);

                if source_ok && target_ok {
                    let stream = Stream::new(source_id, tgt_id);
                    let stream_id = stream.id;
                    let cmd = AddStream { diagram_id: state.active_diagram_id, stream };
                    if let Err(e) =
                        state.undo_stack.execute(Box::new(cmd), &mut state.project)
                    {
                        state.set_status(format!("Add stream failed: {e}"));
                    } else {
                        state.dirty = true;
                        state.selected_id = Some(stream_id);
                        state.validation.validate_incremental(stream_id, &state.project);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Stream endpoint reconnect — END
    // -----------------------------------------------------------------------
    if response.drag_stopped() {
        if let Some((stream_id, is_source_end, fixed_node_id)) =
            state.drag_state.reconnecting_stream.take()
        {
            let ptr = ui.input(|i| i.pointer.hover_pos());
            let new_node_id = ptr.and_then(|p| {
                quick_hit_test_nodes(&diagram, &xform, p)
                    .filter(|(nid, _)| *nid != fixed_node_id)
                    .map(|(nid, _)| nid)
            });

            if let Some(new_id) = new_node_id {
                let (new_src, new_tgt) = if is_source_end {
                    (new_id, fixed_node_id)
                } else {
                    (fixed_node_id, new_id)
                };

                let src_ok = diagram
                    .nodes
                    .get(&new_src)
                    .map(|n| n.kind.produces_outputs())
                    .unwrap_or(false);
                let tgt_ok = diagram
                    .nodes
                    .get(&new_tgt)
                    .map(|n| n.kind.accepts_inputs())
                    .unwrap_or(false);

                if src_ok && tgt_ok {
                    if let Some(old_stream) = diagram.streams.get(&stream_id).cloned() {
                        let new_stream = Stream::new(new_src, new_tgt);
                        let new_stream_id = new_stream.id;
                        let compound = CompoundCommand::new(
                            "Reconnect stream",
                            vec![
                                Box::new(DeleteStream {
                                    diagram_id: state.active_diagram_id,
                                    stream: old_stream,
                                }),
                                Box::new(AddStream {
                                    diagram_id: state.active_diagram_id,
                                    stream: new_stream,
                                }),
                            ],
                        );
                        if let Err(e) =
                            state.undo_stack.execute(Box::new(compound), &mut state.project)
                        {
                            state.set_status(format!("Reconnect stream failed: {e}"));
                        } else {
                            state.dirty = true;
                            state.selected_id = Some(new_stream_id);
                            state.validation.validate_incremental(new_stream_id, &state.project);
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Palette drag — DROP (primary button released anywhere)
    // -----------------------------------------------------------------------
    let primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
    if primary_released {
        if let Some(kind) = state.drag_state.palette_drag {
            state.drag_state.palette_drag = None;
            if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                if panel_rect.contains(ptr) {
                    let cp = xform.to_canvas(ptr);
                    let node_pos =
                        snap_to_grid((cp.0 - NODE_W / 2.0, cp.1 - NODE_H / 2.0));
                    let name = next_node_name(&diagram, kind);
                    let node = Node::new(kind, name);
                    let node_id = node.id;
                    let diagram_id = state.active_diagram_id;
                    let cmd = AddNode { diagram_id, node, position: node_pos };
                    if let Err(e) =
                        state.undo_stack.execute(Box::new(cmd), &mut state.project)
                    {
                        state.set_status(format!("Add node failed: {e}"));
                    } else {
                        state.dirty = true;
                        state.selected_id = Some(node_id);
                        state.validation.validate_incremental(node_id, &state.project);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Escape — cancel active drags without committing
    // -----------------------------------------------------------------------
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.drag_state.dragging_node = None;
        state.drag_state.palette_drag = None;
        state.drag_state.creating_stream = None;
        state.drag_state.reconnecting_stream = None;
        state.multi_selected_streams.clear();
    }

    // -----------------------------------------------------------------------
    // Click — selection (suppressed while any drag is in progress)
    // -----------------------------------------------------------------------
    let dragging_anything = state.drag_state.dragging_node.is_some()
        || state.drag_state.palette_drag.is_some()
        || state.drag_state.creating_stream.is_some()
        || state.drag_state.reconnecting_stream.is_some();

    if !dragging_anything {
        let interact_pos = ui.input(|i| i.pointer.interact_pos());
        let ctrl_held = ui.input(|i| i.modifiers.ctrl);
        if response.double_clicked() {
            let hit = interact_pos.and_then(|p| hit_test(&draw_data, p));
            state.selected_id = hit;
            state.multi_selected_streams.clear();
            if let Some(id) = hit {
                state.active_panel = ActivePanel::Spec(id);
            }
        } else if response.clicked() {
            let hit = interact_pos.and_then(|p| hit_test(&draw_data, p));
            if ctrl_held {
                if let Some(id) = hit {
                    if diagram.streams.contains_key(&id) {
                        // Toggle stream in the multi-select set.
                        if state.multi_selected_streams.contains(&id) {
                            state.multi_selected_streams.remove(&id);
                        } else {
                            state.multi_selected_streams.insert(id);
                        }
                    } else {
                        // Ctrl+click on a node: normal select, clear multi-select.
                        state.selected_id = Some(id);
                        state.multi_selected_streams.clear();
                    }
                }
                // Ctrl+click on empty canvas: no change.
            } else {
                state.selected_id = hit;
                state.multi_selected_streams.clear();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Enter key — open spec panel for selected component
    // -----------------------------------------------------------------------
    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some(id) = state.selected_id {
            state.active_panel = ActivePanel::Spec(id);
        }
    }

    // -----------------------------------------------------------------------
    // Right-click — capture context menu target (before context_menu() call)
    // -----------------------------------------------------------------------
    if response.secondary_clicked() {
        let ptr = ui.input(|i| i.pointer.interact_pos());
        state.context_menu_target = ptr.and_then(|p| hit_test(&draw_data, p)).map(|id| {
            if diagram.nodes.contains_key(&id) {
                ContextTarget::Node(id)
            } else {
                ContextTarget::Stream(id)
            }
        });
    }

    // -----------------------------------------------------------------------
    // Cursor icon
    // -----------------------------------------------------------------------
    if !is_panning {
        if state.drag_state.dragging_node.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if state.drag_state.creating_stream.is_some()
            || state.drag_state.reconnecting_stream.is_some()
        {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        } else if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
            if quick_hit_output_handle(&diagram, &xform, ptr).is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            } else if quick_hit_test_nodes(&diagram, &xform, ptr).is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Sync selection into draw data (after all interactions that may change it)
    // -----------------------------------------------------------------------
    mark_selected(&mut draw_data, state.selected_id, &state.multi_selected_streams);

    // -----------------------------------------------------------------------
    // Write back viewport if it changed this frame
    // -----------------------------------------------------------------------
    let vp = &diagram.viewport;
    if (zoom - vp.zoom).abs() > f32::EPSILON
        || (scroll.x - vp.scroll.0).abs() > f32::EPSILON
        || (scroll.y - vp.scroll.1).abs() > f32::EPSILON
    {
        if let Some(d) = state.project.diagrams.get_mut(&state.active_diagram_id) {
            d.viewport.zoom = zoom;
            d.viewport.scroll = (scroll.x, scroll.y);
        }
    }

    // -----------------------------------------------------------------------
    // Painting
    // -----------------------------------------------------------------------
    let painter = ui.painter_at(panel_rect);

    painter.rect_filled(
        panel_rect,
        0.0,
        if dark_mode {
            Color32::from_rgb(30, 30, 35)
        } else {
            Color32::from_rgb(245, 245, 248)
        },
    );

    draw_grid(&painter, panel_rect, &xform, dark_mode);

    for sd in &draw_data.streams {
        draw_stream(&painter, sd, dark_mode);
    }
    for nd in &draw_data.nodes {
        draw_node(&painter, nd, dark_mode);
    }

    // Selection highlights (above nodes/streams, below badges).
    for nd in &draw_data.nodes {
        if nd.selected {
            draw_node_selection(&painter, nd);
        }
    }
    for sd in &draw_data.streams {
        if sd.selected {
            draw_stream_selection(&painter, sd);
        }
    }
    // Multi-selection highlights (amber) for streams Ctrl+clicked for group creation.
    for sd in &draw_data.streams {
        if sd.multi_selected && !sd.selected {
            draw_stream_multi_selection(&painter, sd);
        }
    }

    // Group name labels (above selection highlights, below badges).
    for gl in &draw_data.group_labels {
        draw_group_label(&painter, gl, dark_mode);
    }

    // Validation badges.
    for nd in &draw_data.nodes {
        if nd.badge != ValidationState::Ok {
            draw_node_badge(&painter, nd);
        }
    }
    for sd in &draw_data.streams {
        if sd.badge != ValidationState::Ok {
            draw_stream_badge(&painter, sd);
        }
    }

    // -----------------------------------------------------------------------
    // Connection handles (visible on node hover when not in a drag)
    // -----------------------------------------------------------------------
    if !dragging_anything {
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
        for nd in &draw_data.nodes {
            let hovered = hover_pos.map(|p| nd.rect.contains(p)).unwrap_or(false);
            if hovered {
                draw_node_handles(&painter, nd);
            }
        }
    }

    // Stream endpoint handles on selected stream (for reconnect dragging).
    if let Some(sel_id) = state.selected_id {
        if let Some(sel_sd) = draw_data.streams.iter().find(|s| s.stream_id == sel_id) {
            if let (Some(&first), Some(&last)) =
                (sel_sd.points.first(), sel_sd.points.last())
            {
                painter.circle_filled(first, HANDLE_R, SELECT_COLOR);
                painter.circle_filled(last, HANDLE_R, SELECT_COLOR);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Stream creation preview line
    // -----------------------------------------------------------------------
    if let Some((source_id, current_ptr)) = state.drag_state.creating_stream {
        if let Some(src_nd) = draw_data.nodes.iter().find(|n| n.node_id == source_id) {
            if let Some(src_handle) = output_handle_pos(src_nd.rect, src_nd.kind) {
                let line_color = Color32::from_rgba_unmultiplied(80, 180, 255, 200);
                painter.line_segment(
                    [src_handle, current_ptr],
                    Stroke::new(2.0, line_color),
                );
                painter.circle_filled(src_handle, HANDLE_R, line_color);
                // Highlight target node input handle if hovering over a valid target.
                if let Some(tgt_nd) = draw_data
                    .nodes
                    .iter()
                    .find(|n| n.rect.contains(current_ptr) && n.node_id != source_id)
                {
                    if tgt_nd.kind.accepts_inputs() {
                        if let Some(tgt_handle) = input_handle_pos(tgt_nd.rect, tgt_nd.kind) {
                            painter.circle_filled(
                                tgt_handle,
                                HANDLE_R + 2.0,
                                Color32::from_rgba_unmultiplied(80, 220, 80, 220),
                            );
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Stream endpoint reconnect preview line
    // -----------------------------------------------------------------------
    if let Some((_stream_id, is_source_end, fixed_node_id)) = state.drag_state.reconnecting_stream {
        let current_ptr = ui.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::ZERO);
        if let Some(fixed_nd) = draw_data.nodes.iter().find(|n| n.node_id == fixed_node_id) {
            // Fixed end is opposite side from the dragged end.
            let fixed_pt = if is_source_end {
                input_handle_pos(fixed_nd.rect, fixed_nd.kind)
                    .unwrap_or(fixed_nd.rect.left_center())
            } else {
                output_handle_pos(fixed_nd.rect, fixed_nd.kind)
                    .unwrap_or(fixed_nd.rect.right_center())
            };
            let line_color = Color32::from_rgba_unmultiplied(255, 160, 80, 200);
            painter.line_segment([fixed_pt, current_ptr], Stroke::new(2.0, line_color));
            painter.circle_filled(fixed_pt, HANDLE_R, line_color);
        }
    }

    // Ghost node while dragging from palette.
    if let Some(kind) = state.drag_state.palette_drag {
        if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
            if panel_rect.contains(ptr) {
                draw_palette_ghost(&painter, &xform, kind, ptr, dark_mode);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Context menu (consumes `response` — must be last)
    // -----------------------------------------------------------------------
    response.context_menu(|ui| {
        match state.context_menu_target {
            Some(ContextTarget::Node(node_id)) => {
                show_node_context_menu(state, &diagram, node_id, ui);
            }
            Some(ContextTarget::Stream(stream_id)) => {
                show_stream_context_menu(state, &diagram, stream_id, ui);
            }
            None => {
                ui.close_menu();
            }
        }
    });

}

// ---------------------------------------------------------------------------
// Transform: canvas ↔ screen
// ---------------------------------------------------------------------------

struct Transform {
    zoom: f32,
    scroll: Vec2,
    origin: Pos2,
}

impl Transform {
    fn to_screen(&self, canvas: (f32, f32)) -> Pos2 {
        let cx = (canvas.0 - self.scroll.x) * self.zoom;
        let cy = (canvas.1 - self.scroll.y) * self.zoom;
        self.origin + Vec2::new(cx, cy)
    }

    fn to_canvas(&self, screen: Pos2) -> (f32, f32) {
        let sx = screen.x - self.origin.x;
        let sy = screen.y - self.origin.y;
        (sx / self.zoom + self.scroll.x, sy / self.zoom + self.scroll.y)
    }

    fn scale(&self, v: f32) -> f32 {
        v * self.zoom
    }
}

// ---------------------------------------------------------------------------
// Draw data structures
// ---------------------------------------------------------------------------

struct NodeDrawData {
    node_id: Uuid,
    rect: Rect,
    kind: NodeKind,
    name: String,
    is_boundary: bool,
    badge: ValidationState,
    selected: bool,
}

struct StreamDrawData {
    stream_id: Uuid,
    points: Vec<Pos2>,
    midpoint: Pos2,
    badge: ValidationState,
    /// Pre-computed perpendicular display offset (screen pixels).
    /// Accounts for group membership AND ungrouped co-incident streams sharing
    /// the same source/target pair so every stream is individually clickable.
    display_offset: Vec2,
    /// Group colour, set for streams that belong to a named group.
    group_color: Option<Color32>,
    selected: bool,
    multi_selected: bool,
}

struct GroupLabelDrawData {
    name: String,
    /// Screen position of the first (slot-0) stream at its path midpoint.
    bracket_a: Pos2,
    /// Screen position of the last stream at its path midpoint.
    bracket_b: Pos2,
    /// Where to draw the name text (just beyond bracket_b).
    label_pos: Pos2,
    color: Color32,
}

pub(crate) struct DiagramDrawData {
    nodes: Vec<NodeDrawData>,
    streams: Vec<StreamDrawData>,
    group_labels: Vec<GroupLabelDrawData>,
}

// ---------------------------------------------------------------------------
// Data collection
// ---------------------------------------------------------------------------

fn collect_draw_data(
    diagram: &Diagram,
    xform: &Transform,
    drag_pos_override: Option<(Uuid, (f32, f32))>,
) -> DiagramDrawData {
    let node_rects: HashMap<Uuid, Rect> = diagram
        .nodes
        .keys()
        .filter_map(|&nid| {
            let pos = if drag_pos_override.map_or(false, |(id, _)| id == nid) {
                drag_pos_override.unwrap().1
            } else {
                diagram.positions.get(&nid).copied().unwrap_or((0.0, 0.0))
            };
            let tl = xform.to_screen(pos);
            let br = tl + Vec2::new(xform.scale(NODE_W), xform.scale(NODE_H));
            Some((nid, Rect::from_min_max(tl, br)))
        })
        .collect();

    let mut nodes: Vec<NodeDrawData> = diagram
        .nodes
        .values()
        .map(|node| {
            let rect = node_rects
                .get(&node.id)
                .copied()
                .unwrap_or(Rect::from_min_size(xform.origin, Vec2::splat(10.0)));
            NodeDrawData {
                node_id: node.id,
                rect,
                kind: node.kind,
                name: node.name.clone(),
                is_boundary: node.is_boundary,
                badge: ValidationState::Ok,
                selected: false,
            }
        })
        .collect();
    nodes.sort_by_key(|n| n.node_id);

    let mut streams: Vec<StreamDrawData> = diagram
        .streams
        .values()
        .map(|stream| {
            let src_rect = node_rects.get(&stream.source_node_id).copied();
            let tgt_rect = node_rects.get(&stream.target_node_id).copied();
            let (points, midpoint) = compute_stream_path(src_rect, tgt_rect);
            let group_color = stream
                .group_id
                .and_then(|gid| diagram.stream_groups.get(&gid))
                .map(|g| Color32::from_rgb(g.color[0], g.color[1], g.color[2]));
            StreamDrawData {
                stream_id: stream.id,
                points,
                midpoint,
                badge: ValidationState::Ok,
                display_offset: Vec2::ZERO,
                group_color,
                selected: false,
                multi_selected: false,
            }
        })
        .collect();
    streams.sort_by_key(|s| s.stream_id);

    // Build an index for O(1) stream lookup by ID.
    let stream_idx: HashMap<Uuid, usize> =
        streams.iter().enumerate().map(|(i, s)| (s.stream_id, i)).collect();

    // Compute per-stream perpendicular display offsets in a single unified pass
    // per (src, tgt) path.  Groups are assigned contiguous slots (sorted by
    // group ID for determinism) so their streams always appear adjacent.
    // Ungrouped co-incident streams get the remaining slots after all groups.
    let mut path_streams: HashMap<(Uuid, Uuid), Vec<Uuid>> = HashMap::new();
    for stream in diagram.streams.values() {
        path_streams
            .entry((stream.source_node_id, stream.target_node_id))
            .or_default()
            .push(stream.id);
    }

    let all_group_ids: Vec<Uuid> = {
        let mut v: Vec<Uuid> = diagram.stream_groups.keys().copied().collect();
        v.sort();
        v
    };

    for sids in path_streams.values() {
        if sids.len() <= 1 {
            continue;
        }
        // Groups that have at least one member on this path, in sorted order.
        let groups_on_path: Vec<Uuid> = all_group_ids
            .iter()
            .copied()
            .filter(|gid| {
                diagram
                    .stream_groups
                    .get(gid)
                    .map_or(false, |g| g.member_stream_ids.iter().any(|s| sids.contains(s)))
            })
            .collect();

        // Ordered slot list: grouped streams first (group order, then member order),
        // then ungrouped streams sorted by ID.
        let mut ordered: Vec<Uuid> = Vec::new();
        for gid in &groups_on_path {
            if let Some(g) = diagram.stream_groups.get(gid) {
                for &sid in &g.member_stream_ids {
                    if sids.contains(&sid) {
                        ordered.push(sid);
                    }
                }
            }
        }
        let grouped_set: HashSet<Uuid> = ordered.iter().copied().collect();
        let mut ungrouped: Vec<Uuid> =
            sids.iter().copied().filter(|s| !grouped_set.contains(s)).collect();
        ungrouped.sort();
        ordered.extend(ungrouped);

        let n = ordered.len() as f32;
        for (slot, &sid) in ordered.iter().enumerate() {
            if let Some(&idx) = stream_idx.get(&sid) {
                let offset = {
                    let sd = &streams[idx];
                    if sd.points.len() >= 2 {
                        let seg_dir = (sd.points[1] - sd.points[0]).normalized();
                        let perp = Vec2::new(-seg_dir.y, seg_dir.x);
                        let centred = slot as f32 - (n - 1.0) * 0.5;
                        perp * (centred * GROUP_OFFSET)
                    } else {
                        Vec2::ZERO
                    }
                };
                streams[idx].display_offset = offset;
            }
        }
    }

    // Build group labels as a bracket spanning all member stream positions at
    // 1/4 of the way along each stream's path.
    let mut group_labels: Vec<GroupLabelDrawData> = Vec::new();
    for group in diagram.stream_groups.values() {
        let n = group.member_stream_ids.len();
        if n == 0 {
            continue;
        }
        let positions: Vec<Pos2> = group
            .member_stream_ids
            .iter()
            .filter_map(|&sid| {
                stream_idx
                    .get(&sid)
                    .map(|&i| path_at_t(&streams[i].points, 0.25) + streams[i].display_offset)
            })
            .collect();
        if positions.is_empty() {
            continue;
        }
        let bracket_a = positions[0];
        let bracket_b = *positions.last().unwrap();
        // Extend beyond the last stream in the cross-stream direction for the name.
        let span = bracket_b - bracket_a;
        let ext_dir = if span.length() > 1.0 {
            span.normalized()
        } else {
            Vec2::new(0.0, -1.0)
        };
        let label_pos = bracket_b + ext_dir * 18.0;
        let color = Color32::from_rgb(group.color[0], group.color[1], group.color[2]);
        group_labels.push(GroupLabelDrawData {
            name: group.name.clone(),
            bracket_a,
            bracket_b,
            label_pos,
            color,
        });
    }

    DiagramDrawData { nodes, streams, group_labels }
}

/// Attach validation badge states — separate pass to keep collect_draw_data
/// free of validation imports.
pub(crate) fn apply_badges(data: &mut DiagramDrawData, state: &AppState) {
    for nd in &mut data.nodes {
        nd.badge = state.validation.state(nd.node_id);
    }
    for sd in &mut data.streams {
        sd.badge = state.validation.state(sd.stream_id);
    }
}

/// Set the `selected` and `multi_selected` flags on draw-data entries.
fn mark_selected(
    data: &mut DiagramDrawData,
    selected_id: Option<Uuid>,
    multi_selected: &HashSet<Uuid>,
) {
    for nd in &mut data.nodes {
        nd.selected = selected_id == Some(nd.node_id);
    }
    for sd in &mut data.streams {
        sd.selected = selected_id == Some(sd.stream_id);
        sd.multi_selected = multi_selected.contains(&sd.stream_id);
    }
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

/// Return the UUID of the topmost node or stream under screen-space `pos`,
/// or `None`.  Nodes take priority over streams.
fn hit_test(draw_data: &DiagramDrawData, pos: Pos2) -> Option<Uuid> {
    for nd in draw_data.nodes.iter().rev() {
        if nd.rect.contains(pos) {
            return Some(nd.node_id);
        }
    }
    for sd in &draw_data.streams {
        let shifted: Vec<Pos2> = sd.points.iter().map(|&p| p + sd.display_offset).collect();
        if hit_test_polyline(pos, &shifted, STREAM_HIT_TOLERANCE) {
            return Some(sd.stream_id);
        }
    }
    None
}

/// Find a node under `ptr` using diagram data directly (before draw data is built).
/// Returns `(node_id, canvas_tl)` or `None`.
fn quick_hit_test_nodes(diagram: &Diagram, xform: &Transform, ptr: Pos2) -> Option<(Uuid, (f32, f32))> {
    let mut ids: Vec<Uuid> = diagram.nodes.keys().copied().collect();
    ids.sort_by(|a, b| b.cmp(a));
    for nid in ids {
        let pos = diagram.positions.get(&nid).copied().unwrap_or((0.0, 0.0));
        let tl = xform.to_screen(pos);
        let br = tl + Vec2::new(xform.scale(NODE_W), xform.scale(NODE_H));
        if Rect::from_min_max(tl, br).contains(ptr) {
            return Some((nid, pos));
        }
    }
    None
}

/// Find a node whose **output handle** is within `HANDLE_HIT_R` of `ptr`.
fn quick_hit_output_handle(diagram: &Diagram, xform: &Transform, ptr: Pos2) -> Option<Uuid> {
    for (&nid, node) in &diagram.nodes {
        if !node.kind.produces_outputs() {
            continue;
        }
        let pos = diagram.positions.get(&nid).copied().unwrap_or((0.0, 0.0));
        let tl = xform.to_screen(pos);
        let br = tl + Vec2::new(xform.scale(NODE_W), xform.scale(NODE_H));
        let handle = Pos2::new(br.x, (tl.y + br.y) * 0.5);
        if (ptr - handle).length() <= HANDLE_HIT_R {
            return Some(nid);
        }
    }
    None
}

/// Check if `ptr` is near either endpoint of the currently selected stream.
/// Returns `(stream_id, is_source_end, fixed_node_id)` on a hit.
fn detect_reconnect_start(
    diagram: &Diagram,
    xform: &Transform,
    selected_id: Option<Uuid>,
    ptr: Pos2,
) -> Option<(Uuid, bool, Uuid)> {
    let sel_id = selected_id?;
    let stream = diagram.streams.get(&sel_id)?;
    if stream.is_dangling {
        return None;
    }
    let (src_pt, tgt_pt) = stream_endpoint_screen_pos(stream, diagram, xform)?;
    if (ptr - src_pt).length() <= HANDLE_HIT_R {
        return Some((sel_id, true, stream.target_node_id));
    }
    if (ptr - tgt_pt).length() <= HANDLE_HIT_R {
        return Some((sel_id, false, stream.source_node_id));
    }
    None
}

/// Compute the screen-space positions of a stream's source-exit and
/// target-entry points, mirroring [`compute_stream_path`].
fn stream_endpoint_screen_pos(
    stream: &Stream,
    diagram: &Diagram,
    xform: &Transform,
) -> Option<(Pos2, Pos2)> {
    let src_pos = diagram.positions.get(&stream.source_node_id).copied()?;
    let tgt_pos = diagram.positions.get(&stream.target_node_id).copied()?;
    let src_tl = xform.to_screen(src_pos);
    let src_br = src_tl + Vec2::new(xform.scale(NODE_W), xform.scale(NODE_H));
    let tgt_tl = xform.to_screen(tgt_pos);
    let tgt_br = tgt_tl + Vec2::new(xform.scale(NODE_W), xform.scale(NODE_H));
    let (src_cx, src_cy) = ((src_tl.x + src_br.x) * 0.5, (src_tl.y + src_br.y) * 0.5);
    let (tgt_cx, tgt_cy) = ((tgt_tl.x + tgt_br.x) * 0.5, (tgt_tl.y + tgt_br.y) * 0.5);
    if src_cx <= tgt_cx {
        Some((Pos2::new(src_br.x, src_cy), Pos2::new(tgt_tl.x, tgt_cy)))
    } else {
        Some((Pos2::new(src_cx, src_br.y), Pos2::new(tgt_cx, tgt_tl.y)))
    }
}

fn hit_test_polyline(pos: Pos2, points: &[Pos2], tolerance: f32) -> bool {
    for w in points.windows(2) {
        if dist_point_to_segment(pos, w[0], w[1]) <= tolerance {
            return true;
        }
    }
    false
}

fn dist_point_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let len_sq = ab.length_sq();
    if len_sq < f32::EPSILON {
        return ap.length();
    }
    let closest = a + ab * (ap.dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - closest).length()
}

// ---------------------------------------------------------------------------
// Grid snap
// ---------------------------------------------------------------------------

fn snap_to_grid(pos: (f32, f32)) -> (f32, f32) {
    (
        (pos.0 / SNAP_GRID).round() * SNAP_GRID,
        (pos.1 / SNAP_GRID).round() * SNAP_GRID,
    )
}

// ---------------------------------------------------------------------------
// Node naming helper
// ---------------------------------------------------------------------------

fn next_node_name(diagram: &Diagram, kind: NodeKind) -> String {
    let count = diagram.nodes.values().filter(|n| n.kind == kind).count();
    format!("{} {}", kind.label(), count + 1)
}

// ---------------------------------------------------------------------------
// Stream path computation
// ---------------------------------------------------------------------------

/// Return the point at fraction `t` (0 = start, 1 = end) along a polyline by arc length.
fn path_at_t(points: &[Pos2], t: f32) -> Pos2 {
    if points.len() < 2 {
        return points.first().copied().unwrap_or(Pos2::ZERO);
    }
    let total: f32 = points.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    if total < f32::EPSILON {
        return points[0];
    }
    let target = t.clamp(0.0, 1.0) * total;
    let mut walked = 0.0_f32;
    for w in points.windows(2) {
        let seg_len = (w[1] - w[0]).length();
        if walked + seg_len >= target {
            let frac = if seg_len > f32::EPSILON { (target - walked) / seg_len } else { 0.0 };
            return w[0].lerp(w[1], frac);
        }
        walked += seg_len;
    }
    *points.last().unwrap()
}

fn compute_stream_path(src_rect: Option<Rect>, tgt_rect: Option<Rect>) -> (Vec<Pos2>, Pos2) {
    let (src_r, tgt_r) = match (src_rect, tgt_rect) {
        (Some(s), Some(t)) => (s, t),
        _ => {
            let p = Pos2::ZERO;
            return (vec![p, p, p, p], p);
        }
    };

    let (src_exit, tgt_entry) = if src_r.center().x <= tgt_r.center().x {
        (
            Pos2::new(src_r.max.x, src_r.center().y),
            Pos2::new(tgt_r.min.x, tgt_r.center().y),
        )
    } else {
        (
            Pos2::new(src_r.center().x, src_r.max.y),
            Pos2::new(tgt_r.center().x, tgt_r.min.y),
        )
    };

    let mid_x = (src_exit.x + tgt_entry.x) * 0.5;
    let h_bend = Pos2::new(mid_x, src_exit.y);
    let v_bend = Pos2::new(mid_x, tgt_entry.y);
    let midpoint = h_bend.lerp(v_bend, 0.5);
    (vec![src_exit, h_bend, v_bend, tgt_entry], midpoint)
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

fn draw_grid(painter: &Painter, rect: Rect, xform: &Transform, dark_mode: bool) {
    let dot_color = if dark_mode {
        Color32::from_rgba_premultiplied(60, 60, 70, 255)
    } else {
        Color32::from_rgba_premultiplied(200, 200, 210, 255)
    };

    let grid_spacing = xform.scale(40.0);
    if grid_spacing < 8.0 {
        return;
    }

    let (canvas_min_x, canvas_min_y) = xform.to_canvas(rect.min);
    let start_cx = (canvas_min_x / 40.0).floor() * 40.0;
    let start_cy = (canvas_min_y / 40.0).floor() * 40.0;
    let start = xform.to_screen((start_cx, start_cy));

    let mut x = start.x;
    while x <= rect.max.x {
        let mut y = start.y;
        while y <= rect.max.y {
            painter.circle_filled(Pos2::new(x, y), 1.0, dot_color);
            y += grid_spacing;
        }
        x += grid_spacing;
    }
}

// ---------------------------------------------------------------------------
// Node drawing
// ---------------------------------------------------------------------------

fn draw_node(painter: &Painter, nd: &NodeDrawData, dark_mode: bool) {
    let fill = node_fill(nd.kind, dark_mode);
    let stroke_color = node_stroke_color(nd.kind, dark_mode);
    let rounding = Rounding::same(NODE_ROUNDING);

    painter.rect_filled(nd.rect, rounding, fill);
    if nd.is_boundary {
        draw_dashed_rect(painter, nd.rect, rounding, Stroke::new(NODE_STROKE, stroke_color));
    } else {
        painter.rect_stroke(nd.rect, rounding, Stroke::new(NODE_STROKE, stroke_color));
    }

    painter.text(
        nd.rect.center(),
        egui::Align2::CENTER_CENTER,
        &nd.name,
        FontId::proportional(11.0),
        if dark_mode { Color32::WHITE } else { Color32::BLACK },
    );
}

fn draw_node_selection(painter: &Painter, nd: &NodeDrawData) {
    painter.rect_stroke(
        nd.rect.expand(3.0),
        Rounding::same(NODE_ROUNDING + 3.0),
        Stroke::new(2.0, SELECT_COLOR),
    );
}

/// Draw small connection-handle circles on a node's relevant edges.
fn draw_node_handles(painter: &Painter, nd: &NodeDrawData) {
    let ring = Color32::WHITE;
    let border = SELECT_COLOR;
    if let Some(pos) = output_handle_pos(nd.rect, nd.kind) {
        painter.circle_filled(pos, HANDLE_R, ring);
        painter.circle_stroke(pos, HANDLE_R, Stroke::new(1.5, border));
    }
    if let Some(pos) = input_handle_pos(nd.rect, nd.kind) {
        painter.circle_filled(pos, HANDLE_R, ring);
        painter.circle_stroke(pos, HANDLE_R, Stroke::new(1.5, border));
    }
}

/// Draw a semi-transparent ghost node at the cursor position for palette drags.
fn draw_palette_ghost(
    painter: &Painter,
    xform: &Transform,
    kind: NodeKind,
    ptr: Pos2,
    dark_mode: bool,
) {
    let cp = xform.to_canvas(ptr);
    let ghost_tl = (cp.0 - NODE_W / 2.0, cp.1 - NODE_H / 2.0);
    let tl = xform.to_screen(ghost_tl);
    let rect = Rect::from_min_size(tl, Vec2::new(xform.scale(NODE_W), xform.scale(NODE_H)));

    let base = node_fill(kind, dark_mode);
    let ghost_fill = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 160);
    let stroke_color = node_stroke_color(kind, dark_mode);

    painter.rect_filled(rect, Rounding::same(NODE_ROUNDING), ghost_fill);
    draw_dashed_rect(painter, rect, Rounding::same(NODE_ROUNDING), Stroke::new(1.5, stroke_color));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        kind.label(),
        FontId::proportional(11.0),
        if dark_mode {
            Color32::from_rgba_unmultiplied(255, 255, 255, 200)
        } else {
            Color32::from_rgba_unmultiplied(0, 0, 0, 200)
        },
    );
}

fn draw_dashed_rect(painter: &Painter, rect: Rect, _rounding: Rounding, stroke: Stroke) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    for window in corners.windows(2) {
        draw_dashed_segment(painter, window[0], window[1], stroke, DASH_LEN, DASH_GAP);
    }
}

fn draw_dashed_segment(painter: &Painter, a: Pos2, b: Pos2, stroke: Stroke, dash: f32, gap: f32) {
    let dir = (b - a).normalized();
    let total = (b - a).length();
    let period = dash + gap;
    if period <= 0.0 {
        return;
    }
    let mut t = 0.0_f32;
    while t < total {
        let end_t = (t + dash).min(total);
        painter.line_segment([a + dir * t, a + dir * end_t], stroke);
        t += period;
    }
}

// ---------------------------------------------------------------------------
// Stream drawing
// ---------------------------------------------------------------------------

fn draw_stream(painter: &Painter, sd: &StreamDrawData, dark_mode: bool) {
    let base_color = sd.group_color.unwrap_or_else(|| stream_color(dark_mode));
    let points: Vec<Pos2> = sd.points.iter().map(|&p| p + sd.display_offset).collect();
    if points.len() < 2 {
        return;
    }
    let stroke = Stroke::new(STREAM_STROKE, base_color);
    for window in points.windows(2) {
        painter.line_segment([window[0], window[1]], stroke);
    }
    if let (Some(&tip), Some(&before)) =
        (points.last(), points.get(points.len().saturating_sub(2)))
    {
        draw_arrowhead(painter, before, tip, ARROW_HALF, base_color);
    }
}

fn draw_stream_selection(painter: &Painter, sd: &StreamDrawData) {
    let points: Vec<Pos2> = sd.points.iter().map(|&p| p + sd.display_offset).collect();
    let stroke = Stroke::new(STREAM_STROKE + 2.0, SELECT_COLOR);
    for window in points.windows(2) {
        painter.line_segment([window[0], window[1]], stroke);
    }
}

fn draw_stream_multi_selection(painter: &Painter, sd: &StreamDrawData) {
    let points: Vec<Pos2> = sd.points.iter().map(|&p| p + sd.display_offset).collect();
    let stroke = Stroke::new(STREAM_STROKE + 2.0, MULTI_SELECT_COLOR);
    for window in points.windows(2) {
        painter.line_segment([window[0], window[1]], stroke);
    }
}

fn draw_group_label(painter: &Painter, gl: &GroupLabelDrawData, _dark_mode: bool) {
    let color = gl.color;
    // Draw a bracket line spanning the bundle width (skip if single-stream group).
    let span = gl.bracket_b - gl.bracket_a;
    if span.length() > 2.0 {
        painter.line_segment([gl.bracket_a, gl.bracket_b], Stroke::new(1.5, color));
        // Small end ticks perpendicular to the bracket.
        let dir = span.normalized();
        let tick = Vec2::new(-dir.y, dir.x) * 4.0;
        painter.line_segment(
            [gl.bracket_a - tick, gl.bracket_a + tick],
            Stroke::new(1.0, color),
        );
        painter.line_segment(
            [gl.bracket_b - tick, gl.bracket_b + tick],
            Stroke::new(1.0, color),
        );
    }
    painter.text(
        gl.label_pos,
        egui::Align2::LEFT_CENTER,
        &gl.name,
        FontId::proportional(10.0),
        color,
    );
}

fn draw_arrowhead(painter: &Painter, from: Pos2, tip: Pos2, half: f32, color: Color32) {
    let dir = (tip - from).normalized();
    let perp = Vec2::new(-dir.y, dir.x);
    let base = tip - dir * (half * 1.5);
    painter.add(egui::Shape::convex_polygon(
        vec![tip, base + perp * half, base - perp * half],
        color,
        Stroke::NONE,
    ));
}


// ---------------------------------------------------------------------------
// Badge drawing
// ---------------------------------------------------------------------------

fn draw_node_badge(painter: &Painter, nd: &NodeDrawData) {
    let color = badge_color(nd.badge);
    let center = nd.rect.right_top() + Vec2::new(-BADGE_R, BADGE_R);
    painter.circle_filled(center, BADGE_R, color);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        badge_icon(nd.badge),
        FontId::proportional(8.0),
        Color32::WHITE,
    );
}

fn draw_stream_badge(painter: &Painter, sd: &StreamDrawData) {
    let color = badge_color(sd.badge);
    painter.circle_filled(sd.midpoint, BADGE_R, color);
    painter.text(
        sd.midpoint,
        egui::Align2::CENTER_CENTER,
        badge_icon(sd.badge),
        FontId::proportional(8.0),
        Color32::WHITE,
    );
}

fn badge_color(state: ValidationState) -> Color32 {
    match state {
        ValidationState::Ok => Color32::TRANSPARENT,
        ValidationState::Warning => AMBER,
        ValidationState::Error => ERROR_RED,
    }
}

fn badge_icon(state: ValidationState) -> &'static str {
    match state {
        ValidationState::Ok => "",
        ValidationState::Warning => "!",
        ValidationState::Error => "✕",
    }
}

// ---------------------------------------------------------------------------
// Context menu helpers
// ---------------------------------------------------------------------------

/// Show the right-click context menu for a node.
fn show_node_context_menu(
    state: &mut AppState,
    diagram: &Diagram,
    node_id: Uuid,
    ui: &mut egui::Ui,
) {
    let Some(node) = diagram.nodes.get(&node_id) else {
        ui.close_menu();
        return;
    };

    if ui.button("Open Spec").clicked() {
        state.selected_id = Some(node_id);
        state.active_panel = ActivePanel::Spec(node_id);
        ui.close_menu();
    }

    if node.is_deletable() && ui.button("Delete").clicked() {
        delete_node_with_streams(state, diagram, node_id, node);
        if state.selected_id == Some(node_id) {
            state.selected_id = None;
        }
        ui.close_menu();
    }

    if node.kind == NodeKind::Process {
        if !node.has_subdiagram() && ui.button("Create Sub-diagram").clicked() {
            let cmd = CreateSubDiagram {
                parent_diagram_id: state.active_diagram_id,
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

        let has_orphans = !state.project.orphaned_diagram_ids.is_empty();
        if has_orphans && ui.button("Set Sub-diagram…").clicked() {
            let orphan_list: Vec<(Uuid, String)> = state
                .project
                .orphaned_diagram_ids
                .iter()
                .filter_map(|&id| {
                    state.project.diagram(id).map(|d| (id, d.name.clone()))
                })
                .collect();
            state.dialogs.set_subdiagram.open_for(node_id, orphan_list);
            ui.close_menu();
        }
    }
}

/// Show the right-click context menu for a stream.
fn show_stream_context_menu(
    state: &mut AppState,
    diagram: &Diagram,
    stream_id: Uuid,
    ui: &mut egui::Ui,
) {
    let Some(stream) = diagram.streams.get(&stream_id) else {
        ui.close_menu();
        return;
    };
    let src_id = stream.source_node_id;
    let tgt_id = stream.target_node_id;
    let stream_group_id = stream.group_id;
    let stream_clone = stream.clone();

    // ── Create group ──────────────────────────────────────────────────────
    // Shown when this stream is part of a Ctrl+click multi-selection (≥2).
    let show_create_group = state.multi_selected_streams.contains(&stream_id)
        && state.multi_selected_streams.len() >= 2;
    let mut do_create_group = false;
    if show_create_group {
        if ui
            .button(format!(
                "Create group ({} streams)",
                state.multi_selected_streams.len()
            ))
            .clicked()
        {
            do_create_group = true;
            ui.close_menu();
        }
        ui.separator();
    }

    // ── Add to group ──────────────────────────────────────────────────────
    // Shown when this stream is ungrouped and at least one group exists.
    let candidate_groups: Vec<(Uuid, String)> = if stream_group_id.is_none() {
        let mut gs: Vec<(Uuid, String)> = diagram
            .stream_groups
            .values()
            .map(|g| (g.id, g.name.clone()))
            .collect();
        gs.sort_by(|a, b| a.1.cmp(&b.1));
        gs
    } else {
        Vec::new()
    };
    let mut add_to_group_id: Option<Uuid> = None;
    if !candidate_groups.is_empty() {
        ui.menu_button("Add to group", |ui| {
            for (gid, gname) in &candidate_groups {
                if ui.button(gname.as_str()).clicked() {
                    add_to_group_id = Some(*gid);
                    ui.close_menu();
                }
            }
        });
    }

    // ── Remove from group ─────────────────────────────────────────────────
    let mut do_remove_from_group = false;
    if stream_group_id.is_some() {
        if ui.button("Remove from group").clicked() {
            do_remove_from_group = true;
            ui.close_menu();
        }
    }

    // ── Standard items ────────────────────────────────────────────────────
    if ui.button("Open Spec").clicked() {
        state.selected_id = Some(stream_id);
        state.active_panel = ActivePanel::Spec(stream_id);
        ui.close_menu();
    }

    if ui.button("Delete").clicked() {
        let cmd = DeleteStream { diagram_id: state.active_diagram_id, stream: stream_clone };
        if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
            state.set_status(format!("Delete stream failed: {e}"));
        } else {
            state.dirty = true;
            if state.selected_id == Some(stream_id) {
                state.selected_id = None;
            }
            state.validation.validate_incremental(src_id, &state.project);
            state.validation.validate_incremental(tgt_id, &state.project);
        }
        ui.close_menu();
    }

    // ── Deferred command dispatch ─────────────────────────────────────────
    if do_create_group {
        let ids: Vec<Uuid> = state.multi_selected_streams.iter().copied().collect();
        state.dialogs.create_group.open_for(ids);
        state.multi_selected_streams.clear();
    }

    if let Some(gid) = add_to_group_id {
        if let Some(group) = diagram.stream_groups.get(&gid) {
            let before = group.clone();
            let mut after = group.clone();
            after.add_stream(stream_id);
            let cmd = EditStreamGroup {
                diagram_id: state.active_diagram_id,
                group_id: gid,
                before,
                after,
            };
            if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                state.set_status(format!("Add to group failed: {e}"));
            } else {
                state.dirty = true;
            }
        }
    }

    if do_remove_from_group {
        if let Some(gid) = stream_group_id {
            if let Some(group) = diagram.stream_groups.get(&gid) {
                if group.len() <= 1 {
                    // Last member: dissolve the group entirely.
                    let cmd = DeleteStreamGroup {
                        diagram_id: state.active_diagram_id,
                        group: group.clone(),
                    };
                    if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                        state.set_status(format!("Remove from group failed: {e}"));
                    } else {
                        state.dirty = true;
                    }
                } else {
                    let before = group.clone();
                    let mut after = group.clone();
                    after.remove_stream(stream_id);
                    let cmd = EditStreamGroup {
                        diagram_id: state.active_diagram_id,
                        group_id: gid,
                        before,
                        after,
                    };
                    if let Err(e) = state.undo_stack.execute(Box::new(cmd), &mut state.project) {
                        state.set_status(format!("Remove from group failed: {e}"));
                    } else {
                        state.dirty = true;
                    }
                }
            }
        }
    }
}

/// Delete a node together with all streams that touch it.
fn delete_node_with_streams(
    state: &mut AppState,
    diagram: &Diagram,
    node_id: Uuid,
    node: &Node,
) {
    let diagram_id = state.active_diagram_id;
    let position = diagram.positions.get(&node_id).copied().unwrap_or((0.0, 0.0));

    let mut cmds: Vec<Box<dyn crate::undo::history::Command>> = Vec::new();

    // Delete all streams touching this node first (pre-condition for DeleteNode).
    for stream in diagram.streams.values().filter(|s| s.touches_node(node_id)) {
        cmds.push(Box::new(DeleteStream { diagram_id, stream: stream.clone() }));
    }
    cmds.push(Box::new(DeleteNode { diagram_id, node: node.clone(), position }));

    let compound = CompoundCommand::new("Delete node", cmds);
    if let Err(e) = state.undo_stack.execute(Box::new(compound), &mut state.project) {
        state.set_status(format!("Delete node failed: {e}"));
    } else {
        state.dirty = true;
        state.validation.validate_full(&state.project);
    }
}

