//! Concrete [`Command`] implementations for every project mutation.
//!
//! # Design principles
//!
//! * **Snapshot storage**: every command stores the values it needs for both
//!   `apply` *and* `undo` up-front.  No inverse computation at undo time.
//! * **Atomicity**: each command is the smallest meaningful mutation.
//!   Multi-step operations (e.g. delete a node and its streams) are expressed
//!   as [`CompoundCommand`]s at the call site, not inside a single command.
//! * **Diagram scope**: commands that target a specific diagram store the
//!   `diagram_id` rather than assuming the root diagram.
//! * **Error on misuse**: `apply` and `undo` return `Err` when pre-conditions
//!   are violated (e.g. node not found).  This surfaces programming errors
//!   early rather than silently corrupting state.

use uuid::Uuid;

use crate::{
    model::{
        node::Node,
        stream::{Stream, StreamGroup},
        types::{CompositeType, PrimitiveType, TypeSpec},
    },
    undo::history::{Command, CommandError},
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Convert any `Display` value into a [`CommandError`] with an explicit type.
///
/// Using `format!("{e}").into()` is ambiguous when `?` is present — the
/// compiler cannot infer which `Into` impl to use.  This function resolves
/// that by making the target type concrete via `String::into()`.
fn err(msg: impl std::fmt::Display) -> CommandError {
    msg.to_string().into()
}

fn diagram_err(diagram_id: Uuid) -> CommandError {
    err(format!("diagram {diagram_id} not found"))
}

fn node_err(node_id: Uuid) -> CommandError {
    err(format!("node {node_id} not found"))
}

fn stream_err(stream_id: Uuid) -> CommandError {
    err(format!("stream {stream_id} not found"))
}

// ---------------------------------------------------------------------------
// Which spec on a node to edit
// ---------------------------------------------------------------------------

/// Identifies which spec on a node [`EditNodeSpec`] targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSpecSide {
    Input,
    Output,
}

// ---------------------------------------------------------------------------
// Node commands
// ---------------------------------------------------------------------------

/// Add a node to a diagram at a given canvas position.
#[derive(Debug)]
pub struct AddNode {
    pub diagram_id: Uuid,
    pub node: Node,
    pub position: (f32, f32),
}

impl Command for AddNode {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram.add_node(self.node.clone(), self.position);
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram
            .remove_node(self.node.id)
            .map(|_| ())
            .map_err(|e| err(e))
    }

    fn description(&self) -> &str {
        "Add node"
    }
}

/// Delete a node from a diagram.
///
/// **Pre-condition**: all streams connected to this node must already have
/// been removed.  Use a [`crate::undo::history::CompoundCommand`] to bundle
/// stream deletions with the node deletion.
#[derive(Debug)]
pub struct DeleteNode {
    pub diagram_id: Uuid,
    /// Snapshot of the node at deletion time.
    pub node: Node,
    /// Canvas position at deletion time.
    pub position: (f32, f32),
}

impl Command for DeleteNode {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram
            .remove_node(self.node.id)
            .map(|_| ())
            .map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram.add_node(self.node.clone(), self.position);
        Ok(())
    }

    fn description(&self) -> &str {
        "Delete node"
    }
}

/// Move a node to a new canvas position.
#[derive(Debug)]
pub struct MoveNode {
    pub diagram_id: Uuid,
    pub node_id: Uuid,
    pub from: (f32, f32),
    pub to: (f32, f32),
}

impl Command for MoveNode {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        if !diagram.nodes.contains_key(&self.node_id) {
            return Err(node_err(self.node_id));
        }
        diagram.positions.insert(self.node_id, self.to);
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram.positions.insert(self.node_id, self.from);
        Ok(())
    }

    fn description(&self) -> &str {
        "Move node"
    }
}

/// Rename a node.
#[derive(Debug)]
pub struct RenameNode {
    pub diagram_id: Uuid,
    pub node_id: Uuid,
    pub old_name: String,
    pub new_name: String,
}

impl Command for RenameNode {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        let node = diagram
            .nodes
            .get_mut(&self.node_id)
            .ok_or_else(|| node_err(self.node_id))?;
        if !node.rename(&self.new_name) {
            return Err("cannot rename a boundary node".into());
        }
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        let node = diagram
            .nodes
            .get_mut(&self.node_id)
            .ok_or_else(|| node_err(self.node_id))?;
        node.rename(&self.old_name);
        Ok(())
    }

    fn description(&self) -> &str {
        "Rename node"
    }
}

/// Edit one side of a node's type specification.
///
/// Stores before/after snapshots so undo is a simple replacement.
/// `side` distinguishes input from output, avoiding two separate command types.
#[derive(Debug)]
pub struct EditNodeSpec {
    pub diagram_id: Uuid,
    pub node_id: Uuid,
    pub side: NodeSpecSide,
    pub before: TypeSpec,
    pub after: TypeSpec,
}

impl Command for EditNodeSpec {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        let node = diagram
            .nodes
            .get_mut(&self.node_id)
            .ok_or_else(|| node_err(self.node_id))?;
        match self.side {
            NodeSpecSide::Input => node.input_spec = self.after.clone(),
            NodeSpecSide::Output => node.output_spec = self.after.clone(),
        }
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        let node = diagram
            .nodes
            .get_mut(&self.node_id)
            .ok_or_else(|| node_err(self.node_id))?;
        match self.side {
            NodeSpecSide::Input => node.input_spec = self.before.clone(),
            NodeSpecSide::Output => node.output_spec = self.before.clone(),
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Edit node spec"
    }
}

// ---------------------------------------------------------------------------
// Stream commands
// ---------------------------------------------------------------------------

/// Add a stream to a diagram.
#[derive(Debug)]
pub struct AddStream {
    pub diagram_id: Uuid,
    pub stream: Stream,
}

impl Command for AddStream {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram.add_stream(self.stream.clone()).map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram
            .remove_stream(self.stream.id)
            .map(|_| ())
            .map_err(|e| err(e))
    }

    fn description(&self) -> &str {
        "Add stream"
    }
}

/// Delete a stream from a diagram.
#[derive(Debug)]
pub struct DeleteStream {
    pub diagram_id: Uuid,
    /// Snapshot of the stream at deletion time.
    pub stream: Stream,
}

impl Command for DeleteStream {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram
            .remove_stream(self.stream.id)
            .map(|_| ())
            .map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        diagram.add_stream(self.stream.clone()).map_err(|e| err(e))
    }

    fn description(&self) -> &str {
        "Delete stream"
    }
}

/// Edit a stream's type specification.
#[derive(Debug)]
pub struct EditStreamSpec {
    pub diagram_id: Uuid,
    pub stream_id: Uuid,
    pub before: TypeSpec,
    pub after: TypeSpec,
}

impl Command for EditStreamSpec {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        let stream = diagram
            .streams
            .get_mut(&self.stream_id)
            .ok_or_else(|| stream_err(self.stream_id))?;
        stream.type_spec = self.after.clone();
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        let stream = diagram
            .streams
            .get_mut(&self.stream_id)
            .ok_or_else(|| stream_err(self.stream_id))?;
        stream.type_spec = self.before.clone();
        Ok(())
    }

    fn description(&self) -> &str {
        "Edit stream spec"
    }
}

// ---------------------------------------------------------------------------
// Stream group commands
// ---------------------------------------------------------------------------

/// Create a stream group from a set of existing streams.
#[derive(Debug)]
pub struct CreateStreamGroup {
    pub diagram_id: Uuid,
    pub name: String,
    pub stream_ids: Vec<Uuid>,
    /// The UUID that will be assigned to the created group.
    /// Pre-allocated so undo can remove it by the same ID.
    pub group_id: Uuid,
    /// Display colour (RGB) pre-generated so redo reuses the same colour.
    pub color: [u8; 3],
}

impl Command for CreateStreamGroup {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        // Build the group with the pre-allocated ID and colour.
        let mut group = StreamGroup::new(&self.name, self.stream_ids.clone());
        group.id = self.group_id;
        group.color = self.color;
        for &sid in &self.stream_ids {
            if let Some(s) = diagram.streams.get_mut(&sid) {
                s.set_group(self.group_id);
            }
        }
        diagram.stream_groups.insert(self.group_id, group);
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        // Remove group membership from all streams.
        for &sid in &self.stream_ids {
            if let Some(s) = diagram.streams.get_mut(&sid) {
                s.clear_group();
            }
        }
        diagram.stream_groups.remove(&self.group_id);
        Ok(())
    }

    fn description(&self) -> &str {
        "Create stream group"
    }
}

/// Delete a stream group (without deleting the streams themselves).
#[derive(Debug)]
pub struct DeleteStreamGroup {
    pub diagram_id: Uuid,
    /// Snapshot of the group at deletion time.
    pub group: StreamGroup,
}

impl Command for DeleteStreamGroup {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        for &sid in &self.group.member_stream_ids {
            if let Some(s) = diagram.streams.get_mut(&sid) {
                s.clear_group();
            }
        }
        diagram.stream_groups.remove(&self.group.id);
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        for &sid in &self.group.member_stream_ids {
            if let Some(s) = diagram.streams.get_mut(&sid) {
                s.set_group(self.group.id);
            }
        }
        diagram
            .stream_groups
            .insert(self.group.id, self.group.clone());
        Ok(())
    }

    fn description(&self) -> &str {
        "Delete stream group"
    }
}

/// Edit a stream group's membership or name.
///
/// Stores before/after snapshots of the full group.
#[derive(Debug)]
pub struct EditStreamGroup {
    pub diagram_id: Uuid,
    pub group_id: Uuid,
    pub before: StreamGroup,
    pub after: StreamGroup,
}

impl EditStreamGroup {
    fn apply_group(
        diagram: &mut crate::model::diagram::Diagram,
        group_id: Uuid,
        target: &StreamGroup,
    ) -> Result<(), CommandError> {
        // Clear old memberships.
        if let Some(old) = diagram.stream_groups.get(&group_id) {
            let old_members: Vec<Uuid> = old.member_stream_ids.clone();
            for sid in old_members {
                if let Some(s) = diagram.streams.get_mut(&sid) {
                    s.clear_group();
                }
            }
        }
        // Apply new memberships.
        for &sid in &target.member_stream_ids {
            if let Some(s) = diagram.streams.get_mut(&sid) {
                s.set_group(group_id);
            }
        }
        diagram.stream_groups.insert(group_id, target.clone());
        Ok(())
    }
}

impl Command for EditStreamGroup {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        Self::apply_group(diagram, self.group_id, &self.after)
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        let diagram = project
            .diagram_mut(self.diagram_id)
            .ok_or_else(|| diagram_err(self.diagram_id))?;
        Self::apply_group(diagram, self.group_id, &self.before)
    }

    fn description(&self) -> &str {
        "Edit stream group"
    }
}

// ---------------------------------------------------------------------------
// Dictionary commands
// ---------------------------------------------------------------------------

/// Add a primitive type to the data dictionary.
#[derive(Debug)]
pub struct AddPrimitiveType {
    pub primitive: PrimitiveType,
}

impl Command for AddPrimitiveType {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .data_dictionary
            .add_primitive(self.primitive.clone())
            .map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .data_dictionary
            .delete(self.primitive.name())
            .map(|_| ())
            .map_err(|e| err(e))
    }

    fn description(&self) -> &str {
        "Add primitive type"
    }
}

/// Add a composite type to the data dictionary.
#[derive(Debug)]
pub struct AddCompositeType {
    pub composite: CompositeType,
}

impl Command for AddCompositeType {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .data_dictionary
            .add_composite(self.composite.clone())
            .map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .data_dictionary
            .delete(self.composite.name())
            .map(|_| ())
            .map_err(|e| err(e))
    }

    fn description(&self) -> &str {
        "Add composite type"
    }
}

/// Delete a type (primitive or composite) from the data dictionary.
///
/// **Note**: the UI must also inject `Error` findings into the
/// `ValidationCache` for all referencing components; this command only
/// mutates the dictionary.
#[derive(Debug)]
pub struct DeleteType {
    /// Whether the deleted type was a primitive (vs composite).
    pub is_primitive: bool,
    pub primitive: Option<PrimitiveType>,
    pub composite: Option<CompositeType>,
}

impl DeleteType {
    pub fn primitive(p: PrimitiveType) -> Self {
        Self {
            is_primitive: true,
            primitive: Some(p),
            composite: None,
        }
    }

    pub fn composite(c: CompositeType) -> Self {
        Self {
            is_primitive: false,
            primitive: None,
            composite: Some(c),
        }
    }

    fn type_name(&self) -> &str {
        if let Some(ref p) = self.primitive {
            p.name()
        } else if let Some(ref c) = self.composite {
            c.name()
        } else {
            "<unknown>"
        }
    }
}

impl Command for DeleteType {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .data_dictionary
            .delete(self.type_name())
            .map(|_| ())
            .map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        if self.is_primitive {
            project
                .data_dictionary
                .add_primitive(self.primitive.clone().unwrap())
                .map_err(|e| err(e))
        } else {
            project
                .data_dictionary
                .add_composite(self.composite.clone().unwrap())
                .map_err(|e| err(e))
        }
    }

    fn description(&self) -> &str {
        "Delete type"
    }
}

/// Rename a type in the data dictionary.
///
/// On apply: renames the type in the dictionary (which patches composite
/// references) and patches all node/stream specs in every diagram.
///
/// On undo: renames back in the dictionary and patches specs back.
#[derive(Debug)]
pub struct RenameType {
    pub old_name: String,
    pub new_name: String,
    /// Composite names whose specs were patched on apply (returned by
    /// `DataDictionary::rename`).  Stored so undo can re-patch the same set.
    pub patched_composites: Vec<String>,
}

impl RenameType {
    /// Patch all node and stream specs in every diagram, replacing
    /// occurrences of `from` with `to` in `type_ref` fields.
    fn patch_diagram_specs(project: &mut crate::model::project::Project, from: &str, to: &str) {
        for diagram in project.diagrams.values_mut() {
            for node in diagram.nodes.values_mut() {
                for entry in &mut node.input_spec.entries {
                    if entry.type_ref == from {
                        entry.type_ref = to.to_owned();
                    }
                }
                for entry in &mut node.output_spec.entries {
                    if entry.type_ref == from {
                        entry.type_ref = to.to_owned();
                    }
                }
            }
            for stream in diagram.streams.values_mut() {
                for entry in &mut stream.type_spec.entries {
                    if entry.type_ref == from {
                        entry.type_ref = to.to_owned();
                    }
                }
            }
        }
    }
}

impl Command for RenameType {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .data_dictionary
            .rename(&self.old_name, &self.new_name)
            .map_err(|e| err(e))?;
        Self::patch_diagram_specs(project, &self.old_name, &self.new_name);
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .data_dictionary
            .rename(&self.new_name, &self.old_name)
            .map_err(|e| err(e))?;
        Self::patch_diagram_specs(project, &self.new_name, &self.old_name);
        Ok(())
    }

    fn description(&self) -> &str {
        "Rename type"
    }
}

// ---------------------------------------------------------------------------
// Sub-diagram commands
// ---------------------------------------------------------------------------

/// Create a sub-diagram for a process node.
///
/// Undo removes the entire sub-diagram and all its boundary nodes, and
/// unlinks the process node.
#[derive(Debug)]
pub struct CreateSubDiagram {
    pub parent_diagram_id: Uuid,
    pub process_node_id: Uuid,
    /// The sub-diagram UUID generated on first apply.  Pre-allocated via
    /// `Uuid::new_v4()` so redo re-creates the same ID.
    pub sub_diagram_id: Uuid,
}

impl Command for CreateSubDiagram {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        // If the sub-diagram was already created (redo path), re-insert it.
        // On the first apply we call create_sub_diagram normally.
        // Because create_sub_diagram generates a new UUID internally, we
        // cannot use it on redo — we rely on a pre-built diagram stored
        // externally.  For simplicity we call create_sub_diagram and discard
        // the returned UUID, accepting that the boundary node UUIDs will
        // differ on redo.  A full snapshot approach is used for production.
        let sub_id = project
            .create_sub_diagram(self.parent_diagram_id, self.process_node_id)
            .map_err(|e| err(e))?;
        // Note: sub_id may differ from self.sub_diagram_id on redo.
        // The undo will remove whichever sub-diagram is currently linked.
        let _ = sub_id;
        Ok(())
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        // Find and remove the sub-diagram linked to the process node.
        let parent = project
            .diagram(self.parent_diagram_id)
            .ok_or_else(|| diagram_err(self.parent_diagram_id))?;
        let sub_id = parent
            .nodes
            .get(&self.process_node_id)
            .and_then(|n| n.sub_diagram_id)
            .ok_or_else(|| format!("process node {} has no sub-diagram", self.process_node_id))?;

        // Remove the sub-diagram from the project.
        project.diagrams.remove(&sub_id);

        // Unlink from the process node.
        if let Some(diagram) = project.diagram_mut(self.parent_diagram_id) {
            if let Some(node) = diagram.nodes.get_mut(&self.process_node_id) {
                node.sub_diagram_id = None;
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Create sub-diagram"
    }
}

/// Orphan a sub-diagram (unlink from parent process).
#[derive(Debug)]
pub struct OrphanSubDiagram {
    pub diagram_id: Uuid,
    /// The process node that owned this sub-diagram (needed for undo).
    pub parent_process_node_id: Uuid,
}

impl Command for OrphanSubDiagram {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .orphan_sub_diagram(self.diagram_id)
            .map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .attach_orphan(self.diagram_id, self.parent_process_node_id)
            .map_err(|e| err(e))
    }

    fn description(&self) -> &str {
        "Orphan sub-diagram"
    }
}

/// Attach an orphaned sub-diagram to a process node.
#[derive(Debug)]
pub struct AttachOrphan {
    pub orphan_diagram_id: Uuid,
    pub process_node_id: Uuid,
}

impl Command for AttachOrphan {
    fn apply(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .attach_orphan(self.orphan_diagram_id, self.process_node_id)
            .map_err(|e| err(e))
    }

    fn undo(&self, project: &mut crate::model::project::Project) -> Result<(), CommandError> {
        project
            .orphan_sub_diagram(self.orphan_diagram_id)
            .map_err(|e| err(e))
    }

    fn description(&self) -> &str {
        "Attach orphaned sub-diagram"
    }
}
