//! Node model (spec §3.1).
//!
//! Every node on a canvas has a [`NodeKind`], a name, input/output type
//! specifications, and optional linkage to a sub-diagram.  Boundary nodes
//! are auto-generated when a sub-diagram is created and are locked against
//! user rename and delete (spec §6.2).
//!
//! # Spec constraints encoded here
//! * A [`NodeKind::Source`] node's `input_spec` is always empty.
//! * A [`NodeKind::Sink`] node's `output_spec` is always empty.
//! * Only a [`NodeKind::Process`] node may hold a `sub_diagram_id`.
//! * A node cannot connect to itself — enforced at the diagram layer.
//!
//! The node does **not** know which streams are attached to it; that
//! information lives in [`crate::model::diagram::Diagram`].

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::types::TypeSpec;

// ---------------------------------------------------------------------------
// NodeKind
// ---------------------------------------------------------------------------

/// The functional role of a node on the canvas.
///
/// | Kind    | Inputs | Outputs | Sub-diagram |
/// |---------|--------|---------|-------------|
/// | Source  | None   | ≥ 1     | No          |
/// | Sink    | ≥ 1    | None    | No          |
/// | Process | ≥ 1    | ≥ 1     | Optional    |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    /// Produces data; has no inputs.
    Source,
    /// Consumes data; has no outputs.
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

    /// Human-readable label used in the UI.
    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Source => "Source",
            NodeKind::Sink => "Sink",
            NodeKind::Process => "Process",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A node on the canvas.
///
/// Nodes are the vertices of the data-flow graph.  [`crate::model::stream::Stream`]s
/// are the directed edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Stable unique identifier.  Never changes for the lifetime of a node.
    pub id: Uuid,
    /// Display name shown on the canvas and in the tree pane.
    ///
    /// User-editable unless [`Node::is_boundary`] is `true`.
    pub name: String,
    /// Functional role of this node on the canvas.
    pub kind: NodeKind,
    /// Types this node accepts on its input side.
    ///
    /// Always empty for [`NodeKind::Source`].
    pub input_spec: TypeSpec,
    /// Types this node produces on its output side.
    ///
    /// Always empty for [`NodeKind::Sink`].
    pub output_spec: TypeSpec,
    /// When `true` this node was auto-generated as a boundary node for a
    /// sub-diagram (spec §6.2).  It cannot be renamed or deleted by the user,
    /// and its specs are controlled exclusively by the parent process node.
    pub is_boundary: bool,
    /// ID of the sub-diagram owned by this node.
    ///
    /// Only meaningful (and only ever `Some`) when `kind == NodeKind::Process`.
    pub sub_diagram_id: Option<Uuid>,
}

impl Node {
    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    /// Create a new, non-boundary node with empty type specs.
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

    /// Create a locked boundary node with explicit specs.
    ///
    /// Boundary nodes are created automatically by the diagram decomposition
    /// logic (spec §6.2); users cannot create them directly.
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

    // -----------------------------------------------------------------------
    // Guard queries
    // -----------------------------------------------------------------------

    /// Returns `true` if the user is allowed to rename this node.
    ///
    /// Boundary nodes are locked — their name is managed by the sub-diagram
    /// machinery and may not be changed by the user.
    pub fn is_renameable(&self) -> bool {
        !self.is_boundary
    }

    /// Returns `true` if the user is allowed to delete this node.
    ///
    /// Boundary nodes cannot be deleted individually; they are removed as a
    /// unit when their parent sub-diagram relationship is severed.
    pub fn is_deletable(&self) -> bool {
        !self.is_boundary
    }

    /// Returns `true` if this process node has an attached sub-diagram.
    pub fn has_subdiagram(&self) -> bool {
        self.sub_diagram_id.is_some()
    }

    // -----------------------------------------------------------------------
    // Spec accessors
    // -----------------------------------------------------------------------

    /// The spec relevant to incoming streams: the `input_spec` for Process/Sink
    /// nodes, and an empty spec for Source nodes (which accept no inputs).
    pub fn effective_input_spec(&self) -> &TypeSpec {
        &self.input_spec
    }

    /// The spec relevant to outgoing streams: the `output_spec` for
    /// Process/Source nodes, and an empty spec for Sink nodes.
    pub fn effective_output_spec(&self) -> &TypeSpec {
        &self.output_spec
    }

    // -----------------------------------------------------------------------
    // Mutations
    // -----------------------------------------------------------------------

    /// Rename this node.
    ///
    /// Returns `false` without mutating if the node is a boundary node.
    pub fn rename(&mut self, new_name: impl Into<String>) -> bool {
        if self.is_boundary {
            return false;
        }
        self.name = new_name.into();
        true
    }

    /// Attach a sub-diagram to this process node.
    ///
    /// Returns `false` without mutating if this node is not a
    /// [`NodeKind::Process`].
    pub fn attach_subdiagram(&mut self, diagram_id: Uuid) -> bool {
        if self.kind != NodeKind::Process {
            return false;
        }
        self.sub_diagram_id = Some(diagram_id);
        true
    }

    /// Detach the sub-diagram from this process node.
    ///
    /// Returns the detached diagram ID, or `None` if there was none.
    pub fn detach_subdiagram(&mut self) -> Option<Uuid> {
        self.sub_diagram_id.take()
    }
}
