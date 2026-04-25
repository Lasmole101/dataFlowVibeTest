//! Node model (spec §3.1).
//!
//! Every node on a canvas has a [`NodeKind`], a name, input/output type
//! specifications, and optional linkage to a sub-diagram.  Boundary nodes
//! (auto-generated when a sub-diagram is created) are locked and cannot be
//! renamed or deleted by the user.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::types::TypeSpec;

// ---------------------------------------------------------------------------
// NodeKind
// ---------------------------------------------------------------------------

/// The functional role of a node on the canvas.
///
/// | Kind    | Inputs | Outputs |
/// |---------|--------|---------|
/// | Source  | None   | ≥ 1     |
/// | Sink    | ≥ 1    | None    |
/// | Process | ≥ 1    | ≥ 1     |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Produces data; no inputs.
    Source,
    /// Consumes data; no outputs.
    Sink,
    /// Transforms data; has both inputs and outputs.  May own a sub-diagram.
    Process,
}

impl NodeKind {
    /// Returns `true` if this kind may have input streams connected to it.
    pub fn accepts_inputs(self) -> bool {
        matches!(self, NodeKind::Sink | NodeKind::Process)
    }

    /// Returns `true` if this kind may have output streams leaving it.
    pub fn produces_outputs(self) -> bool {
        matches!(self, NodeKind::Source | NodeKind::Process)
    }

    /// Returns `true` if this kind may own a sub-diagram.
    pub fn can_have_subdiagram(self) -> bool {
        matches!(self, NodeKind::Process)
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A node on the canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Stable unique identifier.
    pub id: Uuid,
    /// Display name (user-editable unless `is_boundary` is `true`).
    pub name: String,
    /// Functional role of this node.
    pub kind: NodeKind,
    /// Types this node accepts on its input ports.  Empty for [`NodeKind::Source`].
    pub input_spec: TypeSpec,
    /// Types this node produces on its output ports.  Empty for [`NodeKind::Sink`].
    pub output_spec: TypeSpec,
    /// When `true`, this node was auto-generated as a boundary node for a
    /// sub-diagram and cannot be renamed or deleted by the user.
    pub is_boundary: bool,
    /// ID of the sub-diagram owned by this node, if any.
    /// Only meaningful when `kind == NodeKind::Process`.
    pub sub_diagram_id: Option<Uuid>,
}

impl Node {
    /// Create a new non-boundary node with empty type specs.
    pub fn new(kind: NodeKind, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            input_spec: TypeSpec::new(),
            output_spec: TypeSpec::new(),
            is_boundary: false,
            sub_diagram_id: None,
        }
    }

    /// Create a locked boundary node carrying the given specs.
    pub fn new_boundary(
        kind: NodeKind,
        name: impl Into<String>,
        input_spec: TypeSpec,
        output_spec: TypeSpec,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            input_spec,
            output_spec,
            is_boundary: true,
            sub_diagram_id: None,
        }
    }
}
