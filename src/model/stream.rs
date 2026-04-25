//! Stream and stream-group model (spec §3.2–3.3).
//!
//! A [`Stream`] is a directed, point-to-point connection between two nodes.
//! Streams carry a [`TypeSpec`] that may be edited independently of the nodes
//! at either end.
//!
//! [`StreamGroup`] is a display-only construct: grouped streams are rendered
//! as parallel lines but have no effect on validation.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::types::TypeSpec;

// ---------------------------------------------------------------------------
// Stream
// ---------------------------------------------------------------------------

/// A directed connection between two nodes on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stream {
    /// Stable unique identifier.
    pub id: Uuid,
    /// Node from which data flows.
    pub source_node_id: Uuid,
    /// Node to which data flows.
    pub target_node_id: Uuid,
    /// The type specification carried by this stream.
    pub type_spec: TypeSpec,
    /// The display group this stream belongs to, if any.
    pub group_id: Option<Uuid>,
    /// `true` when either the source or target node has been deleted, leaving
    /// this stream without a valid endpoint.
    pub is_dangling: bool,
}

impl Stream {
    /// Create a new stream between two nodes with an empty type spec.
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
}

// ---------------------------------------------------------------------------
// StreamGroup
// ---------------------------------------------------------------------------

/// A named display group containing one or more streams.
///
/// Groups dissolve automatically when their last member stream is deleted.
/// Grouping has no effect on validation (spec §3.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamGroup {
    /// Stable unique identifier.
    pub id: Uuid,
    /// User-assigned display name.
    pub name: String,
    /// IDs of streams belonging to this group.
    pub member_stream_ids: Vec<Uuid>,
}

impl StreamGroup {
    /// Create a new group with the given name and initial members.
    pub fn new(name: impl Into<String>, member_stream_ids: Vec<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            member_stream_ids,
        }
    }

    /// Returns `true` if this group has no members and should be dissolved.
    pub fn is_empty(&self) -> bool {
        self.member_stream_ids.is_empty()
    }

    /// Remove a stream from this group.  Returns whether the stream was
    /// present.
    pub fn remove_stream(&mut self, stream_id: Uuid) -> bool {
        let before = self.member_stream_ids.len();
        self.member_stream_ids.retain(|id| *id != stream_id);
        self.member_stream_ids.len() < before
    }
}
