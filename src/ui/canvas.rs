//! Canvas drawing layer (spec §5.1, phase 8a).
//!
//! Renders the active diagram's nodes, streams, and validation badges using
//! the egui [`Painter`] API.  Interaction (selection, drag, stream creation)
//! is added in phase 8b.
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

use std::collections::HashMap;

use egui::{Color32, FontId, Painter, Pos2, Rect, Rounding, Stroke, Vec2};
use uuid::Uuid;

use crate::{
    app::AppState,
    model::{
        diagram::Diagram,
        node::{Node, NodeKind},
        stream::Stream,
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
const GROUP_OFFSET: f32 = 5.0;

/// Badge circle radius.
const BADGE_R: f32 = 6.0;
/// Amber colour for warnings.
const AMBER: Color32 = Color32::from_rgb(220, 170, 50);
/// Red colour for errors.
const ERROR_RED: Color32 = Color32::from_rgb(220, 70, 70);

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

/// Render the active diagram into the central panel.
pub fn show(state: &mut AppState, ui: &mut egui::Ui) {
    // Clone the diagram so we can release the project borrow before calling
    // apply_badges (which needs &AppState for validation).
    let diagram = match state.project.diagrams.get(&state.active_diagram_id) {
        Some(d) => d.clone(),
        None => {
            ui.centered_and_justified(|ui| {
                ui.label("No diagram selected.");
            });
            return;
        }
    };

    let viewport = diagram.viewport.clone();
    let dark_mode = ui.visuals().dark_mode;
    let panel_rect = ui.available_rect_before_wrap();

    let xform = Transform {
        zoom: viewport.zoom,
        scroll: Vec2::new(viewport.scroll.0, viewport.scroll.1),
        origin: panel_rect.min,
    };

    // `diagram` is now owned — no outstanding borrow of `state.project`.
    let mut draw_data = collect_draw_data(&diagram, &xform);

    // Apply validation badges (uses state.validation — disjoint from project).
    apply_badges(&mut draw_data, state);

    let painter = ui.painter_at(panel_rect);

    // Fill background.
    painter.rect_filled(
        panel_rect,
        0.0,
        if dark_mode {
            Color32::from_rgb(30, 30, 35)
        } else {
            Color32::from_rgb(245, 245, 248)
        },
    );

    // Draw grid dots (light).
    draw_grid(&painter, panel_rect, &xform, dark_mode);

    // Draw streams beneath nodes.
    for sd in &draw_data.streams {
        draw_stream(&painter, sd, dark_mode, &draw_data.group_offsets);
    }

    // Draw nodes on top.
    for nd in &draw_data.nodes {
        draw_node(&painter, nd, dark_mode);
    }

    // Draw validation badges on top of everything.
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
// Transform: canvas → screen
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

fn collect_draw_data(diagram: &Diagram, xform: &Transform) -> DiagramDrawData {
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
                badge: ValidationState::Ok, // filled below
            }
        })
        .collect();
    // Sort nodes for deterministic draw order.
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
    let fallback = Vec2::ZERO;

    let (src_r, tgt_r) = match (src_rect, tgt_rect) {
        (Some(s), Some(t)) => (s, t),
        _ => {
            let p = Pos2::ZERO;
            return (vec![p, p, p, p], p);
        }
    };

    let src_cx = src_r.center().x;
    let tgt_cx = tgt_r.center().x;

    // Choose exit/entry edges based on relative horizontal position.
    let (src_exit, tgt_entry) = if src_cx <= tgt_cx {
        // Source is left of (or equal to) target: exit right, enter left.
        (
            Pos2::new(src_r.max.x, src_r.center().y),
            Pos2::new(tgt_r.min.x, tgt_r.center().y),
        )
    } else {
        // Source is right of target: exit bottom, enter top.
        (
            Pos2::new(src_r.center().x, src_r.max.y),
            Pos2::new(tgt_r.center().x, tgt_r.min.y),
        )
    };

    // L-shape bend: go horizontal first, then vertical.
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

    let grid_spacing = xform.scale(40.0); // 40 canvas units between dots
    if grid_spacing < 8.0 {
        return;
    } // too zoomed out to show dots

    let start_x = (rect.min.x / grid_spacing).floor() * grid_spacing;
    let start_y = (rect.min.y / grid_spacing).floor() * grid_spacing;

    let mut x = start_x;
    while x <= rect.max.x {
        let mut y = start_y;
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
        // Boundary node: filled rect with a dashed border.
        painter.rect_filled(nd.rect, rounding, fill);
        draw_dashed_rect(
            painter,
            nd.rect,
            rounding,
            Stroke::new(NODE_STROKE, stroke_color),
        );
    } else {
        // Normal node: solid border.
        painter.rect_filled(nd.rect, rounding, fill);
        painter.rect_stroke(nd.rect, rounding, Stroke::new(NODE_STROKE, stroke_color));
    }

    // Node label: "Name\nKind"
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

/// Draw a dashed rectangle border (approximate — walks the perimeter).
fn draw_dashed_rect(painter: &Painter, rect: Rect, _rounding: Rounding, stroke: Stroke) {
    // Walk the perimeter clockwise: top → right → bottom → left.
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(), // close
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

    // Compute perpendicular offset for group members.
    let offset_vec = if let Some(gid) = sd.group_id {
        compute_group_offset(sd.stream_id, gid, group_offsets, &sd.points)
    } else {
        Vec2::ZERO
    };

    let points: Vec<Pos2> = sd.points.iter().map(|&p| p + offset_vec).collect();

    if points.len() < 2 {
        return;
    }

    // Draw the polyline.
    let stroke = Stroke::new(STREAM_STROKE, base_color);
    for window in points.windows(2) {
        painter.line_segment([window[0], window[1]], stroke);
    }

    // Draw arrowhead at the last point, pointing toward it from the second-to-last.
    if let (Some(&tip), Some(&before)) = (points.last(), points.get(points.len().saturating_sub(2)))
    {
        draw_arrowhead(painter, before, tip, ARROW_HALF, base_color);
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
///
/// Returns a `Vec2` perpendicular to the first segment of the stream path,
/// scaled by `GROUP_OFFSET * slot_index`.  The slot index is the stream's
/// position in the group's member list.
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
    // Centre the group around zero offset: slot 0 → -n/2, slot n-1 → +n/2.
    let n = members.len() as f32;
    let centred_slot = slot - (n - 1.0) * 0.5;

    // Perpendicular to the first segment.
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
    // White icon character.
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
