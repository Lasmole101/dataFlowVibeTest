//! Diagram model (spec §5.1, §6).
//!
//! A [`Diagram`] is a single canvas: a collection of nodes, streams, and
//! stream groups together with layout information (node positions, viewport).
//!
//! The root diagram is created automatically with a new project.  Sub-diagrams
//! are created when a process node is decomposed (spec §6.1).
//!
//! # Graph invariants maintained here
//! * Self-loops are rejected by [`Diagram::add_stream`].
//! * Removing a node marks all its connected streams as dangling and removes
//!   them from any groups, but does **not** delete the streams — the streams
//!   remain in the diagram in error state so validation can report them.
//! * Removing a stream cleans up its group membership and dissolves the group
//!   if it becomes empty.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{
    node::Node,
    stream::{Stream, StreamGroup},
};

// ---------------------------------------------------------------------------
// StreamError
// ---------------------------------------------------------------------------

/// Errors that can arise from stream or node operations at the diagram level.
#[derive(Debug, PartialEq, Eq)]
pub enum DiagramError {
    /// Attempted to connect a node to itself.
    SelfLoop(Uuid),
    /// The referenced node does not exist in this diagram.
    NodeNotFound(Uuid),
    /// The referenced stream does not exist in this diagram.
    StreamNotFound(Uuid),
}

impl std::fmt::Display for DiagramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiagramError::SelfLoop(id) => write!(f, "self-loop on node {id}"),
            DiagramError::NodeNotFound(id) => write!(f, "node {id} not found"),
            DiagramError::StreamNotFound(id) => write!(f, "stream {id} not found"),
        }
    }
}

// ---------------------------------------------------------------------------
// ViewportState
// ---------------------------------------------------------------------------

/// The saved camera state for a diagram canvas (spec §7 — persisted per
/// diagram).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportState {
    /// Zoom factor: 1.0 = 100 %.
    pub zoom: f32,
    /// Canvas-space scroll offset (pan), in canvas units.
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
// RemovedNode — return value of remove_node
// ---------------------------------------------------------------------------

/// The data returned when a node is removed from a diagram.
///
/// The caller (typically an undo command) needs all of this to reconstruct
/// the removed node and its previous graph connections.
#[derive(Debug, PartialEq)]
pub struct RemovedNode {
    /// The node that was removed.
    pub node: Node,
    /// Its canvas position at removal time.
    pub position: (f32, f32),
    /// Streams that were made dangling as a result of removing this node.
    /// The streams remain in the diagram in an error state; this list lets
    /// an undo command know which ones to un-dangle.
    pub dangled_stream_ids: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Diagram
// ---------------------------------------------------------------------------

/// A single canvas containing nodes, streams, and groups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagram {
    /// Stable unique identifier.
    pub id: Uuid,
    /// User-visible name (shown in the tree pane).
    pub name: String,
    /// All nodes on this canvas, keyed by their ID.
    pub nodes: HashMap<Uuid, Node>,
    /// All streams on this canvas, keyed by their ID.
    ///
    /// Streams that are dangling remain here in an error state; they are not
    /// removed when their endpoint node is deleted.
    pub streams: HashMap<Uuid, Stream>,
    /// All stream groups on this canvas, keyed by their ID.
    pub stream_groups: HashMap<Uuid, StreamGroup>,
    /// Canvas-space `(x, y)` position for each node, keyed by node ID.
    pub positions: HashMap<Uuid, (f32, f32)>,
    /// Saved viewport / camera state (persisted per diagram, spec §7 open
    /// question #3 resolved as: yes, save per diagram).
    pub viewport: ViewportState,
    /// The process node in the *parent* diagram that owns this sub-diagram.
    ///
    /// `None` for the root diagram and for orphaned sub-diagrams.
    pub parent_process_id: Option<Uuid>,
}

impl Diagram {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Node operations
    // -----------------------------------------------------------------------

    /// Add a node to this diagram at the given canvas-space position.
    pub fn add_node(&mut self, node: Node, position: (f32, f32)) {
        self.positions.insert(node.id, position);
        self.nodes.insert(node.id, node);
    }

    /// Remove a node from the diagram.
    ///
    /// All streams that reference this node as their source or target are
    /// **marked dangling** rather than deleted — they remain visible on the
    /// canvas in an error state so the user can see the broken connections.
    /// Each affected stream is also removed from its display group (and the
    /// group is dissolved if it becomes empty).
    ///
    /// Returns a [`RemovedNode`] that contains everything needed for undo.
    ///
    /// # Errors
    /// Returns [`DiagramError::NodeNotFound`] if `node_id` is not present.
    pub fn remove_node(&mut self, node_id: Uuid) -> Result<RemovedNode, DiagramError> {
        let node = self
            .nodes
            .remove(&node_id)
            .ok_or(DiagramError::NodeNotFound(node_id))?;
        let position = self.positions.remove(&node_id).unwrap_or((0.0, 0.0));

        // Mark every stream that touched the removed node as dangling, and
        // evict it from any group it belongs to.
        let mut dangled_stream_ids = Vec::new();
        for stream in self.streams.values_mut() {
            if stream.touches_node(node_id) {
                stream.mark_dangling();
                // Remove from group; dissolve group if now empty.
                if let Some(group_id) = stream.group_id {
                    if let Some(group) = self.stream_groups.get_mut(&group_id) {
                        group.remove_stream(stream.id);
                        // Mark group for dissolution (can't remove here while
                        // iterating streams; handled after the loop).
                    }
                    stream.clear_group();
                }
                dangled_stream_ids.push(stream.id);
            }
        }

        // Dissolve groups that became empty as a side-effect.
        self.stream_groups.retain(|_, g| !g.is_empty());

        Ok(RemovedNode {
            node,
            position,
            dangled_stream_ids,
        })
    }

    // -----------------------------------------------------------------------
    // Stream operations
    // -----------------------------------------------------------------------

    /// Add a stream to this diagram.
    ///
    /// # Errors
    /// Returns [`DiagramError::SelfLoop`] if source and target are the same
    /// node.
    pub fn add_stream(&mut self, stream: Stream) -> Result<(), DiagramError> {
        if stream.source_node_id == stream.target_node_id {
            return Err(DiagramError::SelfLoop(stream.source_node_id));
        }
        self.streams.insert(stream.id, stream);
        Ok(())
    }

    /// Remove a stream by ID.
    ///
    /// Removes the stream from any group it belongs to.  If that empties the
    /// group, the group is dissolved.
    ///
    /// Returns the removed stream.
    ///
    /// # Errors
    /// Returns [`DiagramError::StreamNotFound`] if `stream_id` is not present.
    pub fn remove_stream(&mut self, stream_id: Uuid) -> Result<Stream, DiagramError> {
        let stream = self
            .streams
            .remove(&stream_id)
            .ok_or(DiagramError::StreamNotFound(stream_id))?;

        // Clean up group membership.
        if let Some(group_id) = stream.group_id {
            if let Some(group) = self.stream_groups.get_mut(&group_id) {
                group.remove_stream(stream_id);
                if group.is_empty() {
                    self.stream_groups.remove(&group_id);
                }
            }
        }

        Ok(stream)
    }

    // -----------------------------------------------------------------------
    // Group operations
    // -----------------------------------------------------------------------

    /// Create a display group from a set of existing stream IDs.
    ///
    /// All stream IDs must already exist in this diagram.  The streams are
    /// updated to record their new `group_id`.
    ///
    /// Returns the new group's ID.
    ///
    /// # Errors
    /// Returns [`DiagramError::StreamNotFound`] for the first unknown stream.
    pub fn create_group(
        &mut self,
        name: impl Into<String>,
        stream_ids: Vec<Uuid>,
    ) -> Result<Uuid, DiagramError> {
        // Validate all IDs exist before mutating anything.
        for &sid in &stream_ids {
            if !self.streams.contains_key(&sid) {
                return Err(DiagramError::StreamNotFound(sid));
            }
        }
        let group = StreamGroup::new(name, stream_ids.clone());
        let group_id = group.id;
        // Update each stream's group_id.
        for sid in &stream_ids {
            if let Some(stream) = self.streams.get_mut(sid) {
                stream.set_group(group_id);
            }
        }
        self.stream_groups.insert(group_id, group);
        Ok(group_id)
    }

    /// Add a stream to an existing group.
    ///
    /// # Errors
    /// * [`DiagramError::StreamNotFound`] — stream ID unknown.
    /// * [`DiagramError::StreamNotFound`] (reused) — group ID unknown
    ///   (groups share the same UUID space as streams for error reporting;
    ///   callers can distinguish by the context).
    pub fn add_stream_to_group(
        &mut self,
        stream_id: Uuid,
        group_id: Uuid,
    ) -> Result<(), DiagramError> {
        if !self.streams.contains_key(&stream_id) {
            return Err(DiagramError::StreamNotFound(stream_id));
        }
        // Remove from any previous group first.
        if let Some(stream) = self.streams.get(&stream_id) {
            if let Some(old_gid) = stream.group_id {
                if let Some(old_group) = self.stream_groups.get_mut(&old_gid) {
                    old_group.remove_stream(stream_id);
                    if old_group.is_empty() {
                        self.stream_groups.remove(&old_gid);
                    }
                }
            }
        }
        let group = self
            .stream_groups
            .get_mut(&group_id)
            .ok_or(DiagramError::StreamNotFound(group_id))?;
        group.add_stream(stream_id);
        self.streams
            .get_mut(&stream_id)
            .unwrap()
            .set_group(group_id);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Streams whose source is `node_id`.
    pub fn outgoing_streams(&self, node_id: Uuid) -> impl Iterator<Item = &Stream> {
        self.streams
            .values()
            .filter(move |s| s.source_node_id == node_id)
    }

    /// Streams whose target is `node_id`.
    pub fn incoming_streams(&self, node_id: Uuid) -> impl Iterator<Item = &Stream> {
        self.streams
            .values()
            .filter(move |s| s.target_node_id == node_id)
    }

    /// All streams that touch `node_id` (either as source or target).
    pub fn streams_for_node(&self, node_id: Uuid) -> impl Iterator<Item = &Stream> {
        self.streams
            .values()
            .filter(move |s| s.touches_node(node_id))
    }

    /// Look up a node by ID.
    pub fn node(&self, id: Uuid) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Look up a stream by ID.
    pub fn stream(&self, id: Uuid) -> Option<&Stream> {
        self.streams.get(&id)
    }

    /// The canvas position of a node, if it exists.
    pub fn position_of(&self, node_id: Uuid) -> Option<(f32, f32)> {
        self.positions.get(&node_id).copied()
    }
}
