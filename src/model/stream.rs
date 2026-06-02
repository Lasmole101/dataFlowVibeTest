//! Stream and stream-group model (spec §3.2–3.3).
//!
//! A [`Stream`] is a directed, point-to-point connection between two nodes.
//! Streams carry a [`TypeSpec`] that may be edited independently of the nodes
//! at either end; the stream spec is the authoritative description of what
//! data actually flows along that edge.
//!
//! [`StreamGroup`] is a **display-only** construct.  Grouped streams are
//! rendered as parallel lines on the canvas but grouping has no effect on
//! validation semantics (spec §3.3).
//!
//! # Dangling streams
//! A stream becomes dangling when its source or target node is deleted.  The
//! `is_dangling` flag is set by [`crate::model::diagram::Diagram::remove_node`]
//! rather than by the stream itself, because the stream does not know whether
//! its endpoint IDs still resolve.  Dangling streams display an error badge
//! (spec §4.3).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::types::TypeSpec;

// ---------------------------------------------------------------------------
// Stream
// ---------------------------------------------------------------------------

/// A directed connection between two nodes on the canvas.
///
/// Multiple streams may share a source or target node (fan-out and fan-in are
/// both permitted).  Cycles in the graph are permitted.  A stream may not
/// connect a node to itself — enforced at creation time in
/// [`crate::model::diagram::Diagram`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    /// Stable unique identifier.  Never changes for the lifetime of a stream.
    pub id: Uuid,
    /// The node from which data flows.
    pub source_node_id: Uuid,
    /// The node to which data flows.
    pub target_node_id: Uuid,
    /// The type specification carried by this stream.
    ///
    /// Editable independently of the source and target node specs; the
    /// validation layer compares this against node specs to produce
    /// balance warnings.
    pub type_spec: TypeSpec,
    /// The display group this stream belongs to, if any.
    pub group_id: Option<Uuid>,
    /// `true` when the source or target node has been deleted, leaving this
    /// stream without a valid endpoint (spec §4.3).
    pub is_dangling: bool,
}

impl Stream {
    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    /// Create a new stream between two nodes with an empty type spec.
    ///
    /// The caller is responsible for ensuring `source_node_id !=
    /// target_node_id` (self-loops are rejected by the diagram layer).
    pub fn new(source_node_id: Uuid, target_node_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            source_node_id,
            target_node_id,
            type_spec: TypeSpec::new(),
            group_id: None,
            is_dangling: false,
        }
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Returns `true` if `node_id` is either the source or the target of this
    /// stream.
    pub fn touches_node(&self, node_id: Uuid) -> bool {
        self.source_node_id == node_id || self.target_node_id == node_id
    }

    /// Returns `true` if this stream connects `a` to `b` (directed: source
    /// must be `a`, target must be `b`).
    pub fn connects(&self, a: Uuid, b: Uuid) -> bool {
        self.source_node_id == a && self.target_node_id == b
    }

    /// Returns `true` if this stream belongs to a display group.
    pub fn is_grouped(&self) -> bool {
        self.group_id.is_some()
    }

    // -----------------------------------------------------------------------
    // Mutations
    // -----------------------------------------------------------------------

    /// Mark this stream as dangling (called when an endpoint node is deleted).
    pub fn mark_dangling(&mut self) {
        self.is_dangling = true;
    }

    /// Assign this stream to a display group.
    ///
    /// Replaces any existing group membership.
    pub fn set_group(&mut self, group_id: Uuid) {
        self.group_id = Some(group_id);
    }

    /// Remove this stream from its display group, if any.
    ///
    /// Returns the previous group ID, or `None` if the stream was not grouped.
    pub fn clear_group(&mut self) -> Option<Uuid> {
        self.group_id.take()
    }
}

// ---------------------------------------------------------------------------
// StreamGroup
// ---------------------------------------------------------------------------

/// A named, display-only group of streams.
///
/// Groups are rendered as parallel lines on the canvas.  They carry no
/// semantic weight in validation (spec §3.3).
///
/// A group dissolves automatically when its last member stream is removed;
/// see [`StreamGroup::is_empty`] and
/// [`crate::model::diagram::Diagram::remove_stream`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamGroup {
    /// Stable unique identifier.
    pub id: Uuid,
    /// User-assigned display name.
    pub name: String,
    /// IDs of the streams that belong to this group, in insertion order.
    ///
    /// Invariant: no duplicates.
    pub member_stream_ids: Vec<Uuid>,
    /// Display colour assigned at group creation — RGB bytes.
    /// Default is a neutral grey `[140, 140, 155]` for groups loaded from old
    /// files that pre-date the colour field.
    #[serde(default = "StreamGroup::default_color")]
    pub color: [u8; 3],
}

impl StreamGroup {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    pub(crate) fn default_color() -> [u8; 3] {
        [140, 140, 155]
    }

    /// Create a new group from an initial (non-empty) set of stream IDs.
    ///
    /// Duplicate IDs in `member_stream_ids` are silently deduplicated.
    /// The display colour defaults to grey; set `group.color` afterwards to
    /// assign the intended colour.
    pub fn new(name: impl Into<String>, member_stream_ids: Vec<Uuid>) -> Self {
        // Deduplicate while preserving order.
        let mut seen = std::collections::HashSet::new();
        let deduped = member_stream_ids
            .into_iter()
            .filter(|id| seen.insert(*id))
            .collect();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            member_stream_ids: deduped,
            color: Self::default_color(),
        }
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Returns `true` if the group has no member streams and should be
    /// dissolved.
    pub fn is_empty(&self) -> bool {
        self.member_stream_ids.is_empty()
    }

    /// Returns `true` if `stream_id` is a member of this group.
    pub fn contains(&self, stream_id: Uuid) -> bool {
        self.member_stream_ids.contains(&stream_id)
    }

    /// The number of member streams.
    pub fn len(&self) -> usize {
        self.member_stream_ids.len()
    }

    // -----------------------------------------------------------------------
    // Mutations
    // -----------------------------------------------------------------------

    /// Add `stream_id` to this group.
    ///
    /// Does nothing if the stream is already a member.  Returns `true` if the
    /// stream was newly added.
    pub fn add_stream(&mut self, stream_id: Uuid) -> bool {
        if self.contains(stream_id) {
            return false;
        }
        self.member_stream_ids.push(stream_id);
        true
    }

    /// Remove `stream_id` from this group.
    ///
    /// Returns `true` if the stream was present and was removed.
    pub fn remove_stream(&mut self, stream_id: Uuid) -> bool {
        let before = self.member_stream_ids.len();
        self.member_stream_ids.retain(|id| *id != stream_id);
        self.member_stream_ids.len() < before
    }
}
