//! Project model — the root of the entire document (spec §7).
//!
//! A [`Project`] owns the [`DataDictionary`] and the full tree of
//! [`Diagram`]s.  It is the unit of file persistence: one project = one XML
//! file.
//!
//! # Sub-diagram lifecycle (spec §6)
//!
//! ```text
//! create_sub_diagram(process_id)
//!   └─ generates locked boundary nodes from process I/O
//!   └─ sets process.sub_diagram_id
//!
//! orphan_sub_diagram(diagram_id)          ← called when parent process deleted
//!   └─ clears process.sub_diagram_id
//!   └─ moves diagram_id → orphaned_diagram_ids
//!   └─ boundary nodes kept but shown in error state
//!
//! attach_orphan(orphan_id, process_id)    ← "Set sub-diagram" right-click
//!   └─ removes from orphaned_diagram_ids
//!   └─ sets process.sub_diagram_id = orphan_id
//!   └─ boundary nodes kept; mismatches shown in error state
//!
//! update_boundary_nodes(diagram_id)       ← called after process I/O edit
//!   └─ deletes old boundary nodes (dangling their streams)
//!   └─ regenerates boundary nodes from current process I/O
//!   └─ returns IDs of newly-dangled streams
//! ```
//!
//! # Responsibility split
//! `Project` manages cross-diagram relationships (sub-diagram linkage,
//! orphaned list).  Each `Diagram` manages its own nodes and streams.
//! The undo stack wraps every mutating method in a `Command` so all changes
//! are reversible.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{
    diagram::Diagram,
    dictionary::DataDictionary,
    node::{Node, NodeKind},
    types::TypeSpec,
};

/// Schema version written into saved files.  Increment on breaking changes to
/// the XML structure.  Lives here so that `Project::new()` can stamp it and
/// `persistence::xml` can read it back.
pub const SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// ProjectError
// ---------------------------------------------------------------------------

/// Errors that can arise from project-level operations.
#[derive(Debug, PartialEq, Eq)]
pub enum ProjectError {
    /// The referenced diagram does not exist in the project.
    DiagramNotFound(Uuid),
    /// The referenced node does not exist in the specified diagram.
    NodeNotFound(Uuid),
    /// The operation requires a Process node but the node has a different kind.
    NotAProcessNode(Uuid),
    /// The node already has a sub-diagram attached.
    AlreadyHasSubDiagram(Uuid),
    /// The diagram is not in the orphaned list.
    NotOrphaned(Uuid),
    /// The diagram is the root diagram and cannot be orphaned or deleted.
    IsRootDiagram(Uuid),
    /// `update_boundary_nodes` was called on a diagram with no parent process
    /// (e.g. the root diagram or an orphaned sub-diagram).
    NoParentProcess(Uuid),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::DiagramNotFound(id) => write!(f, "diagram {id} not found"),
            ProjectError::NodeNotFound(id) => write!(f, "node {id} not found"),
            ProjectError::NotAProcessNode(id) => write!(f, "node {id} is not a Process node"),
            ProjectError::AlreadyHasSubDiagram(id) => {
                write!(f, "process {id} already has a sub-diagram")
            }
            ProjectError::NotOrphaned(id) => write!(f, "diagram {id} is not orphaned"),
            ProjectError::IsRootDiagram(id) => write!(f, "diagram {id} is the root diagram"),
            ProjectError::NoParentProcess(id) => write!(f, "diagram {id} has no parent process"),
        }
    }
}

// ---------------------------------------------------------------------------
// BoundaryMismatch
// ---------------------------------------------------------------------------

/// Describes a boundary node whose spec does not match the parent process
/// after an [`Project::attach_orphan`] call.
///
/// The UI uses this to overlay error badges on mismatched boundary nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryMismatch {
    /// The boundary node ID whose spec differs from the process.
    pub node_id: Uuid,
    /// Short human-readable reason.
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

/// The root document object.
///
/// All data in the project flows from here: the data dictionary, every
/// diagram, and the bookkeeping needed to track the sub-diagram tree and the
/// orphaned directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Schema version used when this project was last saved.
    /// Always written as [`SCHEMA_VERSION`] on new projects.
    pub schema_version: u32,
    /// Shared type dictionary for all diagrams.
    pub data_dictionary: DataDictionary,
    /// ID of the top-level diagram (always present; never orphaned).
    pub root_diagram_id: Uuid,
    /// All diagrams in this project — root, sub-diagrams, and orphaned —
    /// keyed by their ID.
    pub diagrams: HashMap<Uuid, Diagram>,
    /// IDs of sub-diagrams whose parent process node has been deleted
    /// (spec §6.4).  Displayed under the "Orphaned" folder in the tree pane.
    pub orphaned_diagram_ids: Vec<Uuid>,
}

impl Project {
    // -----------------------------------------------------------------------
    // Constructor
    // -----------------------------------------------------------------------

    /// Create a new project with an empty data dictionary and a blank root
    /// diagram.
    pub fn new() -> Self {
        let root = Diagram::new("Root");
        let root_id = root.id;
        let mut diagrams = HashMap::new();
        diagrams.insert(root_id, root);
        Self {
            schema_version: SCHEMA_VERSION,
            data_dictionary: DataDictionary::new(),
            root_diagram_id: root_id,
            diagrams,
            orphaned_diagram_ids: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Basic lookups
    // -----------------------------------------------------------------------

    /// Look up a diagram by ID.
    pub fn diagram(&self, id: Uuid) -> Option<&Diagram> {
        self.diagrams.get(&id)
    }

    /// Look up a diagram mutably by ID.
    pub fn diagram_mut(&mut self, id: Uuid) -> Option<&mut Diagram> {
        self.diagrams.get_mut(&id)
    }

    /// Returns `true` if `diagram_id` is in the orphaned list.
    pub fn is_orphaned(&self, diagram_id: Uuid) -> bool {
        self.orphaned_diagram_ids.contains(&diagram_id)
    }

    // -----------------------------------------------------------------------
    // Sub-diagram creation (spec §6.1–6.2)
    // -----------------------------------------------------------------------

    /// Create a sub-diagram for the process node `process_node_id` in
    /// `parent_diagram_id`.
    ///
    /// Generates locked boundary nodes:
    /// * One [`NodeKind::Source`] boundary node per entry in the process
    ///   `input_spec` — data entering the process appears as a source inside.
    /// * One [`NodeKind::Sink`] boundary node per entry in the process
    ///   `output_spec` — data leaving the process appears as a sink inside.
    ///
    /// Links `process_node.sub_diagram_id` to the new diagram.
    ///
    /// Returns the ID of the newly created sub-diagram.
    ///
    /// # Errors
    /// * [`ProjectError::DiagramNotFound`] — `parent_diagram_id` unknown.
    /// * [`ProjectError::NodeNotFound`] — `process_node_id` unknown.
    /// * [`ProjectError::NotAProcessNode`] — node is not a Process.
    /// * [`ProjectError::AlreadyHasSubDiagram`] — process already linked.
    pub fn create_sub_diagram(
        &mut self,
        parent_diagram_id: Uuid,
        process_node_id: Uuid,
    ) -> Result<Uuid, ProjectError> {
        // ---- Validate -------------------------------------------------------
        let parent = self
            .diagrams
            .get(&parent_diagram_id)
            .ok_or(ProjectError::DiagramNotFound(parent_diagram_id))?;

        let process = parent
            .nodes
            .get(&process_node_id)
            .ok_or(ProjectError::NodeNotFound(process_node_id))?;

        if process.kind != NodeKind::Process {
            return Err(ProjectError::NotAProcessNode(process_node_id));
        }
        if process.sub_diagram_id.is_some() {
            return Err(ProjectError::AlreadyHasSubDiagram(process_node_id));
        }

        // ---- Snapshot process I/O before any mutation -----------------------
        // Clone the specs so we can release the immutable borrow on
        // `self.diagrams` before mutating it.
        let process_name = parent.nodes[&process_node_id].name.clone();
        let input_entries = parent.nodes[&process_node_id].input_spec.entries.clone();
        let output_entries = parent.nodes[&process_node_id].output_spec.entries.clone();

        // ---- Build sub-diagram ----------------------------------------------
        let mut sub = Diagram::new(format!("{process_name} (sub)"));
        sub.parent_process_id = Some(process_node_id);
        let sub_id = sub.id;

        // Boundary sources — one per input entry (data flows *in*).
        for (i, entry) in input_entries.iter().enumerate() {
            let mut spec = TypeSpec::new();
            spec.add(entry.count, &entry.type_ref);
            let bnode = Node::new_boundary(
                NodeKind::Source,
                format!("in_{i}"),
                TypeSpec::new(), // sources have no inputs
                spec,
            );
            // Spread boundary nodes along the left edge of the canvas.
            let y = 80.0 + (i as f32) * 120.0;
            sub.add_node(bnode, (40.0, y));
        }

        // Boundary sinks — one per output entry (data flows *out*).
        for (i, entry) in output_entries.iter().enumerate() {
            let mut spec = TypeSpec::new();
            spec.add(entry.count, &entry.type_ref);
            let bnode = Node::new_boundary(
                NodeKind::Sink,
                format!("out_{i}"),
                spec,
                TypeSpec::new(), // sinks have no outputs
            );
            // Spread boundary nodes along the right edge.
            let y = 80.0 + (i as f32) * 120.0;
            sub.add_node(bnode, (800.0, y));
        }

        // ---- Insert sub-diagram and link process node -----------------------
        self.diagrams.insert(sub_id, sub);

        self.diagrams
            .get_mut(&parent_diagram_id)
            .unwrap()
            .nodes
            .get_mut(&process_node_id)
            .unwrap()
            .sub_diagram_id = Some(sub_id);

        Ok(sub_id)
    }

    // -----------------------------------------------------------------------
    // Orphaning (spec §6.4)
    // -----------------------------------------------------------------------

    /// Move `diagram_id` to the orphaned list.
    ///
    /// Called when the parent process node is deleted.  The diagram's
    /// `parent_process_id` is cleared.  Boundary nodes are preserved but the
    /// UI shows them in error state.
    ///
    /// Also clears `sub_diagram_id` on the process node if it still exists
    /// in any diagram (it may have already been removed — the caller handles
    /// the node deletion separately).
    ///
    /// # Errors
    /// * [`ProjectError::DiagramNotFound`] — `diagram_id` unknown.
    /// * [`ProjectError::IsRootDiagram`] — the root diagram cannot be orphaned.
    pub fn orphan_sub_diagram(&mut self, diagram_id: Uuid) -> Result<(), ProjectError> {
        if diagram_id == self.root_diagram_id {
            return Err(ProjectError::IsRootDiagram(diagram_id));
        }
        if !self.diagrams.contains_key(&diagram_id) {
            return Err(ProjectError::DiagramNotFound(diagram_id));
        }

        // Clear the back-pointer on the diagram itself.
        if let Some(sub) = self.diagrams.get_mut(&diagram_id) {
            sub.parent_process_id = None;
        }

        // If the parent process node still exists in some diagram, clear its
        // forward pointer too.  (This is a best-effort scan; the node may
        // already have been removed by the caller.)
        for diagram in self.diagrams.values_mut() {
            for node in diagram.nodes.values_mut() {
                if node.sub_diagram_id == Some(diagram_id) {
                    node.sub_diagram_id = None;
                }
            }
        }

        // Add to orphaned list if not already there.
        if !self.orphaned_diagram_ids.contains(&diagram_id) {
            self.orphaned_diagram_ids.push(diagram_id);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Re-attachment (spec §6.5)
    // -----------------------------------------------------------------------

    /// Attach orphaned sub-diagram `orphan_id` to process node
    /// `process_node_id` (which must live in some diagram in the project).
    ///
    /// After attachment:
    /// * `orphan_id` is removed from `orphaned_diagram_ids`.
    /// * The diagram's `parent_process_id` is set to `process_node_id`.
    /// * The process node's `sub_diagram_id` is set to `orphan_id`.
    ///
    /// Boundary nodes are **not** rebuilt here.  The caller should then call
    /// [`Project::update_boundary_nodes`] and use the returned mismatch
    /// information to overlay error badges.
    ///
    /// # Errors
    /// * [`ProjectError::DiagramNotFound`] — `orphan_id` unknown.
    /// * [`ProjectError::NotOrphaned`] — diagram is not in the orphaned list.
    /// * [`ProjectError::NodeNotFound`] — `process_node_id` not found in any diagram.
    /// * [`ProjectError::NotAProcessNode`] — target node is not a Process.
    /// * [`ProjectError::AlreadyHasSubDiagram`] — target process already linked.
    pub fn attach_orphan(
        &mut self,
        orphan_id: Uuid,
        process_node_id: Uuid,
    ) -> Result<(), ProjectError> {
        // ---- Validate orphan ------------------------------------------------
        if !self.diagrams.contains_key(&orphan_id) {
            return Err(ProjectError::DiagramNotFound(orphan_id));
        }
        if !self.is_orphaned(orphan_id) {
            return Err(ProjectError::NotOrphaned(orphan_id));
        }

        // ---- Find the process node ------------------------------------------
        // Scan all diagrams to locate the process node.
        let parent_diagram_id = self
            .diagrams
            .iter()
            .find(|(_, d)| d.nodes.contains_key(&process_node_id))
            .map(|(id, _)| *id)
            .ok_or(ProjectError::NodeNotFound(process_node_id))?;

        let process = &self.diagrams[&parent_diagram_id].nodes[&process_node_id];
        if process.kind != NodeKind::Process {
            return Err(ProjectError::NotAProcessNode(process_node_id));
        }
        if process.sub_diagram_id.is_some() {
            return Err(ProjectError::AlreadyHasSubDiagram(process_node_id));
        }

        // ---- Mutate ---------------------------------------------------------
        // Remove from orphaned list.
        self.orphaned_diagram_ids.retain(|&id| id != orphan_id);

        // Set the diagram's parent pointer.
        self.diagrams.get_mut(&orphan_id).unwrap().parent_process_id = Some(process_node_id);

        // Set the process node's forward pointer.
        self.diagrams
            .get_mut(&parent_diagram_id)
            .unwrap()
            .nodes
            .get_mut(&process_node_id)
            .unwrap()
            .sub_diagram_id = Some(orphan_id);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Boundary node update (spec §6.3)
    // -----------------------------------------------------------------------

    /// Rebuild the locked boundary nodes in `diagram_id` to match the current
    /// I/O spec of the diagram's parent process node.
    ///
    /// This is the **top-down propagation** described in spec §6.3.  It is an
    /// atomic operation: old boundary nodes are removed first (dangling any
    /// streams that were connected to them), then new ones are generated from
    /// the current process spec.
    ///
    /// Returns the IDs of all streams that became dangling as a result of
    /// removing the old boundary nodes.  The caller is responsible for
    /// triggering incremental validation on those streams.
    ///
    /// # Errors
    /// * [`ProjectError::DiagramNotFound`] — `diagram_id` unknown.
    /// * [`ProjectError::NodeNotFound`] — the parent process node cannot be
    ///   found (e.g. the diagram is orphaned).
    pub fn update_boundary_nodes(&mut self, diagram_id: Uuid) -> Result<Vec<Uuid>, ProjectError> {
        // ---- Find parent process and snapshot its I/O -----------------------
        let sub = self
            .diagrams
            .get(&diagram_id)
            .ok_or(ProjectError::DiagramNotFound(diagram_id))?;

        let process_node_id = sub
            .parent_process_id
            .ok_or(ProjectError::NoParentProcess(diagram_id))?;

        // Locate the parent diagram that owns the process node.
        let parent_diagram_id = self
            .diagrams
            .iter()
            .find(|(_, d)| d.nodes.contains_key(&process_node_id))
            .map(|(id, _)| *id)
            .ok_or(ProjectError::NodeNotFound(process_node_id))?;

        // Snapshot I/O specs before releasing the borrow.
        let input_entries = self.diagrams[&parent_diagram_id].nodes[&process_node_id]
            .input_spec
            .entries
            .clone();
        let output_entries = self.diagrams[&parent_diagram_id].nodes[&process_node_id]
            .output_spec
            .entries
            .clone();

        // ---- Remove old boundary nodes --------------------------------------
        let boundary_ids: Vec<Uuid> = self.diagrams[&diagram_id]
            .nodes
            .values()
            .filter(|n| n.is_boundary)
            .map(|n| n.id)
            .collect();

        let sub = self.diagrams.get_mut(&diagram_id).unwrap();
        let mut dangled_stream_ids: Vec<Uuid> = Vec::new();
        for bid in boundary_ids {
            // remove_node marks streams as dangling and returns their IDs.
            if let Ok(removed) = sub.remove_node(bid) {
                dangled_stream_ids.extend(removed.dangled_stream_ids);
            }
        }

        // ---- Generate new boundary nodes ------------------------------------
        let sub = self.diagrams.get_mut(&diagram_id).unwrap();

        for (i, entry) in input_entries.iter().enumerate() {
            let mut spec = TypeSpec::new();
            spec.add(entry.count, &entry.type_ref);
            let bnode =
                Node::new_boundary(NodeKind::Source, format!("in_{i}"), TypeSpec::new(), spec);
            let y = 80.0 + (i as f32) * 120.0;
            sub.add_node(bnode, (40.0, y));
        }

        for (i, entry) in output_entries.iter().enumerate() {
            let mut spec = TypeSpec::new();
            spec.add(entry.count, &entry.type_ref);
            let bnode =
                Node::new_boundary(NodeKind::Sink, format!("out_{i}"), spec, TypeSpec::new());
            let y = 80.0 + (i as f32) * 120.0;
            sub.add_node(bnode, (800.0, y));
        }

        Ok(dangled_stream_ids)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}
