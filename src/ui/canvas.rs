//! Canvas drawing and interaction layer (spec §5.1, phases 8a–8b).
//!
//! Renders the active diagram's nodes, streams, and validation badges using
//! the egui [`Painter`] API.  Phase 8b adds zoom, pan, and selection.
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
//! Inverse:   `canvas = screen / zoom - panel_origin / zoom + scroll`
//!            (i.e. `cx = (screen.x - origin.x) / zoom + scroll.x`)
//!
//! # Node layout
//!
//! Each node is a fixed 120×60 canvas-unit rounded rectangle.  The position
//! stored in `diagram.positions` is the **top-left corner** in canvas space.
//!
//! # Stream routing
//!
//! Streams are drawn as two-segment orthogonal (L-shaped) paths:
//! * Segment 1: horizontal from the source node's right-edge midpoint to the
//!   horizontal midpoint between source and target.
//! * Segment 2: vertical from that midpoint down/up to the target's left-edge
//!   midpoint y, then horizontal to the target.
//!
//! When the source centre is to the right of the target centre the routing
//! exits from the source bottom and enters the target top instead, to avoid
//! a degenerate backward arrow.
//!
//! # Stream groups
//!
//! Grouped streams are drawn with a small perpendicular offset so they appear
//! as parallel lines rather than overlapping.  The group membership index
//! determines the offset direction.
//!
//! # Validation badges
//!
//! A small filled circle is drawn at the top-right corner of each node and
//! at the midpoint of each stream:
//! * Amber  — `ValidationState::Warning`
//! * Red    — `ValidationState::Error`
//! * Nothing — `ValidationState::Ok`
//!
//! # Selection (phase 8b)
//!
//! Clicking a node or stream selects it (stores the UUID in
//! [`AppState::selected_id`]).  The selected component is drawn with a
//! cyan highlight outline.  Clicking empty canvas deselects.
//! Double-clicking or pressing Enter opens the spec panel for the selection.
//!
//! # Zoom and pan (phase 8b)
//!
//! * **Zoom**: scroll wheel scales around the mouse cursor.  Stored in
//!   [`ViewportState::zoom`] and applied immediately on the next frame.
//! * **Pan**: middle-mouse drag or Space+left-drag moves the viewport.
//!   Stored in [`ViewportState::scroll`].

use std::collections::HashMap;

use egui::{Color32, CursorIcon, FontId, Painter, Pos2, Rect, Rounding, Stroke, Vec2};
use uuid::Uuid;

use crate::{
    app::{ActivePanel, AppState},
    model::{diagram::Diagram, node::NodeKind},
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
const GROUP_OFFSET: f32 = 5.0;

/// Badge circle radius.
const BADGE_R: f32 = 6.0;
/// Amber colour for warnings.
const AMBER: Color32 = Color32::from_rgb(220, 170, 50);
/// Red colour for errors.
const ERROR_RED: Color32 = Color32::from_rgb(220, 70, 70);
/// Selection highlight colour.
const SELECT_COLOR: Color32 = Color32::from_rgb(80, 180, 255);
/// Tolerance (screen pixels) for clicking a stream polyline.
const STREAM_HIT_TOLERANCE: f32 = 6.0;
/// Zoom step per one scroll-wheel notch (120 scroll units).
const ZOOM_NOTCH: f32 = 120.0;
/// Multiplicative zoom change per scroll notch.
const ZOOM_FACTOR_PER_NOTCH: f32 = 1.15;
/// Hard zoom bounds.
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 20.0;

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
// Public entry point
// ---------------------------------------------------------------------------

/// Render the active diagram into the central panel, handling zoom/pan/selection.
pub fn show(state: &mut AppState, ui: &mut egui::Ui) {
    let panel_rect = ui.available_rect_before_wrap();

    // Allocate the full panel as an interactive widget (primary-button click +
    // drag).  Middle-mouse events are read from the global InputState directly.
    let response = ui.allocate_rect(panel_rect, egui::Sense::click_and_drag());

    // Clone the active diagram so we can release the project borrow before the
    // interaction code needs `&mut state`.
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

            // Keep the canvas point currently under the cursor fixed.
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

        // Latch: pan only if the drag started while the pointer was in the canvas.
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
    {
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
    }

    // -----------------------------------------------------------------------
    // Build transform with (potentially updated) zoom / scroll
    // -----------------------------------------------------------------------
    let xform = Transform {
        zoom,
        scroll,
        origin: panel_rect.min,
    };

    // -----------------------------------------------------------------------
    // Collect draw data — positions are now in screen space
    // -----------------------------------------------------------------------
    let mut draw_data = collect_draw_data(&diagram, &xform, state.selected_id);
    apply_badges(&mut draw_data, state);

    // -----------------------------------------------------------------------
    // Hit-test and selection (single click)
    // -----------------------------------------------------------------------
    // Read interact_pos inside the closures to avoid multiple borrows of `ui`.
    let interact_pos = ui.input(|i| i.pointer.interact_pos());

    if response.double_clicked() {
        // Double-click: select the hit component and open its spec panel.
        let hit = interact_pos.and_then(|p| hit_test(&draw_data, p));
        state.selected_id = hit;
        if let Some(id) = hit {
            state.active_panel = ActivePanel::Spec(id);
            mark_selected(&mut draw_data, id);
        }
    } else if response.clicked() {
        // Single click: update selection; deselect on empty canvas.
        let hit = interact_pos.and_then(|p| hit_test(&draw_data, p));
        state.selected_id = hit;
        if let Some(id) = hit {
            mark_selected(&mut draw_data, id);
        }
    } else {
        // No click this frame — just keep the existing selection highlighted.
        if let Some(id) = state.selected_id {
            mark_selected(&mut draw_data, id);
        }
    }

    // -----------------------------------------------------------------------
    // Enter key: open spec panel for the currently selected component
    // -----------------------------------------------------------------------
    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some(id) = state.selected_id {
            state.active_panel = ActivePanel::Spec(id);
        }
    }

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

    // Background fill.
    painter.rect_filled(
        panel_rect,
        0.0,
        if dark_mode {
            Color32::from_rgb(30, 30, 35)
        } else {
            Color32::from_rgb(245, 245, 248)
        },
    );

    // Grid dots.
    draw_grid(&painter, panel_rect, &xform, dark_mode);

    // Streams beneath nodes.
    for sd in &draw_data.streams {
        draw_stream(&painter, sd, dark_mode, &draw_data.group_offsets);
    }

    // Nodes.
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
            draw_stream_selection(&painter, sd, &draw_data.group_offsets);
        }
    }

    // Validation badges on top.
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
    /// Convert a canvas-space position to screen-space.
    fn to_screen(&self, canvas: (f32, f32)) -> Pos2 {
        let cx = (canvas.0 - self.scroll.x) * self.zoom;
        let cy = (canvas.1 - self.scroll.y) * self.zoom;
        self.origin + Vec2::new(cx, cy)
    }

    /// Convert a screen-space position to canvas-space.
    fn to_canvas(&self, screen: Pos2) -> (f32, f32) {
        let sx = screen.x - self.origin.x;
        let sy = screen.y - self.origin.y;
        (sx / self.zoom + self.scroll.x, sy / self.zoom + self.scroll.y)
    }

    /// Scale a canvas-space length to screen-space.
    fn scale(&self, v: f32) -> f32 {
        v * self.zoom
    }
}

// ---------------------------------------------------------------------------
// Draw data structures
// ---------------------------------------------------------------------------

struct NodeDrawData {
    node_id: Uuid,
    rect: Rect, // screen-space rect
    kind: NodeKind,
    name: String,
    is_boundary: bool,
    badge: ValidationState,
    selected: bool,
}

struct StreamDrawData {
    stream_id: Uuid,
    /// Polyline points in screen space (3 or 4 points for an L-shape).
    points: Vec<Pos2>,
    /// Midpoint of the path — badge placement.
    midpoint: Pos2,
    badge: ValidationState,
    /// Which group this stream belongs to (None if ungrouped).
    group_id: Option<Uuid>,
    selected: bool,
}

struct DiagramDrawData {
    nodes: Vec<NodeDrawData>,
    streams: Vec<StreamDrawData>,
    /// group_id → Vec of stream_ids in order (for offset computation).
    group_offsets: HashMap<Uuid, Vec<Uuid>>,
}

// ---------------------------------------------------------------------------
// Data collection
// ---------------------------------------------------------------------------

fn collect_draw_data(
    diagram: &Diagram,
    xform: &Transform,
    selected_id: Option<Uuid>,
) -> DiagramDrawData {
    // Build group membership map: group_id → ordered stream ids.
    let mut group_offsets: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (gid, group) in &diagram.stream_groups {
        group_offsets.insert(*gid, group.member_stream_ids.clone());
    }

    // Build a lookup of node screen rects for stream endpoint computation.
    let node_rects: HashMap<Uuid, Rect> = diagram
        .nodes
        .keys()
        .filter_map(|&nid| {
            let pos = diagram.positions.get(&nid).copied().unwrap_or((0.0, 0.0));
            let tl = xform.to_screen(pos);
            let br = tl + Vec2::new(xform.scale(NODE_W), xform.scale(NODE_H));
            Some((nid, Rect::from_min_max(tl, br)))
        })
        .collect();

    // Collect node draw data.
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
                selected: selected_id == Some(node.id),
            }
        })
        .collect();
    nodes.sort_by_key(|n| n.node_id);

    // Collect stream draw data.
    let mut streams: Vec<StreamDrawData> = diagram
        .streams
        .values()
        .map(|stream| {
            let src_rect = node_rects.get(&stream.source_node_id).copied();
            let tgt_rect = node_rects.get(&stream.target_node_id).copied();
            let (points, midpoint) = compute_stream_path(src_rect, tgt_rect);
            StreamDrawData {
                stream_id: stream.id,
                points,
                midpoint,
                badge: ValidationState::Ok,
                group_id: stream.group_id,
                selected: selected_id == Some(stream.id),
            }
        })
        .collect();
    streams.sort_by_key(|s| s.stream_id);

    DiagramDrawData {
        nodes,
        streams,
        group_offsets,
    }
}

/// Attach validation badge states — done as a separate pass to keep
/// `collect_draw_data` free of validation imports.
pub fn apply_badges(data: &mut DiagramDrawData, state: &AppState) {
    for nd in &mut data.nodes {
        nd.badge = state.validation.state(nd.node_id);
    }
    for sd in &mut data.streams {
        sd.badge = state.validation.state(sd.stream_id);
    }
}

/// Mark the draw-data entry with the given UUID as selected.
fn mark_selected(data: &mut DiagramDrawData, id: Uuid) {
    for nd in &mut data.nodes {
        nd.selected = nd.node_id == id;
    }
    for sd in &mut data.streams {
        sd.selected = sd.stream_id == id;
    }
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

/// Return the UUID of the topmost node or stream under `pos` (screen space),
/// or `None` if nothing was hit.  Nodes are tested before streams (drawn on
/// top).
fn hit_test(draw_data: &DiagramDrawData, pos: Pos2) -> Option<Uuid> {
    for nd in &draw_data.nodes {
        if nd.rect.contains(pos) {
            return Some(nd.node_id);
        }
    }
    for sd in &draw_data.streams {
        if hit_test_polyline(pos, &sd.points, STREAM_HIT_TOLERANCE) {
            return Some(sd.stream_id);
        }
    }
    None
}

/// Return `true` if `pos` is within `tolerance` screen pixels of any segment
/// in the polyline.
fn hit_test_polyline(pos: Pos2, points: &[Pos2], tolerance: f32) -> bool {
    for w in points.windows(2) {
        if dist_point_to_segment(pos, w[0], w[1]) <= tolerance {
            return true;
        }
    }
    false
}

/// Shortest distance from point `p` to the line segment `[a, b]`.
fn dist_point_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let ap = p - a;
    let len_sq = ab.length_sq();
    if len_sq < f32::EPSILON {
        return ap.length();
    }
    let t = ap.dot(ab) / len_sq;
    let closest = a + ab * t.clamp(0.0, 1.0);
    (p - closest).length()
}

// ---------------------------------------------------------------------------
// Stream path computation
// ---------------------------------------------------------------------------

/// Compute the screen-space polyline for a stream.
///
/// Returns (points, midpoint).  `points` has 4 entries for a full L-shape:
/// `[src_exit, h_bend, v_bend, tgt_enter]`.
///
/// If either endpoint rect is missing (dangling stream) a degenerate
/// horizontal line is returned at the panel origin.
fn compute_stream_path(src_rect: Option<Rect>, tgt_rect: Option<Rect>) -> (Vec<Pos2>, Pos2) {
    let (src_r, tgt_r) = match (src_rect, tgt_rect) {
        (Some(s), Some(t)) => (s, t),
        _ => {
            let p = Pos2::ZERO;
            return (vec![p, p, p, p], p);
        }
    };

    let src_cx = src_r.center().x;
    let tgt_cx = tgt_r.center().x;

    let (src_exit, tgt_entry) = if src_cx <= tgt_cx {
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

    let points = vec![src_exit, h_bend, v_bend, tgt_entry];
    let midpoint = h_bend.lerp(v_bend, 0.5);

    (points, midpoint)
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

    // Align first dot to the grid in canvas space, then convert to screen.
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

    if nd.is_boundary {
        painter.rect_filled(nd.rect, rounding, fill);
        draw_dashed_rect(
            painter,
            nd.rect,
            rounding,
            Stroke::new(NODE_STROKE, stroke_color),
        );
    } else {
        painter.rect_filled(nd.rect, rounding, fill);
        painter.rect_stroke(nd.rect, rounding, Stroke::new(NODE_STROKE, stroke_color));
    }

    let label = format!("{}\n{}", nd.name, nd.kind.label());
    let text_color = if dark_mode {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    painter.text(
        nd.rect.center(),
        egui::Align2::CENTER_CENTER,
        &label,
        FontId::proportional(11.0),
        text_color,
    );
}

/// Draw a selection highlight around a node.
fn draw_node_selection(painter: &Painter, nd: &NodeDrawData) {
    let expanded = nd.rect.expand(3.0);
    painter.rect_stroke(
        expanded,
        Rounding::same(NODE_ROUNDING + 3.0),
        Stroke::new(2.0, SELECT_COLOR),
    );
}

/// Draw a dashed rectangle border (approximate — walks the perimeter).
fn draw_dashed_rect(painter: &Painter, rect: Rect, _rounding: Rounding, stroke: Stroke) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    for window in corners.windows(2) {
        let (a, b) = (window[0], window[1]);
        draw_dashed_segment(painter, a, b, stroke, DASH_LEN, DASH_GAP);
    }
}

/// Draw a dashed line segment from `a` to `b`.
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

fn draw_stream(
    painter: &Painter,
    sd: &StreamDrawData,
    dark_mode: bool,
    group_offsets: &HashMap<Uuid, Vec<Uuid>>,
) {
    let base_color = stream_color(dark_mode);
    let offset_vec = stream_offset(sd, group_offsets);
    let points: Vec<Pos2> = sd.points.iter().map(|&p| p + offset_vec).collect();

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

/// Draw a selection highlight along a stream's path.
fn draw_stream_selection(
    painter: &Painter,
    sd: &StreamDrawData,
    group_offsets: &HashMap<Uuid, Vec<Uuid>>,
) {
    let offset_vec = stream_offset(sd, group_offsets);
    let points: Vec<Pos2> = sd.points.iter().map(|&p| p + offset_vec).collect();
    let stroke = Stroke::new(STREAM_STROKE + 2.0, SELECT_COLOR);
    for window in points.windows(2) {
        painter.line_segment([window[0], window[1]], stroke);
    }
}

/// Compute the perpendicular group offset for a stream, or `Vec2::ZERO`.
fn stream_offset(sd: &StreamDrawData, group_offsets: &HashMap<Uuid, Vec<Uuid>>) -> Vec2 {
    if let Some(gid) = sd.group_id {
        compute_group_offset(sd.stream_id, gid, group_offsets, &sd.points)
    } else {
        Vec2::ZERO
    }
}

/// Draw a small filled triangle arrowhead at `tip`, pointing from `from` → `tip`.
fn draw_arrowhead(painter: &Painter, from: Pos2, tip: Pos2, half: f32, color: Color32) {
    let dir = (tip - from).normalized();
    let perp = Vec2::new(-dir.y, dir.x);
    let base = tip - dir * (half * 1.5);
    let left = base + perp * half;
    let right = base - perp * half;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, left, right],
        color,
        Stroke::NONE,
    ));
}

/// Compute the perpendicular offset for a grouped stream.
fn compute_group_offset(
    stream_id: Uuid,
    group_id: Uuid,
    group_offsets: &HashMap<Uuid, Vec<Uuid>>,
    points: &[Pos2],
) -> Vec2 {
    let members = match group_offsets.get(&group_id) {
        Some(m) => m,
        None => return Vec2::ZERO,
    };
    let slot = members.iter().position(|&id| id == stream_id).unwrap_or(0) as f32;
    let n = members.len() as f32;
    let centred_slot = slot - (n - 1.0) * 0.5;

    if points.len() < 2 {
        return Vec2::ZERO;
    }
    let seg_dir = (points[1] - points[0]).normalized();
    let perp = Vec2::new(-seg_dir.y, seg_dir.x);
    perp * (centred_slot * GROUP_OFFSET)
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
