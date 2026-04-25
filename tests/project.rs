//! Tests for Project sub-diagram lifecycle (spec §6).
//!
//! Covers: create_sub_diagram, orphan_sub_diagram, attach_orphan,
//! update_boundary_nodes, and all associated error paths.

use dataFlowClaude::model::{
    dictionary::DataDictionary,
    node::{Node, NodeKind},
    project::{Project, ProjectError, SCHEMA_VERSION},
    types::{PrimitiveType, TypeSpec},
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a project pre-loaded with `uint32_t` and `char` primitives.
fn project_with_types() -> Project {
    let mut p = Project::new();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("uint32_t"))
        .unwrap();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("char"))
        .unwrap();
    p
}

/// Add a node to the root diagram and return its ID.
fn add_to_root(project: &mut Project, node: Node) -> Uuid {
    let id = node.id;
    let root_id = project.root_diagram_id;
    project
        .diagrams
        .get_mut(&root_id)
        .unwrap()
        .add_node(node, (0.0, 0.0));
    id
}

/// Build a Process node whose input spec has `(2, uint32_t)` and
/// output spec has `(1, char)`.
fn process_node(name: &str) -> Node {
    let mut n = Node::new(NodeKind::Process, name);
    n.input_spec.add(2, "uint32_t");
    n.output_spec.add(1, "char");
    n
}

// ---------------------------------------------------------------------------
// Project::new
// ---------------------------------------------------------------------------

#[test]
fn new_project_has_schema_version() {
    let p = Project::new();
    assert_eq!(p.schema_version, SCHEMA_VERSION);
}

#[test]
fn new_project_has_root_diagram() {
    let p = Project::new();
    assert!(p.diagrams.contains_key(&p.root_diagram_id));
    assert!(p.orphaned_diagram_ids.is_empty());
}

// ---------------------------------------------------------------------------
// Project::create_sub_diagram — happy path
// ---------------------------------------------------------------------------

#[test]
fn create_sub_diagram_returns_new_id() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();
    assert!(p.diagrams.contains_key(&sub_id));
}

#[test]
fn create_sub_diagram_sets_process_link() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    let root = p.diagram(root_id).unwrap();
    assert_eq!(root.nodes[&pid].sub_diagram_id, Some(sub_id));
}

#[test]
fn create_sub_diagram_sets_parent_process_link() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    let sub = p.diagram(sub_id).unwrap();
    assert_eq!(sub.parent_process_id, Some(pid));
}

#[test]
fn create_sub_diagram_generates_boundary_sources_from_input_spec() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P")); // input: (2, uint32_t)
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    let sub = p.diagram(sub_id).unwrap();
    let boundary_sources: Vec<_> = sub
        .nodes
        .values()
        .filter(|n| n.is_boundary && n.kind == NodeKind::Source)
        .collect();

    // One boundary source for the single input entry (2, uint32_t).
    assert_eq!(boundary_sources.len(), 1);
    let bs = boundary_sources[0];
    assert_eq!(bs.output_spec.entries[0].count, 2);
    assert_eq!(bs.output_spec.entries[0].type_ref, "uint32_t");
    assert!(bs.input_spec.is_empty());
}

#[test]
fn create_sub_diagram_generates_boundary_sinks_from_output_spec() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P")); // output: (1, char)
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    let sub = p.diagram(sub_id).unwrap();
    let boundary_sinks: Vec<_> = sub
        .nodes
        .values()
        .filter(|n| n.is_boundary && n.kind == NodeKind::Sink)
        .collect();

    assert_eq!(boundary_sinks.len(), 1);
    let bs = boundary_sinks[0];
    assert_eq!(bs.input_spec.entries[0].count, 1);
    assert_eq!(bs.input_spec.entries[0].type_ref, "char");
    assert!(bs.output_spec.is_empty());
}

#[test]
fn create_sub_diagram_multiple_io_entries() {
    let mut p = project_with_types();
    let mut proc = Node::new(NodeKind::Process, "Multi");
    proc.input_spec.add(1, "uint32_t");
    proc.input_spec.add(3, "char"); // 2 input entries
    proc.output_spec.add(2, "uint32_t"); // 1 output entry
    let pid = add_to_root(&mut p, proc);
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    let sub = p.diagram(sub_id).unwrap();
    let sources = sub
        .nodes
        .values()
        .filter(|n| n.is_boundary && n.kind == NodeKind::Source)
        .count();
    let sinks = sub
        .nodes
        .values()
        .filter(|n| n.is_boundary && n.kind == NodeKind::Sink)
        .count();
    assert_eq!(sources, 2); // one per input entry
    assert_eq!(sinks, 1); // one per output entry
}

#[test]
fn create_sub_diagram_empty_io_generates_no_boundary_nodes() {
    let mut p = Project::new(); // no types needed — specs are empty
    let proc = Node::new(NodeKind::Process, "Empty");
    let pid = add_to_root(&mut p, proc);
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    let sub = p.diagram(sub_id).unwrap();
    assert!(sub.nodes.is_empty());
}

// ---------------------------------------------------------------------------
// create_sub_diagram — error paths
// ---------------------------------------------------------------------------

#[test]
fn create_sub_diagram_unknown_parent_diagram() {
    let mut p = Project::new();
    let ghost = Uuid::new_v4();
    let node_id = Uuid::new_v4();
    assert_eq!(
        p.create_sub_diagram(ghost, node_id),
        Err(ProjectError::DiagramNotFound(ghost))
    );
}

#[test]
fn create_sub_diagram_unknown_process_node() {
    let mut p = Project::new();
    let root_id = p.root_diagram_id;
    let ghost = Uuid::new_v4();
    assert_eq!(
        p.create_sub_diagram(root_id, ghost),
        Err(ProjectError::NodeNotFound(ghost))
    );
}

#[test]
fn create_sub_diagram_source_node_rejected() {
    let mut p = Project::new();
    let src = add_to_root(&mut p, Node::new(NodeKind::Source, "S"));
    let root_id = p.root_diagram_id;
    assert_eq!(
        p.create_sub_diagram(root_id, src),
        Err(ProjectError::NotAProcessNode(src))
    );
}

#[test]
fn create_sub_diagram_already_linked_rejected() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    p.create_sub_diagram(root_id, pid).unwrap();
    assert_eq!(
        p.create_sub_diagram(root_id, pid),
        Err(ProjectError::AlreadyHasSubDiagram(pid))
    );
}

// ---------------------------------------------------------------------------
// Project::orphan_sub_diagram
// ---------------------------------------------------------------------------

#[test]
fn orphan_sub_diagram_moves_to_orphaned_list() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    p.orphan_sub_diagram(sub_id).unwrap();
    assert!(p.is_orphaned(sub_id));
}

#[test]
fn orphan_sub_diagram_clears_parent_process_id() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    p.orphan_sub_diagram(sub_id).unwrap();
    assert_eq!(p.diagram(sub_id).unwrap().parent_process_id, None);
}

#[test]
fn orphan_sub_diagram_clears_process_node_link() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    p.orphan_sub_diagram(sub_id).unwrap();
    assert_eq!(p.diagram(root_id).unwrap().nodes[&pid].sub_diagram_id, None);
}

#[test]
fn orphan_sub_diagram_idempotent_on_repeated_calls() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    p.orphan_sub_diagram(sub_id).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap(); // second call is a no-op
    assert_eq!(
        p.orphaned_diagram_ids
            .iter()
            .filter(|&&x| x == sub_id)
            .count(),
        1
    );
}

#[test]
fn orphan_root_diagram_rejected() {
    let mut p = Project::new();
    let root_id = p.root_diagram_id;
    assert_eq!(
        p.orphan_sub_diagram(root_id),
        Err(ProjectError::IsRootDiagram(root_id))
    );
}

#[test]
fn orphan_unknown_diagram_rejected() {
    let mut p = Project::new();
    let ghost = Uuid::new_v4();
    assert_eq!(
        p.orphan_sub_diagram(ghost),
        Err(ProjectError::DiagramNotFound(ghost))
    );
}

// ---------------------------------------------------------------------------
// Project::attach_orphan
// ---------------------------------------------------------------------------

#[test]
fn attach_orphan_removes_from_orphaned_list() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap();

    let pid2 = add_to_root(&mut p, Node::new(NodeKind::Process, "P2"));
    p.attach_orphan(sub_id, pid2).unwrap();

    assert!(!p.is_orphaned(sub_id));
}

#[test]
fn attach_orphan_sets_parent_process_id() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap();

    let pid2 = add_to_root(&mut p, Node::new(NodeKind::Process, "P2"));
    p.attach_orphan(sub_id, pid2).unwrap();

    assert_eq!(p.diagram(sub_id).unwrap().parent_process_id, Some(pid2));
}

#[test]
fn attach_orphan_sets_process_node_link() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap();

    let pid2 = add_to_root(&mut p, Node::new(NodeKind::Process, "P2"));
    p.attach_orphan(sub_id, pid2).unwrap();

    assert_eq!(
        p.diagram(root_id).unwrap().nodes[&pid2].sub_diagram_id,
        Some(sub_id)
    );
}

#[test]
fn attach_orphan_non_orphaned_rejected() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();
    // sub_id is NOT orphaned
    let pid2 = add_to_root(&mut p, Node::new(NodeKind::Process, "P2"));
    assert_eq!(
        p.attach_orphan(sub_id, pid2),
        Err(ProjectError::NotOrphaned(sub_id))
    );
}

#[test]
fn attach_orphan_to_non_process_rejected() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap();

    let src = add_to_root(&mut p, Node::new(NodeKind::Source, "Src"));
    assert_eq!(
        p.attach_orphan(sub_id, src),
        Err(ProjectError::NotAProcessNode(src))
    );
}

#[test]
fn attach_orphan_to_already_linked_process_rejected() {
    let mut p = project_with_types();
    let root_id = p.root_diagram_id;

    let pid1 = add_to_root(&mut p, process_node("P1"));
    let sub1 = p.create_sub_diagram(root_id, pid1).unwrap();

    let pid2 = add_to_root(&mut p, process_node("P2"));
    let sub2 = p.create_sub_diagram(root_id, pid2).unwrap();

    // Orphan sub2 and try to attach to pid1 (which already has sub1).
    p.orphan_sub_diagram(sub2).unwrap();
    assert_eq!(
        p.attach_orphan(sub2, pid1),
        Err(ProjectError::AlreadyHasSubDiagram(pid1))
    );
}

// ---------------------------------------------------------------------------
// Project::update_boundary_nodes
// ---------------------------------------------------------------------------

#[test]
fn update_boundary_nodes_regenerates_from_new_spec() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P")); // input: (2, u32), output: (1, char)
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    // Change process output spec to add a char output entry.
    p.diagrams
        .get_mut(&root_id)
        .unwrap()
        .nodes
        .get_mut(&pid)
        .unwrap()
        .output_spec
        .add(4, "uint32_t");

    let dangled = p.update_boundary_nodes(sub_id).unwrap();
    // No streams existed so nothing dangled.
    assert!(dangled.is_empty());

    // New boundary sinks: one for (1, char) and one for (4, uint32_t).
    let sub = p.diagram(sub_id).unwrap();
    let sinks: Vec<_> = sub
        .nodes
        .values()
        .filter(|n| n.is_boundary && n.kind == NodeKind::Sink)
        .collect();
    assert_eq!(sinks.len(), 2);
}

#[test]
fn update_boundary_nodes_dangles_connected_streams() {
    use dataFlowClaude::model::stream::Stream;

    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    // Add a regular process node inside the sub-diagram and connect it
    // to one of the boundary sources.
    let inner_id = {
        let sub = p.diagrams.get_mut(&sub_id).unwrap();
        let inner = Node::new(NodeKind::Sink, "Inner");
        let iid = inner.id;
        sub.add_node(inner, (400.0, 200.0));
        iid
    };

    // Find the boundary source and wire it to the inner sink.
    let bsource_id = p
        .diagram(sub_id)
        .unwrap()
        .nodes
        .values()
        .find(|n| n.is_boundary && n.kind == NodeKind::Source)
        .map(|n| n.id)
        .unwrap();

    {
        let sub = p.diagrams.get_mut(&sub_id).unwrap();
        sub.add_stream(Stream::new(bsource_id, inner_id)).unwrap();
    }

    // Now update boundary nodes — the old boundary source is removed,
    // dangling the stream we just added.
    let dangled = p.update_boundary_nodes(sub_id).unwrap();
    assert!(!dangled.is_empty());

    // The stream still exists in the diagram but is dangling.
    let sub = p.diagram(sub_id).unwrap();
    for &sid in &dangled {
        assert!(sub.streams[&sid].is_dangling);
    }
}

#[test]
fn update_boundary_nodes_removes_all_old_boundary_nodes() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();

    let old_boundary_ids: Vec<_> = p
        .diagram(sub_id)
        .unwrap()
        .nodes
        .values()
        .filter(|n| n.is_boundary)
        .map(|n| n.id)
        .collect();

    p.update_boundary_nodes(sub_id).unwrap();

    let sub = p.diagram(sub_id).unwrap();
    for old_id in old_boundary_ids {
        assert!(
            !sub.nodes.contains_key(&old_id),
            "old boundary node should be gone"
        );
    }
}

#[test]
fn update_boundary_nodes_on_orphan_returns_error() {
    let mut p = project_with_types();
    let pid = add_to_root(&mut p, process_node("P"));
    let root_id = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(root_id, pid).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap();

    // Orphaned diagram has no parent_process_id — should return NoParentProcess.
    let result = p.update_boundary_nodes(sub_id);
    assert_eq!(result, Err(ProjectError::NoParentProcess(sub_id)));
}

#[test]
fn update_boundary_nodes_unknown_diagram_errors() {
    let mut p = Project::new();
    let ghost = Uuid::new_v4();
    assert_eq!(
        p.update_boundary_nodes(ghost),
        Err(ProjectError::DiagramNotFound(ghost))
    );
}
