//! Round-trip and error-handling tests for XML persistence (spec §7).

use uuid::Uuid;

use dataFlowClaude::{
    model::{
        node::{Node, NodeKind},
        project::Project,
        stream::Stream,
        types::{CompositeType, PrimitiveType, TypeSpec},
    },
    persistence::xml::{load_project, save_project, PersistenceError},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base_project() -> Project {
    let mut p = Project::new();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("uint32_t"))
        .unwrap();
    p.data_dictionary
        .add_primitive(PrimitiveType::new("char"))
        .unwrap();
    p
}

fn spec(pairs: &[(u32, &str)]) -> TypeSpec {
    TypeSpec::from_pairs(pairs.iter().copied().map(|(c, n)| (c, n)))
}

fn add_node(p: &mut Project, node: Node, pos: (f32, f32)) -> Uuid {
    let id = node.id;
    let rid = p.root_diagram_id;
    p.diagrams.get_mut(&rid).unwrap().add_node(node, pos);
    id
}

fn add_stream(p: &mut Project, stream: Stream) -> Uuid {
    let id = stream.id;
    let rid = p.root_diagram_id;
    p.diagrams
        .get_mut(&rid)
        .unwrap()
        .add_stream(stream)
        .unwrap();
    id
}

// ---------------------------------------------------------------------------
// schema_version attribute
// ---------------------------------------------------------------------------

#[test]
fn schema_version_attribute_is_present() {
    let p = Project::new();
    let xml = save_project(&p).unwrap();
    assert!(
        xml.contains("schema_version=\"1\""),
        "schema_version must be in output:\n{xml}"
    );
}

#[test]
fn root_diagram_id_attribute_is_present() {
    let p = Project::new();
    let xml = save_project(&p).unwrap();
    let rid = p.root_diagram_id.hyphenated().to_string();
    assert!(
        xml.contains(&format!("root_diagram_id=\"{rid}\"")),
        "root_diagram_id must be in output"
    );
}

// ---------------------------------------------------------------------------
// Round-trip: empty project
// ---------------------------------------------------------------------------

#[test]
fn round_trip_empty_project() {
    let p = Project::new();
    let xml = save_project(&p).unwrap();
    let p2 = load_project(&xml).unwrap();

    assert_eq!(p2.schema_version, 1);
    assert_eq!(p2.root_diagram_id, p.root_diagram_id);
    assert!(p2.data_dictionary.primitives.is_empty());
    assert!(p2.data_dictionary.composites.is_empty());
    assert_eq!(p2.diagrams.len(), 1);
    assert!(p2.orphaned_diagram_ids.is_empty());
}

// ---------------------------------------------------------------------------
// Round-trip: data dictionary
// ---------------------------------------------------------------------------

#[test]
fn round_trip_primitives() {
    let p = base_project();
    let xml = save_project(&p).unwrap();
    let p2 = load_project(&xml).unwrap();

    let names: Vec<&str> = p2
        .data_dictionary
        .primitives
        .iter()
        .map(|p| p.name())
        .collect();
    assert!(names.contains(&"uint32_t"));
    assert!(names.contains(&"char"));
}

#[test]
fn round_trip_composite_type() {
    let mut p = base_project();
    p.data_dictionary
        .add_composite(CompositeType::new(
            "Pair",
            spec(&[(2, "uint32_t"), (1, "char")]),
        ))
        .unwrap();

    let xml = save_project(&p).unwrap();
    let p2 = load_project(&xml).unwrap();

    let comp = p2
        .data_dictionary
        .composite("Pair")
        .expect("Pair must survive round-trip");
    assert_eq!(comp.spec.entries.len(), 2);
    let counts: Vec<(u32, &str)> = comp
        .spec
        .entries
        .iter()
        .map(|e| (e.count, e.type_ref.as_str()))
        .collect();
    assert!(counts.contains(&(2, "uint32_t")));
    assert!(counts.contains(&(1, "char")));
}

#[test]
fn round_trip_nested_composite() {
    let mut p = base_project();
    p.data_dictionary
        .add_composite(CompositeType::new("Inner", spec(&[(4, "uint32_t")])))
        .unwrap();
    p.data_dictionary
        .add_composite(CompositeType::new("Outer", spec(&[(2, "Inner")])))
        .unwrap();

    let xml = save_project(&p).unwrap();
    let p2 = load_project(&xml).unwrap();

    assert!(p2.data_dictionary.composite("Inner").is_some());
    let outer = p2.data_dictionary.composite("Outer").unwrap();
    assert_eq!(outer.spec.entries[0].type_ref, "Inner");
    assert_eq!(outer.spec.entries[0].count, 2);
}

// ---------------------------------------------------------------------------
// Round-trip: nodes
// ---------------------------------------------------------------------------

#[test]
fn round_trip_source_node() {
    let mut p = base_project();
    let mut src = Node::new(NodeKind::Source, "MySrc");
    src.output_spec = spec(&[(3, "uint32_t")]);
    let nid = add_node(&mut p, src, (10.0, 20.0));

    let xml = save_project(&p).unwrap();
    let p2 = load_project(&xml).unwrap();

    let rid = p2.root_diagram_id;
    let node = p2
        .diagram(rid)
        .unwrap()
        .nodes
        .get(&nid)
        .expect("node must survive");
    assert_eq!(node.kind, NodeKind::Source);
    assert_eq!(node.name, "MySrc");
    assert!(node.input_spec.is_empty());
    assert_eq!(node.output_spec.entries[0].count, 3);
    assert_eq!(node.output_spec.entries[0].type_ref, "uint32_t");
    let pos = p2
        .diagram(rid)
        .unwrap()
        .positions
        .get(&nid)
        .copied()
        .unwrap();
    assert!((pos.0 - 10.0).abs() < 0.01);
    assert!((pos.1 - 20.0).abs() < 0.01);
}

#[test]
fn round_trip_sink_node() {
    let mut p = base_project();
    let mut tgt = Node::new(NodeKind::Sink, "MySink");
    tgt.input_spec = spec(&[(1, "char")]);
    let nid = add_node(&mut p, tgt, (100.0, 200.0));

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    let node = p2
        .diagram(p2.root_diagram_id)
        .unwrap()
        .nodes
        .get(&nid)
        .unwrap();
    assert_eq!(node.kind, NodeKind::Sink);
    assert!(node.output_spec.is_empty());
    assert_eq!(node.input_spec.entries[0].type_ref, "char");
}

#[test]
fn round_trip_process_node_with_both_specs() {
    let mut p = base_project();
    let mut proc = Node::new(NodeKind::Process, "Transform");
    proc.input_spec = spec(&[(2, "uint32_t")]);
    proc.output_spec = spec(&[(1, "char")]);
    let nid = add_node(&mut p, proc, (50.0, 50.0));

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    let node = p2
        .diagram(p2.root_diagram_id)
        .unwrap()
        .nodes
        .get(&nid)
        .unwrap();
    assert_eq!(node.kind, NodeKind::Process);
    assert_eq!(node.input_spec.entries.len(), 1);
    assert_eq!(node.output_spec.entries.len(), 1);
}

#[test]
fn round_trip_boundary_node_flag() {
    let mut p = base_project();
    let mut b = Node::new_boundary(
        NodeKind::Source,
        "in_0",
        TypeSpec::new(),
        spec(&[(1, "uint32_t")]),
    );
    let nid = b.id;
    add_node(&mut p, b, (40.0, 80.0));

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    let node = p2
        .diagram(p2.root_diagram_id)
        .unwrap()
        .nodes
        .get(&nid)
        .unwrap();
    assert!(node.is_boundary);
}

#[test]
fn round_trip_node_sub_diagram_id() {
    let mut p = base_project();
    let mut proc = Node::new(NodeKind::Process, "P");
    proc.input_spec.add(1, "uint32_t");
    let pid = add_node(&mut p, proc, (0.0, 0.0));
    let rid = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(rid, pid).unwrap();

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    let node = p2
        .diagram(p2.root_diagram_id)
        .unwrap()
        .nodes
        .get(&pid)
        .unwrap();
    assert_eq!(node.sub_diagram_id, Some(sub_id));
}

// ---------------------------------------------------------------------------
// Round-trip: streams
// ---------------------------------------------------------------------------

#[test]
fn round_trip_stream_basic() {
    let mut p = base_project();
    let src_id = add_node(&mut p, Node::new(NodeKind::Source, "S"), (0.0, 0.0));
    let tgt_id = add_node(&mut p, Node::new(NodeKind::Sink, "T"), (100.0, 0.0));
    let mut s = Stream::new(src_id, tgt_id);
    s.type_spec = spec(&[(2, "uint32_t")]);
    let sid = add_stream(&mut p, s);

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    let stream = p2
        .diagram(p2.root_diagram_id)
        .unwrap()
        .streams
        .get(&sid)
        .unwrap();
    assert_eq!(stream.source_node_id, src_id);
    assert_eq!(stream.target_node_id, tgt_id);
    assert_eq!(stream.type_spec.entries[0].count, 2);
    assert!(!stream.is_dangling);
}

#[test]
fn round_trip_dangling_stream() {
    let mut p = base_project();
    let src_id = add_node(&mut p, Node::new(NodeKind::Source, "S"), (0.0, 0.0));
    let tgt_id = add_node(&mut p, Node::new(NodeKind::Sink, "T"), (100.0, 0.0));
    let sid = add_stream(&mut p, Stream::new(src_id, tgt_id));
    // Mark dangling manually.
    let rid = p.root_diagram_id;
    p.diagrams
        .get_mut(&rid)
        .unwrap()
        .streams
        .get_mut(&sid)
        .unwrap()
        .mark_dangling();

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    assert!(p2.diagram(p2.root_diagram_id).unwrap().streams[&sid].is_dangling);
}

#[test]
fn round_trip_stream_empty_type_spec() {
    let mut p = base_project();
    let src_id = add_node(&mut p, Node::new(NodeKind::Source, "S"), (0.0, 0.0));
    let tgt_id = add_node(&mut p, Node::new(NodeKind::Sink, "T"), (100.0, 0.0));
    let sid = add_stream(&mut p, Stream::new(src_id, tgt_id)); // empty spec

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    assert!(p2.diagram(p2.root_diagram_id).unwrap().streams[&sid]
        .type_spec
        .is_empty());
}

// ---------------------------------------------------------------------------
// Round-trip: stream groups
// ---------------------------------------------------------------------------

#[test]
fn round_trip_stream_group() {
    let mut p = base_project();
    let src_id = add_node(&mut p, Node::new(NodeKind::Source, "S"), (0.0, 0.0));
    let t1 = add_node(&mut p, Node::new(NodeKind::Sink, "T1"), (100.0, 0.0));
    let t2 = add_node(&mut p, Node::new(NodeKind::Sink, "T2"), (100.0, 100.0));
    let s1 = add_stream(&mut p, Stream::new(src_id, t1));
    let s2 = add_stream(&mut p, Stream::new(src_id, t2));
    let rid = p.root_diagram_id;
    let gid = p
        .diagrams
        .get_mut(&rid)
        .unwrap()
        .create_group("DataBus", vec![s1, s2])
        .unwrap();

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    let diag = p2.diagram(p2.root_diagram_id).unwrap();
    let group = diag
        .stream_groups
        .get(&gid)
        .expect("group must survive round-trip");
    assert_eq!(group.name, "DataBus");
    assert!(group.contains(s1));
    assert!(group.contains(s2));
    assert_eq!(diag.streams[&s1].group_id, Some(gid));
    assert_eq!(diag.streams[&s2].group_id, Some(gid));
}

// ---------------------------------------------------------------------------
// Round-trip: viewport state
// ---------------------------------------------------------------------------

#[test]
fn round_trip_viewport() {
    let mut p = Project::new();
    let rid = p.root_diagram_id;
    p.diagrams.get_mut(&rid).unwrap().viewport.zoom = 2.5;
    p.diagrams.get_mut(&rid).unwrap().viewport.scroll = (120.0, -45.0);

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    let vp = &p2.diagram(p2.root_diagram_id).unwrap().viewport;
    assert!((vp.zoom - 2.5).abs() < 0.001);
    assert!((vp.scroll.0 - 120.0).abs() < 0.001);
    assert!((vp.scroll.1 - (-45.0)).abs() < 0.001);
}

// ---------------------------------------------------------------------------
// Round-trip: sub-diagrams and orphaned
// ---------------------------------------------------------------------------

#[test]
fn round_trip_sub_diagram() {
    let mut p = base_project();
    let mut proc = Node::new(NodeKind::Process, "P");
    proc.input_spec.add(1, "uint32_t");
    let pid = add_node(&mut p, proc, (0.0, 0.0));
    let rid = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(rid, pid).unwrap();

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    assert!(p2.diagrams.contains_key(&sub_id));
    let sub = p2.diagram(sub_id).unwrap();
    assert_eq!(sub.parent_process_id, Some(pid));
    // Boundary source should be there.
    assert!(sub
        .nodes
        .values()
        .any(|n| n.is_boundary && n.kind == NodeKind::Source));
}

#[test]
fn round_trip_orphaned_diagram_ids() {
    let mut p = base_project();
    let proc = Node::new(NodeKind::Process, "P");
    let pid = add_node(&mut p, proc, (0.0, 0.0));
    let rid = p.root_diagram_id;
    let sub_id = p.create_sub_diagram(rid, pid).unwrap();
    p.orphan_sub_diagram(sub_id).unwrap();

    let p2 = load_project(&save_project(&p).unwrap()).unwrap();
    assert!(p2.orphaned_diagram_ids.contains(&sub_id));
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[test]
fn missing_schema_version_is_error_not_panic() {
    let xml = r#"<?xml version="1.0"?>
        <project root_diagram_id="00000000-0000-0000-0000-000000000001">
          <dictionary/>
          <diagrams/>
          <orphaned/>
        </project>"#;
    let result = load_project(xml);
    assert!(result.is_err());
    assert!(matches!(result, Err(PersistenceError::Malformed(_))));
}

#[test]
fn wrong_schema_version_returns_unsupported_version_error() {
    let xml = r#"<?xml version="1.0"?>
        <project schema_version="99"
                 root_diagram_id="00000000-0000-0000-0000-000000000001">
          <dictionary/>
          <diagrams/>
          <orphaned/>
        </project>"#;
    let result = load_project(xml);
    assert!(matches!(
        result,
        Err(PersistenceError::UnsupportedVersion {
            found: 99,
            expected: 1
        })
    ));
}

#[test]
fn invalid_uuid_returns_error_not_panic() {
    let xml = r#"<?xml version="1.0"?>
        <project schema_version="1" root_diagram_id="not-a-uuid">
          <dictionary/>
          <diagrams/>
          <orphaned/>
        </project>"#;
    let result = load_project(xml);
    assert!(result.is_err());
    assert!(matches!(result, Err(PersistenceError::InvalidUuid { .. })));
}

#[test]
fn missing_root_diagram_id_is_error_not_panic() {
    let xml = r#"<?xml version="1.0"?>
        <project schema_version="1">
          <dictionary/>
          <diagrams/>
          <orphaned/>
        </project>"#;
    let result = load_project(xml);
    assert!(matches!(result, Err(PersistenceError::Malformed(_))));
}

#[test]
fn unknown_node_kind_returns_malformed_error() {
    let nid = Uuid::new_v4();
    let rid = Uuid::new_v4();
    let xml = format!(
        r#"<?xml version="1.0"?>
        <project schema_version="1" root_diagram_id="{rid}">
          <dictionary/>
          <diagrams>
            <diagram id="{rid}" name="Root">
              <viewport zoom="1.0" scroll_x="0.0" scroll_y="0.0"/>
              <nodes>
                <node id="{nid}" kind="BOGUS" name="Bad" x="0" y="0"/>
              </nodes>
              <streams/>
              <groups/>
            </diagram>
          </diagrams>
          <orphaned/>
        </project>"#
    );
    let result = load_project(&xml);
    assert!(matches!(result, Err(PersistenceError::Malformed(_))));
}

#[test]
fn missing_type_ref_in_spec_is_recoverable_error_not_panic() {
    // A stream with a type_ref that doesn't exist in the dictionary is valid
    // XML — the persistence layer loads it, and the validation layer reports
    // the error.  This test confirms loading doesn't panic.
    let src = Uuid::new_v4();
    let tgt = Uuid::new_v4();
    let sid = Uuid::new_v4();
    let rid = Uuid::new_v4();
    let xml = format!(
        r#"<?xml version="1.0"?>
        <project schema_version="1" root_diagram_id="{rid}">
          <dictionary/>
          <diagrams>
            <diagram id="{rid}" name="Root">
              <viewport zoom="1.0" scroll_x="0.0" scroll_y="0.0"/>
              <nodes>
                <node id="{src}" kind="Source" name="S" x="0" y="0"/>
                <node id="{tgt}" kind="Sink"   name="T" x="100" y="0"/>
              </nodes>
              <streams>
                <stream id="{sid}" source_node_id="{src}" target_node_id="{tgt}">
                  <type_spec>
                    <entry count="1" type_ref="MISSING_TYPE"/>
                  </type_spec>
                </stream>
              </streams>
              <groups/>
            </diagram>
          </diagrams>
          <orphaned/>
        </project>"#
    );
    // Must not panic; must return either Ok (loaded with unknown ref) or a
    // recoverable Err — never a panic.
    let result = load_project(&xml);
    match result {
        Ok(p) => {
            // Loaded successfully; the unknown type_ref is present in the stream.
            let stream = p.diagram(rid).unwrap().streams.get(&sid).unwrap();
            assert_eq!(stream.type_spec.entries[0].type_ref, "MISSING_TYPE");
        }
        Err(e) => {
            // A recoverable error is also acceptable; just must not be a panic.
            assert!(!format!("{e}").is_empty());
        }
    }
}

#[test]
fn completely_empty_xml_is_error_not_panic() {
    let result = load_project("");
    assert!(result.is_err());
}

#[test]
fn truncated_xml_is_error_not_panic() {
    let result = load_project(r#"<?xml version="1.0"?><project schema_version="1""#);
    assert!(result.is_err());
}
