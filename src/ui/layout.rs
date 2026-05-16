//! Automatic graph layout for a [`Diagram`] (spec §9b).
//!
//! Uses a simple layered-grid algorithm:
//! 1. Topological sort via Kahn's algorithm; back-edges from cycles are ignored.
//! 2. Assign each node a column equal to its longest incoming-path length.
//! 3. Within each column, assign rows in BFS encounter order.
//! 4. `x = col * (NODE_W + H_GAP)`,  `y = row * (NODE_H + V_GAP)`.
//!
//! The result is returned as a [`CompoundCommand`] so it is undoable in one step.

use std::collections::{HashMap, VecDeque};

use uuid::Uuid;

use crate::{
    model::diagram::Diagram,
    undo::{
        commands::MoveNode,
        history::{Command, CompoundCommand},
    },
};

/// Canvas node dimensions — kept in sync with `ui::canvas`.
const NODE_W: f32 = 120.0;
/// Canvas node dimensions — kept in sync with `ui::canvas`.
const NODE_H: f32 = 60.0;
/// Horizontal gap between columns.
const H_GAP: f32 = 80.0;
/// Vertical gap between rows.
const V_GAP: f32 = 40.0;

/// Compute auto-layout positions for all nodes in `diagram`.
///
/// Returns a map of node UUID → canvas-space `(x, y)` top-left corner.
/// Dangling streams are ignored when computing the graph topology.
pub fn auto_layout(diagram: &Diagram) -> HashMap<Uuid, (f32, f32)> {
    let node_ids: Vec<Uuid> = diagram.nodes.keys().copied().collect();

    if node_ids.is_empty() {
        return HashMap::new();
    }

    // ── Build directed adjacency and in-degree tables ─────────────────────
    let mut successors: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    let mut in_degree: HashMap<Uuid, usize> = HashMap::new();

    for &id in &node_ids {
        successors.entry(id).or_default();
        in_degree.entry(id).or_insert(0);
    }

    for stream in diagram.streams.values() {
        if stream.is_dangling {
            continue;
        }
        let src = stream.source_node_id;
        let tgt = stream.target_node_id;
        if diagram.nodes.contains_key(&src) && diagram.nodes.contains_key(&tgt) {
            successors.entry(src).or_default().push(tgt);
            *in_degree.entry(tgt).or_insert(0) += 1;
        }
    }

    // ── Kahn's BFS — also tracks longest-path (column) per node ──────────
    //
    // `column[id]` = max over all predecessors of (predecessor_col + 1).
    // Seeded to 0 for source nodes.
    let mut column: HashMap<Uuid, usize> = HashMap::new();
    let mut queue: VecDeque<Uuid> = VecDeque::new();
    let mut topo_order: Vec<Uuid> = Vec::new();

    for &id in &node_ids {
        if in_degree[&id] == 0 {
            column.insert(id, 0);
            queue.push_back(id);
        }
    }

    while let Some(id) = queue.pop_front() {
        topo_order.push(id);
        let col = column[&id];

        for &succ in &successors[&id] {
            let new_col = col + 1;
            let entry = column.entry(succ).or_insert(0);
            if new_col > *entry {
                *entry = new_col;
            }

            let deg = in_degree.get_mut(&succ).unwrap();
            *deg -= 1;
            if *deg == 0 {
                queue.push_back(succ);
            }
        }
    }

    // Nodes involved in cycles were skipped by Kahn's — place them in the
    // column after all acyclic nodes.
    let max_acyclic_col = column.values().copied().max().unwrap_or(0);
    let cycle_col = if column.len() < node_ids.len() {
        max_acyclic_col + 1
    } else {
        max_acyclic_col
    };

    for &id in &node_ids {
        if !column.contains_key(&id) {
            column.insert(id, cycle_col);
            topo_order.push(id);
        }
    }

    // ── Assign rows within each column (in topo encounter order) ─────────
    let mut row_counter: HashMap<usize, usize> = HashMap::new();
    let mut row: HashMap<Uuid, usize> = HashMap::new();

    for &id in &topo_order {
        let col = column[&id];
        let r = row_counter.entry(col).or_insert(0);
        row.insert(id, *r);
        *r += 1;
    }

    // ── Convert column/row to canvas (x, y), then centre around (0, 0) ────
    let col_step = NODE_W + H_GAP;
    let row_step = NODE_H + V_GAP;

    // Raw positions (top-left corner of each node box).
    let raw: HashMap<Uuid, (f32, f32)> = node_ids
        .iter()
        .map(|&id| {
            let x = column[&id] as f32 * col_step;
            let y = row[&id] as f32 * row_step;
            (id, (x, y))
        })
        .collect();

    // Bounding-box of all node top-left corners + node size.
    let min_x = raw.values().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let min_y = raw.values().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let max_x = raw.values().map(|p| p.0 + NODE_W).fold(f32::NEG_INFINITY, f32::max);
    let max_y = raw.values().map(|p| p.1 + NODE_H).fold(f32::NEG_INFINITY, f32::max);
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;

    // Shift every position so the bounding-box centre lands at canvas (0, 0).
    raw.into_iter().map(|(id, (x, y))| (id, (x - cx, y - cy))).collect()
}

/// Build a [`CompoundCommand`] that moves every node in `diagram` to its
/// auto-layout position, storing the original positions so the operation
/// can be undone.
///
/// Returns an empty compound (no sub-commands) when the diagram has no nodes.
pub fn auto_layout_command(diagram: &Diagram) -> CompoundCommand {
    let new_positions = auto_layout(diagram);
    let diagram_id = diagram.id;

    let commands: Vec<Box<dyn Command>> = new_positions
        .into_iter()
        .map(|(node_id, to)| {
            let from = diagram
                .positions
                .get(&node_id)
                .copied()
                .unwrap_or((0.0, 0.0));
            Box::new(MoveNode {
                diagram_id,
                node_id,
                from,
                to,
            }) as Box<dyn Command>
        })
        .collect();

    CompoundCommand::new("Auto Layout", commands)
}
