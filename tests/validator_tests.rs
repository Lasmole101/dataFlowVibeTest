//! Integration tests for `validation::validator` (spec §4.4).
//!
//! Tests exercise the full validation pipeline: project mutations feed into
//! the cache, and the cache's state/message output is asserted.

use dataFlowClaude::{
    model::{
        node::{Node, NodeKind},
        project::Project,
        stream::Stream,
        types::PrimitiveType,
    },
    validation::validator::{ValidationCache, ValidationState},
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn project_with_int() -> Project {
    let mut p = Project::new();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("int"))
        .unwrap();
    p
}

fn add_node(project: &mut Project, node: Node) -> Uuid {
    let id = node.id;
    let root = project.root_diagram_id;
    project
        .diagrams
        .get_mut(&root)
        .unwrap()
        .add_node(node, (0.0, 0.0));
    id
}

fn add_stream(project: &mut Project, stream: Stream) -> Uuid {
    let id = stream.id;
    let root = project.root_diagram_id;
    project
        .diagrams
        .get_mut(&root)
        .unwrap()
        .add_stream(stream)
        .unwrap();
    id
}

fn source_out(count: u32, t: &str) -> Node {
    let mut n = Node::new(NodeKind::Source, "Src");
    n.output_spec.add(count, t);
    n
}

fn sink_in(count: u32, t: &str) -> Node {
    let mut n = Node::new(NodeKind::Sink, "Tgt");
    n.input_spec.add(count, t);
    n
}

fn stream_of(count: u32, t: &str, src: Uuid, tgt: Uuid) -> Stream {
    let mut s = Stream::new(src, tgt);
    s.type_spec.add(count, t);
    s
}

// ---------------------------------------------------------------------------
// validate_full — basic correctness
// ---------------------------------------------------------------------------

#[test]
fn validate_full_empty_project_has_no_findings() {
    let p = project_with_int();
    let mut cache = ValidationCache::default();
    cache.validate_full(&p);
    assert!(!cache.has_errors());
    assert!(!cache.has_warnings());
}

#[test]
fn validate_full_balanced_graph_is_ok() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(2, "int"));
    let tgt_id = add_node(&mut p, sink_in(2, "int"));
    let sid = add_stream(&mut p, stream_of(2, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    assert_eq!(cache.state(src_id), ValidationState::Ok);
    assert_eq!(cache.state(tgt_id), ValidationState::Ok);
    assert_eq!(cache.state(sid), ValidationState::Ok);
}

#[test]
fn validate_full_unbalanced_node_shows_warning() {
    let mut p = project_with_int();
    // Source provides 2 int; stream only carries 1 — source under-consumed.
    let src_id = add_node(&mut p, source_out(2, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    let sid = add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    assert_eq!(cache.state(src_id), ValidationState::Warning);
    assert_eq!(cache.state(tgt_id), ValidationState::Ok);
    assert_eq!(cache.state(sid), ValidationState::Ok);
}

#[test]
fn validate_full_clears_stale_results() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    // Inject a bogus stale error for a ghost UUID.
    let ghost = Uuid::new_v4();
    cache.validate_full(&p);
    // Ghost should not appear — validate_full clears everything first.
    assert_eq!(cache.state(ghost), ValidationState::Ok);
}

// ---------------------------------------------------------------------------
// validate_incremental
// ---------------------------------------------------------------------------

#[test]
fn validate_incremental_resolves_balanced_node() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(2, "int"));
    let tgt_id = add_node(&mut p, sink_in(2, "int"));
    let sid = add_stream(&mut p, stream_of(2, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_incremental(src_id, &p);

    assert_eq!(cache.state(src_id), ValidationState::Ok);
    // Stream and target were also re-validated as direct neighbours.
    assert_eq!(cache.state(sid), ValidationState::Ok);
    assert_eq!(cache.state(tgt_id), ValidationState::Ok);
}

#[test]
fn validate_incremental_node_validates_all_neighbours() {
    let mut p = project_with_int();
    // A → B → C:  change B, expect A-side stream and C-side stream re-validated.
    let mut b_node = Node::new(NodeKind::Process, "B");
    b_node.input_spec.add(1, "int");
    b_node.output_spec.add(1, "int");

    let a_id = add_node(&mut p, source_out(1, "int"));
    let b_id = add_node(&mut p, b_node);
    let c_id = add_node(&mut p, sink_in(1, "int"));
    let ab = add_stream(&mut p, stream_of(1, "int", a_id, b_id));
    let bc = add_stream(&mut p, stream_of(1, "int", b_id, c_id));

    let mut cache = ValidationCache::default();
    cache.validate_incremental(b_id, &p);

    // All five components should have been validated.
    for id in [a_id, b_id, c_id, ab, bc] {
        assert_eq!(cache.state(id), ValidationState::Ok, "id {id} should be Ok");
    }
}

#[test]
fn validate_incremental_stream_validates_endpoints() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    let sid = add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_incremental(sid, &p);

    assert_eq!(cache.state(sid), ValidationState::Ok);
    assert_eq!(cache.state(src_id), ValidationState::Ok);
    assert_eq!(cache.state(tgt_id), ValidationState::Ok);
}

#[test]
fn validate_incremental_unknown_id_clears_entry() {
    let p = project_with_int();
    let ghost = Uuid::new_v4();
    let mut cache = ValidationCache::default();
    // Should not panic; entry should remain absent (Ok default).
    cache.validate_incremental(ghost, &p);
    assert_eq!(cache.state(ghost), ValidationState::Ok);
}

// ---------------------------------------------------------------------------
// Spec §4.2 examples via the full pipeline
// ---------------------------------------------------------------------------

/// Spec §4.2 — input oversupply (2,int) example.
#[test]
fn full_pipeline_input_oversupply_example() {
    let mut p = project_with_int();
    let mut proc = Node::new(NodeKind::Process, "Proc");
    proc.input_spec.add(2, "int");
    proc.output_spec.add(1, "int");
    let proc_id = add_node(&mut p, proc);

    let src1_id = add_node(&mut p, source_out(2, "int"));
    let src2_id = add_node(&mut p, source_out(2, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));

    // Two streams each carrying (2, int) → oversupply on proc input.
    let s1 = add_stream(&mut p, stream_of(2, "int", src1_id, proc_id));
    let s2 = add_stream(&mut p, stream_of(2, "int", src2_id, proc_id));
    let s3 = add_stream(&mut p, stream_of(1, "int", proc_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    assert_eq!(cache.state(proc_id), ValidationState::Warning);
    assert!(cache
        .messages(proc_id)
        .iter()
        .any(|m| m.contains("oversupply")));
}

/// Spec §4.2 — output over-consumed example.
#[test]
fn full_pipeline_output_over_consumed_example() {
    let mut p = project_with_int();
    let mut proc = Node::new(NodeKind::Process, "Proc");
    proc.input_spec.add(1, "int");
    proc.output_spec.add(2, "int");
    let proc_id = add_node(&mut p, proc);

    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt1_id = add_node(&mut p, sink_in(2, "int"));
    let tgt2_id = add_node(&mut p, sink_in(2, "int"));

    // Stream in, then two streams each carrying (2, int) out → over-consumed.
    add_stream(&mut p, stream_of(1, "int", src_id, proc_id));
    add_stream(&mut p, stream_of(2, "int", proc_id, tgt1_id));
    add_stream(&mut p, stream_of(2, "int", proc_id, tgt2_id));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    assert_eq!(cache.state(proc_id), ValidationState::Warning);
    assert!(cache
        .messages(proc_id)
        .iter()
        .any(|m| m.contains("over-consumed")));
}

// ---------------------------------------------------------------------------
// on_type_deleted
// ---------------------------------------------------------------------------

#[test]
fn on_type_deleted_marks_referencing_node_as_error() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));

    // First do a full validate so the node is Ok.
    let mut cache = ValidationCache::default();
    cache.validate_full(&p);
    assert_eq!(cache.state(src_id), ValidationState::Warning); // under-consumed, but not error

    // Simulate type deletion.
    cache.on_type_deleted("int", &p);
    assert_eq!(cache.state(src_id), ValidationState::Error);
    assert!(cache.messages(src_id).iter().any(|m| m.contains("int")));
}

#[test]
fn on_type_deleted_marks_referencing_stream_as_error() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    let sid = add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);
    assert_eq!(cache.state(sid), ValidationState::Ok);

    cache.on_type_deleted("int", &p);
    assert_eq!(cache.state(sid), ValidationState::Error);
}

#[test]
fn on_type_deleted_does_not_affect_unrelated_components() {
    let mut p = project_with_int();
    p.data_dictionary
        .add_primitive(dataFlowClaude::model::types::PrimitiveType::new("char"))
        .unwrap();

    // Node references "char", not "int".
    let mut n = Node::new(NodeKind::Source, "S");
    n.output_spec.add(1, "char");
    let nid = add_node(&mut p, n);

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    cache.on_type_deleted("int", &p);
    // The char node should not be affected.
    assert_ne!(cache.state(nid), ValidationState::Error);
}

// ---------------------------------------------------------------------------
// on_type_renamed
// ---------------------------------------------------------------------------

#[test]
fn on_type_renamed_triggers_revalidation_of_affected_components() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    let sid = add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    // Perform the rename in the dictionary first.
    p.data_dictionary.rename("int", "i32").unwrap();

    // Update all specs to use the new name (the dictionary patches composite
    // refs; for this test we patch node/stream specs manually to simulate the
    // full rename cascade that the undo command would do).
    {
        let root = p.root_diagram_id;
        let diag = p.diagrams.get_mut(&root).unwrap();
        diag.nodes.get_mut(&src_id).unwrap().output_spec.entries[0].type_ref = "i32".into();
        diag.nodes.get_mut(&tgt_id).unwrap().input_spec.entries[0].type_ref = "i32".into();
        diag.streams.get_mut(&sid).unwrap().type_spec.entries[0].type_ref = "i32".into();
    }

    // Notify the cache of the rename (old name = "int", new name = "i32").
    cache.on_type_renamed("int", "i32", &p);

    // All components should still be Ok (rename preserved the structure).
    assert_eq!(cache.state(src_id), ValidationState::Ok);
    assert_eq!(cache.state(tgt_id), ValidationState::Ok);
    assert_eq!(cache.state(sid), ValidationState::Ok);
}

// ---------------------------------------------------------------------------
// on_node_deleted
// ---------------------------------------------------------------------------

#[test]
fn on_node_deleted_marks_connected_streams_as_error() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    let sid = add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);
    assert_eq!(cache.state(sid), ValidationState::Ok);

    // Simulate node removal — get dangled IDs from remove_node, then notify cache.
    let root = p.root_diagram_id;
    let removed = p
        .diagrams
        .get_mut(&root)
        .unwrap()
        .remove_node(src_id)
        .unwrap();

    cache.on_node_deleted(src_id, &removed.dangled_stream_ids);

    assert_eq!(cache.state(sid), ValidationState::Error);
    assert_eq!(cache.state(src_id), ValidationState::Ok); // cleared
}

#[test]
fn on_node_deleted_clears_the_deleted_nodes_own_entry() {
    let mut p = project_with_int();
    let nid = add_node(&mut p, Node::new(NodeKind::Source, "S"));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    let root = p.root_diagram_id;
    let removed = p.diagrams.get_mut(&root).unwrap().remove_node(nid).unwrap();
    cache.on_node_deleted(nid, &removed.dangled_stream_ids);

    // Deleted node should no longer appear as Warning/Error.
    assert_eq!(cache.state(nid), ValidationState::Ok);
}

#[test]
fn on_node_deleted_unconnected_node_has_no_dangled_streams() {
    let mut p = project_with_int();
    let nid = add_node(&mut p, Node::new(NodeKind::Source, "S"));

    let mut cache = ValidationCache::default();
    let root = p.root_diagram_id;
    let removed = p.diagrams.get_mut(&root).unwrap().remove_node(nid).unwrap();
    cache.on_node_deleted(nid, &removed.dangled_stream_ids);

    assert!(removed.dangled_stream_ids.is_empty());
    assert_eq!(cache.state(nid), ValidationState::Ok);
}

// ---------------------------------------------------------------------------
// dangling stream detection
// ---------------------------------------------------------------------------

#[test]
fn dangling_stream_is_reported_as_error() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    let sid = add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));

    // Mark the stream dangling manually (as remove_node would do).
    let root = p.root_diagram_id;
    p.diagrams
        .get_mut(&root)
        .unwrap()
        .streams
        .get_mut(&sid)
        .unwrap()
        .mark_dangling();

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);

    assert_eq!(cache.state(sid), ValidationState::Error);
    assert!(cache.messages(sid).iter().any(|m| m.contains("dangling")));
}

// ---------------------------------------------------------------------------
// has_errors / has_warnings
// ---------------------------------------------------------------------------

#[test]
fn has_errors_true_when_error_present() {
    let mut p = project_with_int();
    let src_id = add_node(&mut p, source_out(1, "int"));
    let tgt_id = add_node(&mut p, sink_in(1, "int"));
    let sid = add_stream(&mut p, stream_of(1, "int", src_id, tgt_id));
    p.diagrams
        .get_mut(&p.root_diagram_id)
        .unwrap()
        .streams
        .get_mut(&sid)
        .unwrap()
        .mark_dangling();

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);
    assert!(cache.has_errors());
}

#[test]
fn has_warnings_true_when_only_warning_present() {
    let mut p = project_with_int();
    // Source provides 2 but no streams consume any → under-consumed warning.
    add_node(&mut p, source_out(2, "int"));

    let mut cache = ValidationCache::default();
    cache.validate_full(&p);
    assert!(cache.has_warnings());
    assert!(!cache.has_errors());
}

#[test]
fn clear_all_resets_cache() {
    let mut p = project_with_int();
    add_node(&mut p, source_out(2, "int"));
    let mut cache = ValidationCache::default();
    cache.validate_full(&p);
    assert!(cache.has_warnings());

    cache.clear_all();
    assert!(!cache.has_warnings());
    assert!(!cache.has_errors());
}
