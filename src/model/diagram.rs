//! Diagram model (spec §5.1, §6).
//!
//! A [`Diagram`] is a single canvas: a collection of nodes, streams, and
//! stream groups together with layout information (positions, viewport).
//!
//! The root diagram is created automatically with a new project.  Sub-diagrams
//! are created by attaching one to a [`crate::model::node::NodeKind::Process`]
//! node.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{
    node::Node,
    stream::{Stream, StreamGroup},
};

// ---------------------------------------------------------------------------
// ViewportState
// ---------------------------------------------------------------------------

/// The saved camera state for a diagram canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportState {
    /// Zoom factor: 1.0 = 100%.
    pub zoom: f32,
    /// Canvas-space scroll offset (pan).
    pub scroll: (f32, f32),
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            scroll: (0.0, 0.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Diagram
// ---------------------------------------------------------------------------

/// A single canvas containing nodes, streams and groups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagram {
    /// Stable unique identifier.
    pub id: Uuid,
    /// User-visible name (shown in the tree pane and as the window title).
    pub name: String,
    /// All nodes on this canvas, keyed by their ID.
    pub nodes: HashMap<Uuid, Node>,
    /// All streams on this canvas, keyed by their ID.
    pub streams: HashMap<Uuid, Stream>,
    /// All stream groups on this canvas, keyed by their ID.
    pub stream_groups: HashMap<Uuid, StreamGroup>,
    /// Canvas-space `(x, y)` position for each node, keyed by node ID.
    pub positions: HashMap<Uuid, (f32, f32)>,
    /// Saved viewport / camera state.
    pub viewport: ViewportState,
    /// The process node in the *parent* diagram that owns this sub-diagram.
    /// `None` for the root diagram and for orphaned diagrams.
    pub parent_process_id: Option<Uuid>,
}

impl Diagram {
    /// Create a new, empty diagram.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            nodes: HashMap::new(),
            streams: HashMap::new(),
            stream_groups: HashMap::new(),
            positions: HashMap::new(),
            viewport: ViewportState::default(),
            parent_process_id: None,
        }
    }

    /// Add a node to this diagram at the given canvas position.
    pub fn add_node(&mut self, node: Node, position: (f32, f32)) {
        self.positions.insert(node.id, position);
        self.nodes.insert(node.id, node);
    }

    /// Remove a node and all streams connected to it.  Returns the IDs of
    /// any streams that were removed as a side-effect.
    pub fn remove_node(&mut self, node_id: Uuid) -> Vec<Uuid> {
        todo!("Phase 2 — Diagram::remove_node")
    }

    /// Add a stream to this diagram.
    pub fn add_stream(&mut self, stream: Stream) {
        self.streams.insert(stream.id, stream);
    }

    /// Remove a stream by ID.  Cleans up its group membership.
    pub fn remove_stream(&mut self, stream_id: Uuid) {
        todo!("Phase 2 — Diagram::remove_stream")
    }
}
