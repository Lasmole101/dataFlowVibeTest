//! Tests for the node, stream, stream-group, and diagram model (spec §3, §6).
//!
//! Phase 2 coverage:
//! * NodeKind guard methods
//! * Node rename / attach / detach
//! * Stream helpers: touches_node, connects, dangling, group membership
//! * StreamGroup: deduplication, add/remove, auto-dissolve
//! * Diagram: add/remove node (dangling propagation), add/remove stream
//!   (group cleanup), self-loop rejection, group creation

use uuid::Uuid;

use dataFlowClaude::model::{
    diagram::{Diagram, DiagramError},
    node::{Node, NodeKind},
    stream::{Stream, StreamGroup},
    types::TypeSpec,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn source(name: &str) -> Node {
    Node::new(NodeKind::Source, name)
}
fn sink(name: &str) -> Node {
    Node::new(NodeKind::Sink, name)
}
fn proc(name: &str) -> Node {
    Node::new(NodeKind::Process, name)
}

fn place(d: &mut Diagram, n: Node) -> Uuid {
    let id = n.id;
    d.add_node(n, (0.0, 0.0));
    id
}

fn connect(d: &mut Diagram, src: Uuid, tgt: Uuid) -> Uuid {
    let s = Stream::new(src, tgt);
    let id = s.id;
    d.add_stream(s).unwrap();
    id
}

// ---------------------------------------------------------------------------
// NodeKind guard methods
// ---------------------------------------------------------------------------

#[test]
fn source_accepts_no_inputs() {
    assert!(!NodeKind::Source.accepts_inputs());
    assert!(NodeKind::Source.produces_outputs());
    assert!(!NodeKind::Source.can_have_subdiagram());
}

#[test]
fn sink_produces_no_outputs() {
    assert!(NodeKind::Sink.accepts_inputs());
    assert!(!NodeKind::Sink.produces_outputs());
    assert!(!NodeKind::Sink.can_have_subdiagram());
}

#[test]
fn process_accepts_and_produces() {
    assert!(NodeKind::Process.accepts_inputs());
    assert!(NodeKind::Process.produces_outputs());
    assert!(NodeKind::Process.can_have_subdiagram());
}

// ---------------------------------------------------------------------------
// Node: boundary lock
// ---------------------------------------------------------------------------

#[test]
fn boundary_node_is_not_renameable_or_deletable() {
    let b = Node::new_boundary(
        NodeKind::Source,
        "BoundaryIn",
        TypeSpec::new(),
        TypeSpec::new(),
    );
    assert!(b.is_boundary);
    assert!(!b.is_renameable());
    assert!(!b.is_deletable());
}

#[test]
fn non_boundary_node_is_renameable_and_deletable() {
    let n = source("S");
    assert!(!n.is_boundary);
    assert!(n.is_renameable());
    assert!(n.is_deletable());
}

#[test]
fn rename_returns_false_for_boundary_node() {
    let mut b = Node::new_boundary(NodeKind::Sink, "Locked", TypeSpec::new(), TypeSpec::new());
    let ok = b.rename("New");
    assert!(!ok);
    assert_eq!(b.name, "Locked");
}

#[test]
fn rename_returns_true_for_normal_node() {
    let mut n = sink("Old");
    let ok = n.rename("New");
    assert!(ok);
    assert_eq!(n.name, "New");
}

// ---------------------------------------------------------------------------
// Node: sub-diagram attachment
// ---------------------------------------------------------------------------

#[test]
fn attach_subdiagram_to_process_succeeds() {
    let mut p = proc("P");
    let did = Uuid::new_v4();
    assert!(p.attach_subdiagram(did));
    assert_eq!(p.sub_diagram_id, Some(did));
    assert!(p.has_subdiagram());
}

#[test]
fn attach_subdiagram_to_source_fails() {
    let mut s = source("S");
    let did = Uuid::new_v4();
    assert!(!s.attach_subdiagram(did));
    assert_eq!(s.sub_diagram_id, None);
}

#[test]
fn detach_subdiagram_returns_id_and_clears() {
    let mut p = proc("P");
    let did = Uuid::new_v4();
    p.attach_subdiagram(did);
    let returned = p.detach_subdiagram();
    assert_eq!(returned, Some(did));
    assert!(!p.has_subdiagram());
}

#[test]
fn detach_when_none_returns_none() {
    let mut p = proc("P");
    assert_eq!(p.detach_subdiagram(), None);
}

// ---------------------------------------------------------------------------
// Stream: construction and queries
// ---------------------------------------------------------------------------

#[test]
fn new_stream_is_not_dangling_and_not_grouped() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let s = Stream::new(a, b);
    assert!(!s.is_dangling);
    assert!(!s.is_grouped());
    assert!(s.touches_node(a));
    assert!(s.touches_node(b));
    assert!(!s.touches_node(Uuid::new_v4()));
}

#[test]
fn connects_checks_direction() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let s = Stream::new(a, b);
    assert!(s.connects(a, b));
    assert!(!s.connects(b, a));
}

#[test]
fn mark_dangling_sets_flag() {
    let mut s = Stream::new(Uuid::new_v4(), Uuid::new_v4());
    s.mark_dangling();
    assert!(s.is_dangling);
}

#[test]
fn set_and_clear_group() {
    let mut s = Stream::new(Uuid::new_v4(), Uuid::new_v4());
    let gid = Uuid::new_v4();
    s.set_group(gid);
    assert_eq!(s.group_id, Some(gid));
    assert!(s.is_grouped());
    let prev = s.clear_group();
    assert_eq!(prev, Some(gid));
    assert!(!s.is_grouped());
}

// ---------------------------------------------------------------------------
// StreamGroup: deduplication and membership
// ---------------------------------------------------------------------------

#[test]
fn stream_group_new_deduplicates() {
    let id = Uuid::new_v4();
    let g = StreamGroup::new("G", vec![id, id, id]);
    assert_eq!(g.member_stream_ids.len(), 1);
}

#[test]
fn stream_group_add_stream_no_duplicate() {
    let id = Uuid::new_v4();
    let mut g = StreamGroup::new("G", vec![id]);
    let added = g.add_stream(id);
    assert!(!added);
    assert_eq!(g.len(), 1);
}

#[test]
fn stream_group_add_and_remove() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let mut g = StreamGroup::new("G", vec![a]);
    g.add_stream(b);
    assert_eq!(g.len(), 2);
    assert!(g.contains(b));
    let removed = g.remove_stream(a);
    assert!(removed);
    assert!(!g.contains(a));
    assert_eq!(g.len(), 1);
}

#[test]
fn stream_group_is_empty_after_all_removed() {
    let id = Uuid::new_v4();
    let mut g = StreamGroup::new("G", vec![id]);
    g.remove_stream(id);
    assert!(g.is_empty());
}

// ---------------------------------------------------------------------------
// Diagram: self-loop rejection
// ---------------------------------------------------------------------------

#[test]
fn add_stream_rejects_self_loop() {
    let mut d = Diagram::new("D");
    let nid = place(&mut d, source("S"));
    let self_stream = Stream::new(nid, nid);
    let err = d.add_stream(self_stream).unwrap_err();
    assert_eq!(err, DiagramError::SelfLoop(nid));
    assert!(d.streams.is_empty());
}

// ---------------------------------------------------------------------------
// Diagram: remove_node marks streams dangling
// ---------------------------------------------------------------------------

#[test]
fn remove_node_not_found_errors() {
    let mut d = Diagram::new("D");
    let ghost = Uuid::new_v4();
    assert_eq!(d.remove_node(ghost), Err(DiagramError::NodeNotFound(ghost)));
}

#[test]
fn remove_node_returns_node_and_position() {
    let mut d = Diagram::new("D");
    let n = source("S");
    let nid = n.id;
    d.add_node(n, (10.0, 20.0));
    let removed = d.remove_node(nid).unwrap();
    assert_eq!(removed.node.id, nid);
    assert_eq!(removed.position, (10.0, 20.0));
    assert!(!d.nodes.contains_key(&nid));
    assert!(!d.positions.contains_key(&nid));
}

#[test]
fn remove_node_marks_outgoing_stream_dangling() {
    let mut d = Diagram::new("D");
    let src_id = place(&mut d, source("Src"));
    let tgt_id = place(&mut d, sink("Tgt"));
    let sid = connect(&mut d, src_id, tgt_id);

    let removed = d.remove_node(src_id).unwrap();
    assert!(removed.dangled_stream_ids.contains(&sid));
    assert!(d.streams[&sid].is_dangling);
}

#[test]
fn remove_node_marks_incoming_stream_dangling() {
    let mut d = Diagram::new("D");
    let src_id = place(&mut d, source("Src"));
    let tgt_id = place(&mut d, sink("Tgt"));
    let sid = connect(&mut d, src_id, tgt_id);

    let removed = d.remove_node(tgt_id).unwrap();
    assert!(removed.dangled_stream_ids.contains(&sid));
    assert!(d.streams[&sid].is_dangling);
}

#[test]
fn remove_node_unaffected_streams_stay_intact() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, proc("B"));
    let c = place(&mut d, sink("C"));
    let ab = connect(&mut d, a, b);
    let bc = connect(&mut d, b, c);

    // Remove an unrelated node.
    let extra = place(&mut d, source("Extra"));
    d.remove_node(extra).unwrap();

    // Both streams remain intact.
    assert!(!d.streams[&ab].is_dangling);
    assert!(!d.streams[&bc].is_dangling);
}

#[test]
fn remove_node_evicts_stream_from_group_and_dissolves_if_empty() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, sink("B"));
    let c = place(&mut d, sink("C"));
    // Two streams from A: one to B (will be dangled), one to C.
    let ab = connect(&mut d, a, b);
    let ac = connect(&mut d, a, c);
    // Group contains only ab.
    let gid = d.create_group("G", vec![ab]).unwrap();

    // Remove A — ab is dangled and removed from the group.
    d.remove_node(a).unwrap();

    // Group should be dissolved (its only member was ab).
    assert!(!d.stream_groups.contains_key(&gid));
    // ac is also dangled (it also originated from A).
    assert!(d.streams[&ac].is_dangling);
}

#[test]
fn remove_node_partial_group_survives() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, sink("B"));
    let c = place(&mut d, source("C"));
    let e = place(&mut d, sink("E"));
    let ab = connect(&mut d, a, b);
    let ce = connect(&mut d, c, e);
    // Group has both ab and ce.
    let gid = d.create_group("G", vec![ab, ce]).unwrap();

    // Remove A — ab dangled and removed from group.  ce untouched.
    d.remove_node(a).unwrap();

    // Group still exists with only ce.
    assert!(d.stream_groups.contains_key(&gid));
    assert!(!d.stream_groups[&gid].contains(ab));
    assert!(d.stream_groups[&gid].contains(ce));
}

// ---------------------------------------------------------------------------
// Diagram: remove_stream cleans up groups
// ---------------------------------------------------------------------------

#[test]
fn remove_stream_not_found_errors() {
    let mut d = Diagram::new("D");
    let ghost = Uuid::new_v4();
    assert_eq!(
        d.remove_stream(ghost),
        Err(DiagramError::StreamNotFound(ghost))
    );
}

#[test]
fn remove_stream_deletes_stream_and_returns_it() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, sink("B"));
    let sid = connect(&mut d, a, b);
    let removed = d.remove_stream(sid).unwrap();
    assert_eq!(removed.id, sid);
    assert!(!d.streams.contains_key(&sid));
}

#[test]
fn remove_stream_dissolves_empty_group() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, sink("B"));
    let sid = connect(&mut d, a, b);
    let gid = d.create_group("G", vec![sid]).unwrap();

    d.remove_stream(sid).unwrap();
    assert!(!d.stream_groups.contains_key(&gid));
}

#[test]
fn remove_stream_keeps_non_empty_group() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, sink("B"));
    let c = place(&mut d, sink("C"));
    let ab = connect(&mut d, a, b);
    let ac = connect(&mut d, a, c);
    let gid = d.create_group("G", vec![ab, ac]).unwrap();

    d.remove_stream(ab).unwrap();
    // Group still has ac.
    assert!(d.stream_groups.contains_key(&gid));
    assert!(!d.stream_groups[&gid].contains(ab));
    assert!(d.stream_groups[&gid].contains(ac));
}

// ---------------------------------------------------------------------------
// Diagram: group operations
// ---------------------------------------------------------------------------

#[test]
fn create_group_rejects_unknown_stream() {
    let mut d = Diagram::new("D");
    let ghost = Uuid::new_v4();
    let err = d.create_group("G", vec![ghost]).unwrap_err();
    assert_eq!(err, DiagramError::StreamNotFound(ghost));
}

#[test]
fn create_group_sets_group_id_on_streams() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, sink("B"));
    let c = place(&mut d, sink("C"));
    let ab = connect(&mut d, a, b);
    let ac = connect(&mut d, a, c);

    let gid = d.create_group("G", vec![ab, ac]).unwrap();
    assert_eq!(d.streams[&ab].group_id, Some(gid));
    assert_eq!(d.streams[&ac].group_id, Some(gid));
}

// ---------------------------------------------------------------------------
// Diagram: query helpers
// ---------------------------------------------------------------------------

#[test]
fn incoming_and_outgoing_streams() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, proc("B"));
    let c = place(&mut d, sink("C"));
    let ab = connect(&mut d, a, b);
    let bc = connect(&mut d, b, c);

    let out_b: Vec<_> = d.outgoing_streams(b).map(|s| s.id).collect();
    assert_eq!(out_b, vec![bc]);

    let in_b: Vec<_> = d.incoming_streams(b).map(|s| s.id).collect();
    assert_eq!(in_b, vec![ab]);
}

#[test]
fn streams_for_node_returns_both_directions() {
    let mut d = Diagram::new("D");
    let a = place(&mut d, source("A"));
    let b = place(&mut d, proc("B"));
    let c = place(&mut d, sink("C"));
    let ab = connect(&mut d, a, b);
    let bc = connect(&mut d, b, c);

    let mut ids: Vec<_> = d.streams_for_node(b).map(|s| s.id).collect();
    ids.sort();
    let mut expected = vec![ab, bc];
    expected.sort();
    assert_eq!(ids, expected);
}
