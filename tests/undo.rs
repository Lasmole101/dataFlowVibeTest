//! Tests for the undo/redo subsystem (spec §5.5).
//!
//! Covers: UndoStack mechanics, CompoundCommand, all concrete commands,
//! and round-trip apply→undo→redo for each.

use uuid::Uuid;

use dataFlowClaude::{
    model::{
        node::{Node, NodeKind},
        project::Project,
        stream::Stream,
        stream::StreamGroup,
        types::{CompositeType, PrimitiveType, TypeSpec},
    },
    undo::{
        commands::{
            AddCompositeType, AddNode, AddPrimitiveType, AddStream, AttachOrphan,
            CreateStreamGroup, CreateSubDiagram, DeleteNode, DeleteStream, DeleteStreamGroup,
            DeleteType, EditNodeSpec, EditStreamGroup, EditStreamSpec, MoveNode, NodeSpecSide,
            OrphanSubDiagram, RenameNode, RenameType,
        },
        history::{CompoundCommand, UndoStack},
    },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base_project() -> Project {
    let mut p = Project::new();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("int"))
        .unwrap();
    p
}

fn root_id(p: &Project) -> Uuid {
    p.root_diagram_id
}

fn add_raw(p: &mut Project, node: Node) -> Uuid {
    let id = node.id;
    let rid = p.root_diagram_id;
    p.diagrams.get_mut(&rid).unwrap().add_node(node, (0.0, 0.0));
    id
}

fn connect_raw(p: &mut Project, src: Uuid, tgt: Uuid) -> Uuid {
    let s = Stream::new(src, tgt);
    let id = s.id;
    let rid = p.root_diagram_id;
    p.diagrams.get_mut(&rid).unwrap().add_stream(s).unwrap();
    id
}

fn spec(pairs: &[(u32, &str)]) -> TypeSpec {
    TypeSpec::from_pairs(pairs.iter().copied().map(|(c, n)| (c, n)))
}

fn execute<C: dataFlowClaude::undo::history::Command + 'static>(
    stack: &mut UndoStack,
    cmd: C,
    project: &mut Project,
) {
    stack.execute(Box::new(cmd), project).unwrap();
}

// ---------------------------------------------------------------------------
// UndoStack mechanics
// ---------------------------------------------------------------------------

#[test]
fn empty_stack_cannot_undo_or_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    assert!(!stack.can_undo());
    assert!(!stack.can_redo());
    stack.undo(&mut p).unwrap(); // should not panic
    stack.redo(&mut p).unwrap();
}

#[test]
fn execute_clears_future() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let src = Node::new(NodeKind::Source, "S");
    let src_id = src.id;
    execute(
        &mut stack,
        AddNode {
            diagram_id: root_id(&p),
            node: src,
            position: (0.0, 0.0),
        },
        &mut p,
    );
    stack.undo(&mut p).unwrap();
    assert!(stack.can_redo());
    // Execute a new command — future must be cleared.
    let src2 = Node::new(NodeKind::Source, "S2");
    execute(
        &mut stack,
        AddNode {
            diagram_id: root_id(&p),
            node: src2,
            position: (0.0, 0.0),
        },
        &mut p,
    );
    assert!(!stack.can_redo());
}

#[test]
fn undo_depth_bounded_by_max() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    // Push MAX_UNDO_DEPTH + 5 commands.
    for _ in 0..(dataFlowClaude::undo::history::MAX_UNDO_DEPTH + 5) {
        let n = Node::new(NodeKind::Source, "S");
        execute(
            &mut stack,
            AddNode {
                diagram_id: rid,
                node: n,
                position: (0.0, 0.0),
            },
            &mut p,
        );
    }
    assert_eq!(
        stack.undo_depth(),
        dataFlowClaude::undo::history::MAX_UNDO_DEPTH
    );
}

#[test]
fn description_reflects_top_of_stack() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    execute(
        &mut stack,
        AddNode {
            diagram_id: root_id(&p),
            node: Node::new(NodeKind::Source, "S"),
            position: (0.0, 0.0),
        },
        &mut p,
    );
    assert_eq!(stack.undo_description(), Some("Add node"));
    stack.undo(&mut p).unwrap();
    assert_eq!(stack.redo_description(), Some("Add node"));
}

// ---------------------------------------------------------------------------
// CompoundCommand
// ---------------------------------------------------------------------------

#[test]
fn compound_applies_in_order_and_undoes_in_reverse() {
    let mut p = base_project();
    let rid = root_id(&p);
    let src = Node::new(NodeKind::Source, "Src");
    let tgt = Node::new(NodeKind::Sink, "Tgt");
    let src_id = src.id;
    let tgt_id = tgt.id;

    let stream = Stream::new(src_id, tgt_id);
    let sid = stream.id;

    let compound = CompoundCommand::new(
        "Add nodes and stream",
        vec![
            Box::new(AddNode {
                diagram_id: rid,
                node: src,
                position: (0.0, 0.0),
            }),
            Box::new(AddNode {
                diagram_id: rid,
                node: tgt,
                position: (200.0, 0.0),
            }),
            Box::new(AddStream {
                diagram_id: rid,
                stream,
            }),
        ],
    );

    let mut stack = UndoStack::default();
    stack.execute(Box::new(compound), &mut p).unwrap();

    let diag = p.diagram(rid).unwrap();
    assert!(diag.nodes.contains_key(&src_id));
    assert!(diag.nodes.contains_key(&tgt_id));
    assert!(diag.streams.contains_key(&sid));

    stack.undo(&mut p).unwrap();

    let diag = p.diagram(rid).unwrap();
    assert!(!diag.nodes.contains_key(&src_id));
    assert!(!diag.nodes.contains_key(&tgt_id));
    assert!(!diag.streams.contains_key(&sid));
}

#[test]
fn compound_rolls_back_on_partial_failure() {
    let mut p = base_project();
    let rid = root_id(&p);
    let src = Node::new(NodeKind::Source, "Src");
    let src_id = src.id;

    // Second command references a node that doesn't exist — apply should fail.
    let ghost_stream = Stream::new(src_id, Uuid::new_v4()); // target doesn't exist but add_stream allows it
                                                            // Use a DeleteNode on a non-existent node to force failure.
    let bad_node = Node::new(NodeKind::Sink, "Ghost");
    let bad_id = bad_node.id; // never added to diagram

    let compound = CompoundCommand::new(
        "Bad compound",
        vec![
            Box::new(AddNode {
                diagram_id: rid,
                node: src,
                position: (0.0, 0.0),
            }),
            // This will fail: node not in diagram.
            Box::new(DeleteNode {
                diagram_id: rid,
                node: bad_node,
                position: (0.0, 0.0),
            }),
        ],
    );

    let mut stack = UndoStack::default();
    let result = stack.execute(Box::new(compound), &mut p);
    assert!(result.is_err());
    // Rollback: src_id should have been removed.
    assert!(!p.diagram(rid).unwrap().nodes.contains_key(&src_id));
    // Nothing should be on the stack.
    assert!(!stack.can_undo());
}

// ---------------------------------------------------------------------------
// AddNode / DeleteNode round-trip
// ---------------------------------------------------------------------------

#[test]
fn add_node_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let node = Node::new(NodeKind::Source, "S");
    let nid = node.id;

    execute(
        &mut stack,
        AddNode {
            diagram_id: rid,
            node,
            position: (10.0, 20.0),
        },
        &mut p,
    );
    assert!(p.diagram(rid).unwrap().nodes.contains_key(&nid));

    stack.undo(&mut p).unwrap();
    assert!(!p.diagram(rid).unwrap().nodes.contains_key(&nid));

    stack.redo(&mut p).unwrap();
    assert!(p.diagram(rid).unwrap().nodes.contains_key(&nid));
    assert_eq!(p.diagram(rid).unwrap().position_of(nid), Some((10.0, 20.0)));
}

#[test]
fn delete_node_undo_restores_position() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let node = Node::new(NodeKind::Source, "S");
    let nid = node.id;
    add_raw(&mut p, node.clone());

    execute(
        &mut stack,
        DeleteNode {
            diagram_id: rid,
            node,
            position: (5.0, 7.0),
        },
        &mut p,
    );
    assert!(!p.diagram(rid).unwrap().nodes.contains_key(&nid));

    stack.undo(&mut p).unwrap();
    assert!(p.diagram(rid).unwrap().nodes.contains_key(&nid));
    // Position is restored.
    assert_eq!(p.diagram(rid).unwrap().position_of(nid), Some((5.0, 7.0)));
}

// ---------------------------------------------------------------------------
// MoveNode
// ---------------------------------------------------------------------------

#[test]
fn move_node_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let node = Node::new(NodeKind::Source, "S");
    let nid = node.id;
    add_raw(&mut p, node);
    p.diagram_mut(rid)
        .unwrap()
        .positions
        .insert(nid, (0.0, 0.0));

    execute(
        &mut stack,
        MoveNode {
            diagram_id: rid,
            node_id: nid,
            from: (0.0, 0.0),
            to: (100.0, 200.0),
        },
        &mut p,
    );
    assert_eq!(
        p.diagram(rid).unwrap().position_of(nid),
        Some((100.0, 200.0))
    );

    stack.undo(&mut p).unwrap();
    assert_eq!(p.diagram(rid).unwrap().position_of(nid), Some((0.0, 0.0)));

    stack.redo(&mut p).unwrap();
    assert_eq!(
        p.diagram(rid).unwrap().position_of(nid),
        Some((100.0, 200.0))
    );
}

// ---------------------------------------------------------------------------
// RenameNode
// ---------------------------------------------------------------------------

#[test]
fn rename_node_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let node = Node::new(NodeKind::Source, "Old");
    let nid = node.id;
    add_raw(&mut p, node);

    execute(
        &mut stack,
        RenameNode {
            diagram_id: rid,
            node_id: nid,
            old_name: "Old".into(),
            new_name: "New".into(),
        },
        &mut p,
    );
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].name, "New");

    stack.undo(&mut p).unwrap();
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].name, "Old");

    stack.redo(&mut p).unwrap();
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].name, "New");
}

// ---------------------------------------------------------------------------
// EditNodeSpec
// ---------------------------------------------------------------------------

#[test]
fn edit_node_spec_input_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let mut node = Node::new(NodeKind::Process, "P");
    node.input_spec.add(1, "int");
    let nid = node.id;
    add_raw(&mut p, node);

    let before = spec(&[(1, "int")]);
    let after = spec(&[(3, "int")]);

    execute(
        &mut stack,
        EditNodeSpec {
            diagram_id: rid,
            node_id: nid,
            side: NodeSpecSide::Input,
            before: before.clone(),
            after: after.clone(),
        },
        &mut p,
    );
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].input_spec, after);

    stack.undo(&mut p).unwrap();
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].input_spec, before);

    stack.redo(&mut p).unwrap();
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].input_spec, after);
}

#[test]
fn edit_node_spec_output_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let mut node = Node::new(NodeKind::Process, "P");
    node.output_spec.add(2, "int");
    let nid = node.id;
    add_raw(&mut p, node);

    let before = spec(&[(2, "int")]);
    let after = spec(&[(5, "int")]);

    execute(
        &mut stack,
        EditNodeSpec {
            diagram_id: rid,
            node_id: nid,
            side: NodeSpecSide::Output,
            before: before.clone(),
            after: after.clone(),
        },
        &mut p,
    );
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].output_spec, after);

    stack.undo(&mut p).unwrap();
    assert_eq!(p.diagram(rid).unwrap().nodes[&nid].output_spec, before);
}

// ---------------------------------------------------------------------------
// AddStream / DeleteStream / EditStreamSpec
// ---------------------------------------------------------------------------

#[test]
fn add_stream_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let src_id = add_raw(&mut p, Node::new(NodeKind::Source, "S"));
    let tgt_id = add_raw(&mut p, Node::new(NodeKind::Sink, "T"));

    let stream = Stream::new(src_id, tgt_id);
    let sid = stream.id;
    execute(
        &mut stack,
        AddStream {
            diagram_id: rid,
            stream,
        },
        &mut p,
    );
    assert!(p.diagram(rid).unwrap().streams.contains_key(&sid));

    stack.undo(&mut p).unwrap();
    assert!(!p.diagram(rid).unwrap().streams.contains_key(&sid));

    stack.redo(&mut p).unwrap();
    assert!(p.diagram(rid).unwrap().streams.contains_key(&sid));
}

#[test]
fn delete_stream_undo_restores_stream() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let src_id = add_raw(&mut p, Node::new(NodeKind::Source, "S"));
    let tgt_id = add_raw(&mut p, Node::new(NodeKind::Sink, "T"));
    let sid = connect_raw(&mut p, src_id, tgt_id);
    let stream = p.diagram(rid).unwrap().streams[&sid].clone();

    execute(
        &mut stack,
        DeleteStream {
            diagram_id: rid,
            stream,
        },
        &mut p,
    );
    assert!(!p.diagram(rid).unwrap().streams.contains_key(&sid));

    stack.undo(&mut p).unwrap();
    assert!(p.diagram(rid).unwrap().streams.contains_key(&sid));
}

#[test]
fn edit_stream_spec_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let src_id = add_raw(&mut p, Node::new(NodeKind::Source, "S"));
    let tgt_id = add_raw(&mut p, Node::new(NodeKind::Sink, "T"));
    let sid = connect_raw(&mut p, src_id, tgt_id);

    let before = TypeSpec::new();
    let after = spec(&[(2, "int")]);
    execute(
        &mut stack,
        EditStreamSpec {
            diagram_id: rid,
            stream_id: sid,
            before: before.clone(),
            after: after.clone(),
        },
        &mut p,
    );
    assert_eq!(p.diagram(rid).unwrap().streams[&sid].type_spec, after);

    stack.undo(&mut p).unwrap();
    assert_eq!(p.diagram(rid).unwrap().streams[&sid].type_spec, before);
}

// ---------------------------------------------------------------------------
// Stream group commands
// ---------------------------------------------------------------------------

#[test]
fn create_stream_group_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let src = add_raw(&mut p, Node::new(NodeKind::Source, "S"));
    let tgt = add_raw(&mut p, Node::new(NodeKind::Sink, "T"));
    let sid = connect_raw(&mut p, src, tgt);
    let gid = Uuid::new_v4();

    execute(
        &mut stack,
        CreateStreamGroup {
            diagram_id: rid,
            name: "G".into(),
            stream_ids: vec![sid],
            group_id: gid,
        },
        &mut p,
    );
    assert!(p.diagram(rid).unwrap().stream_groups.contains_key(&gid));
    assert_eq!(p.diagram(rid).unwrap().streams[&sid].group_id, Some(gid));

    stack.undo(&mut p).unwrap();
    assert!(!p.diagram(rid).unwrap().stream_groups.contains_key(&gid));
    assert_eq!(p.diagram(rid).unwrap().streams[&sid].group_id, None);

    stack.redo(&mut p).unwrap();
    assert!(p.diagram(rid).unwrap().stream_groups.contains_key(&gid));
}

#[test]
fn delete_stream_group_undo_restores() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let src = add_raw(&mut p, Node::new(NodeKind::Source, "S"));
    let tgt = add_raw(&mut p, Node::new(NodeKind::Sink, "T"));
    let sid = connect_raw(&mut p, src, tgt);
    let gid = p
        .diagram_mut(rid)
        .unwrap()
        .create_group("G", vec![sid])
        .unwrap();
    let group = p.diagram(rid).unwrap().stream_groups[&gid].clone();

    execute(
        &mut stack,
        DeleteStreamGroup {
            diagram_id: rid,
            group,
        },
        &mut p,
    );
    assert!(!p.diagram(rid).unwrap().stream_groups.contains_key(&gid));

    stack.undo(&mut p).unwrap();
    assert!(p.diagram(rid).unwrap().stream_groups.contains_key(&gid));
    assert_eq!(p.diagram(rid).unwrap().streams[&sid].group_id, Some(gid));
}

#[test]
fn edit_stream_group_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);
    let src = add_raw(&mut p, Node::new(NodeKind::Source, "S"));
    let t1 = add_raw(&mut p, Node::new(NodeKind::Sink, "T1"));
    let t2 = add_raw(&mut p, Node::new(NodeKind::Sink, "T2"));
    let s1 = connect_raw(&mut p, src, t1);
    let s2 = connect_raw(&mut p, src, t2);
    let gid = p
        .diagram_mut(rid)
        .unwrap()
        .create_group("G", vec![s1])
        .unwrap();

    let before = p.diagram(rid).unwrap().stream_groups[&gid].clone();
    let mut after = before.clone();
    after.add_stream(s2);

    execute(
        &mut stack,
        EditStreamGroup {
            diagram_id: rid,
            group_id: gid,
            before: before.clone(),
            after: after.clone(),
        },
        &mut p,
    );
    assert!(p.diagram(rid).unwrap().stream_groups[&gid].contains(s2));

    stack.undo(&mut p).unwrap();
    assert!(!p.diagram(rid).unwrap().stream_groups[&gid].contains(s2));
}

// ---------------------------------------------------------------------------
// Dictionary commands
// ---------------------------------------------------------------------------

#[test]
fn add_primitive_type_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = Project::new();
    execute(
        &mut stack,
        AddPrimitiveType {
            primitive: PrimitiveType::new("u8"),
        },
        &mut p,
    );
    assert!(p.data_dictionary.contains("u8"));

    stack.undo(&mut p).unwrap();
    assert!(!p.data_dictionary.contains("u8"));

    stack.redo(&mut p).unwrap();
    assert!(p.data_dictionary.contains("u8"));
}

#[test]
fn add_composite_type_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = Project::new();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("int"))
        .unwrap();
    let comp = CompositeType::new("Pair", spec(&[(2, "int")]));

    execute(&mut stack, AddCompositeType { composite: comp }, &mut p);
    assert!(p.data_dictionary.contains("Pair"));

    stack.undo(&mut p).unwrap();
    assert!(!p.data_dictionary.contains("Pair"));

    stack.redo(&mut p).unwrap();
    assert!(p.data_dictionary.contains("Pair"));
}

#[test]
fn delete_primitive_type_undo_redo() {
    let mut stack = UndoStack::default();
    let mut p = Project::new();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("int"))
        .unwrap();
    let prim = p.data_dictionary.primitive("int").unwrap().clone();

    execute(&mut stack, DeleteType::primitive(prim), &mut p);
    assert!(!p.data_dictionary.contains("int"));

    stack.undo(&mut p).unwrap();
    assert!(p.data_dictionary.contains("int"));

    stack.redo(&mut p).unwrap();
    assert!(!p.data_dictionary.contains("int"));
}

#[test]
fn rename_type_undo_redo_patches_specs() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);

    // Add a node whose output spec references "int".
    let mut node = Node::new(NodeKind::Source, "S");
    node.output_spec.add(1, "int");
    let nid = node.id;
    add_raw(&mut p, node);

    execute(
        &mut stack,
        RenameType {
            old_name: "int".into(),
            new_name: "i32".into(),
            patched_composites: vec![],
        },
        &mut p,
    );
    // Dictionary renamed.
    assert!(p.data_dictionary.contains("i32"));
    assert!(!p.data_dictionary.contains("int"));
    // Node spec patched.
    assert_eq!(
        p.diagram(rid).unwrap().nodes[&nid].output_spec.entries[0].type_ref,
        "i32"
    );

    stack.undo(&mut p).unwrap();
    assert!(p.data_dictionary.contains("int"));
    assert_eq!(
        p.diagram(rid).unwrap().nodes[&nid].output_spec.entries[0].type_ref,
        "int"
    );

    stack.redo(&mut p).unwrap();
    assert!(p.data_dictionary.contains("i32"));
    assert_eq!(
        p.diagram(rid).unwrap().nodes[&nid].output_spec.entries[0].type_ref,
        "i32"
    );
}

// ---------------------------------------------------------------------------
// Sub-diagram commands
// ---------------------------------------------------------------------------

#[test]
fn create_sub_diagram_undo_removes_diagram_and_unlinks() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);

    let mut proc = Node::new(NodeKind::Process, "P");
    proc.input_spec.add(1, "int");
    let pid = proc.id;
    add_raw(&mut p, proc);

    execute(
        &mut stack,
        CreateSubDiagram {
            parent_diagram_id: rid,
            process_node_id: pid,
            sub_diagram_id: Uuid::new_v4(),
        },
        &mut p,
    );
    assert!(p.diagram(rid).unwrap().nodes[&pid].sub_diagram_id.is_some());
    assert_eq!(p.diagrams.len(), 2);

    stack.undo(&mut p).unwrap();
    assert!(p.diagram(rid).unwrap().nodes[&pid].sub_diagram_id.is_none());
    assert_eq!(p.diagrams.len(), 1);
}

#[test]
fn orphan_and_attach_round_trip() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);

    let proc = Node::new(NodeKind::Process, "P");
    let pid = proc.id;
    add_raw(&mut p, proc);
    let sub_id = p.create_sub_diagram(rid, pid).unwrap();

    execute(
        &mut stack,
        OrphanSubDiagram {
            diagram_id: sub_id,
            parent_process_node_id: pid,
        },
        &mut p,
    );
    assert!(p.is_orphaned(sub_id));
    assert!(p.diagram(rid).unwrap().nodes[&pid].sub_diagram_id.is_none());

    stack.undo(&mut p).unwrap();
    assert!(!p.is_orphaned(sub_id));
    assert_eq!(
        p.diagram(rid).unwrap().nodes[&pid].sub_diagram_id,
        Some(sub_id)
    );
}

#[test]
fn attach_orphan_undo_re_orphans() {
    let mut stack = UndoStack::default();
    let mut p = base_project();
    let rid = root_id(&p);

    // Create a process + sub-diagram, then orphan it manually.
    let p1 = Node::new(NodeKind::Process, "P1");
    let pid1 = p1.id;
    add_raw(&mut p, p1);
    let sub_id = p.create_sub_diagram(rid, pid1).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap();

    // New process to attach to.
    let p2 = Node::new(NodeKind::Process, "P2");
    let pid2 = p2.id;
    add_raw(&mut p, p2);

    execute(
        &mut stack,
        AttachOrphan {
            orphan_diagram_id: sub_id,
            process_node_id: pid2,
        },
        &mut p,
    );
    assert!(!p.is_orphaned(sub_id));
    assert_eq!(
        p.diagram(rid).unwrap().nodes[&pid2].sub_diagram_id,
        Some(sub_id)
    );

    stack.undo(&mut p).unwrap();
    assert!(p.is_orphaned(sub_id));
    assert!(p.diagram(rid).unwrap().nodes[&pid2]
        .sub_diagram_id
        .is_none());
}
