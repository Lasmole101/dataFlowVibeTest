//! Tests for `validation::balance` (spec §4.2).
//!
//! Every example from the spec is covered explicitly, labelled with the
//! section reference.  Additional cases cover edge conditions the spec
//! implies but doesn't illustrate.

use dataFlowClaude::{
    model::{
        dictionary::DataDictionary,
        node::{Node, NodeKind},
        stream::Stream,
        types::{CompositeType, PrimitiveType, TypeSpec},
    },
    validation::balance::{
        node_input_balance, node_output_balance, stream_balance, BalanceResult, Severity,
    },
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dict_with(primitives: &[&str]) -> DataDictionary {
    let mut d = DataDictionary::new();
    for &name in primitives {
        d.add_primitive(PrimitiveType::new(name)).unwrap();
    }
    d
}

fn spec(pairs: &[(u32, &str)]) -> TypeSpec {
    TypeSpec::from_pairs(pairs.iter().copied().map(|(c, n)| (c, n)))
}

/// Source node with the given output spec.
fn source_with_output(output: TypeSpec) -> Node {
    let mut n = Node::new(NodeKind::Source, "Src");
    n.output_spec = output;
    n
}

/// Sink node with the given input spec.
fn sink_with_input(input: TypeSpec) -> Node {
    let mut n = Node::new(NodeKind::Sink, "Tgt");
    n.input_spec = input;
    n
}

/// Process node with given input and output specs.
fn process(input: TypeSpec, output: TypeSpec) -> Node {
    let mut n = Node::new(NodeKind::Process, "Proc");
    n.input_spec = input;
    n.output_spec = output;
    n
}

/// Stream carrying the given type spec between placeholder node IDs.
fn stream_with(type_spec: TypeSpec) -> Stream {
    let mut s = Stream::new(Uuid::new_v4(), Uuid::new_v4());
    s.type_spec = type_spec;
    s
}

// ---------------------------------------------------------------------------
// BalanceResult helpers
// ---------------------------------------------------------------------------

fn is_warning(r: &BalanceResult) -> bool {
    matches!(r, BalanceResult::Warning(_))
}

fn is_error(r: &BalanceResult) -> bool {
    matches!(r, BalanceResult::Error(_))
}

fn message_contains(r: &BalanceResult, needle: &str) -> bool {
    r.messages().iter().any(|m| m.contains(needle))
}

// ---------------------------------------------------------------------------
// BalanceResult::merge
// ---------------------------------------------------------------------------

#[test]
fn merge_ok_ok_is_ok() {
    assert_eq!(
        BalanceResult::Ok.merge(BalanceResult::Ok),
        BalanceResult::Ok
    );
}

#[test]
fn merge_ok_warning_is_warning() {
    let w = BalanceResult::Warning(vec!["w".into()]);
    assert!(is_warning(&BalanceResult::Ok.merge(w)));
}

#[test]
fn merge_warning_error_is_error() {
    let w = BalanceResult::Warning(vec!["w".into()]);
    let e = BalanceResult::Error(vec!["e".into()]);
    assert!(is_error(&w.merge(e)));
}

#[test]
fn merge_combines_messages() {
    let w1 = BalanceResult::Warning(vec!["w1".into()]);
    let w2 = BalanceResult::Warning(vec!["w2".into()]);
    let merged = w1.merge(w2);
    assert_eq!(merged.messages().len(), 2);
}

#[test]
fn severity_ok_is_none() {
    assert_eq!(BalanceResult::Ok.severity(), None);
}

#[test]
fn severity_warning_is_warning() {
    assert_eq!(
        BalanceResult::Warning(vec![]).severity(),
        Some(Severity::Warning)
    );
}

#[test]
fn severity_error_is_error() {
    assert_eq!(
        BalanceResult::Error(vec![]).severity(),
        Some(Severity::Error)
    );
}

// ---------------------------------------------------------------------------
// stream_balance — spec §4.2 (stream balance)
// ---------------------------------------------------------------------------

/// Perfectly matched stream: source outputs exactly what stream carries,
/// target accepts exactly what stream carries.
#[test]
fn stream_balance_exact_match_is_ok() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(2, "int")]));
    let tgt = sink_with_input(spec(&[(2, "int")]));
    let s = stream_with(spec(&[(2, "int")]));
    assert_eq!(stream_balance(&s, &src, &tgt, &dict), BalanceResult::Ok);
}

/// Stream carries less than source provides and less than target accepts —
/// still ok at stream level (fan-out/node-balance will catch the rest).
#[test]
fn stream_balance_stream_subset_of_node_specs_is_ok() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(4, "int")])); // provides 4
    let tgt = sink_with_input(spec(&[(4, "int")])); // accepts  4
    let s = stream_with(spec(&[(2, "int")])); // carries  2
    assert_eq!(stream_balance(&s, &src, &tgt, &dict), BalanceResult::Ok);
}

/// Stream over-draws from source: stream needs 3 int but source only provides 2.
#[test]
fn stream_balance_over_draws_from_source_warns() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(2, "int")])); // provides 2
    let tgt = sink_with_input(spec(&[(3, "int")])); // accepts  3
    let s = stream_with(spec(&[(3, "int")])); // carries  3
    let result = stream_balance(&s, &src, &tgt, &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "source"));
}

/// Stream over-delivers to target: stream carries 3 int but target accepts only 2.
#[test]
fn stream_balance_over_delivers_to_target_warns() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(3, "int")])); // provides 3
    let tgt = sink_with_input(spec(&[(2, "int")])); // accepts  2
    let s = stream_with(spec(&[(3, "int")])); // carries  3
    let result = stream_balance(&s, &src, &tgt, &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "target"));
}

/// Stream with unknown type in its own spec → error.
#[test]
fn stream_balance_unknown_stream_type_is_error() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(1, "int")]));
    let tgt = sink_with_input(spec(&[(1, "int")]));
    let s = stream_with(spec(&[(1, "GHOST")]));
    assert!(is_error(&stream_balance(&s, &src, &tgt, &dict)));
}

/// Stream with unknown type in source output spec → error.
#[test]
fn stream_balance_unknown_source_type_is_error() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(1, "GHOST")]));
    let tgt = sink_with_input(spec(&[(1, "int")]));
    let s = stream_with(spec(&[(1, "int")]));
    assert!(is_error(&stream_balance(&s, &src, &tgt, &dict)));
}

/// Empty stream spec (nothing is carried) — always ok.
#[test]
fn stream_balance_empty_stream_spec_is_ok() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(2, "int")]));
    let tgt = sink_with_input(spec(&[(2, "int")]));
    let s = stream_with(TypeSpec::new());
    assert_eq!(stream_balance(&s, &src, &tgt, &dict), BalanceResult::Ok);
}

/// Source node kind is Source (no inputs); target is Sink (no outputs).
/// Direction flags don't affect the balance check — only specs matter.
#[test]
fn stream_balance_source_to_sink_node_kinds() {
    let dict = dict_with(&["int"]);
    let src = source_with_output(spec(&[(1, "int")]));
    let tgt = sink_with_input(spec(&[(1, "int")]));
    let s = stream_with(spec(&[(1, "int")]));
    assert_eq!(stream_balance(&s, &src, &tgt, &dict), BalanceResult::Ok);
}

/// Multi-primitive stream — mismatch on one primitive only.
#[test]
fn stream_balance_multi_primitive_partial_mismatch() {
    let dict = dict_with(&["int", "char"]);
    // Source provides enough int but not enough char.
    let src = source_with_output(spec(&[(2, "int"), (1, "char")]));
    let tgt = sink_with_input(spec(&[(2, "int"), (2, "char")]));
    let s = stream_with(spec(&[(2, "int"), (2, "char")]));
    let result = stream_balance(&s, &src, &tgt, &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "char"));
}

// ---------------------------------------------------------------------------
// node_input_balance — spec §4.2 (node input balance)
// ---------------------------------------------------------------------------

/// Spec §4.2 exact balanced example:
/// Node input requires: (2, int)
/// Stream 1 carries:    (1, int)
/// Stream 2 carries:    (1, int)
/// → Balanced ✓
#[test]
fn node_input_balance_spec_example_balanced() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(2, "int")]), TypeSpec::new());
    let s1 = stream_with(spec(&[(1, "int")]));
    let s2 = stream_with(spec(&[(1, "int")]));
    assert_eq!(
        node_input_balance(&node, &[&s1, &s2], &dict),
        BalanceResult::Ok
    );
}

/// Spec §4.2 oversupplied example:
/// Node input requires: (2, int)
/// Stream 1 carries:    (2, int)
/// Stream 2 carries:    (2, int)
/// → Oversupplied ✗ (warning)
#[test]
fn node_input_balance_spec_example_oversupplied() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(2, "int")]), TypeSpec::new());
    let s1 = stream_with(spec(&[(2, "int")]));
    let s2 = stream_with(spec(&[(2, "int")]));
    let result = node_input_balance(&node, &[&s1, &s2], &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "oversupply"));
}

/// Undersupply: node needs (2, int) but only (1, int) arrives.
#[test]
fn node_input_balance_undersupply_warns() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(2, "int")]), TypeSpec::new());
    let s1 = stream_with(spec(&[(1, "int")]));
    let result = node_input_balance(&node, &[&s1], &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "undersupply"));
}

/// No incoming streams, non-empty input spec → undersupply warning.
#[test]
fn node_input_balance_no_streams_with_spec_warns() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(2, "int")]), TypeSpec::new());
    let result = node_input_balance(&node, &[], &dict);
    assert!(is_warning(&result));
}

/// No incoming streams, empty input spec (Source node) → ok.
#[test]
fn node_input_balance_source_node_no_streams_is_ok() {
    let dict = dict_with(&["int"]);
    let node = Node::new(NodeKind::Source, "Src");
    let result = node_input_balance(&node, &[], &dict);
    assert_eq!(result, BalanceResult::Ok);
}

/// Exact match with a single stream.
#[test]
fn node_input_balance_single_stream_exact_match() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(3, "int")]), TypeSpec::new());
    let s = stream_with(spec(&[(3, "int")]));
    assert_eq!(node_input_balance(&node, &[&s], &dict), BalanceResult::Ok);
}

/// Multiple primitives — one balanced, one oversupplied.
#[test]
fn node_input_balance_partial_oversupply() {
    let dict = dict_with(&["int", "char"]);
    let node = process(spec(&[(2, "int"), (1, "char")]), TypeSpec::new());
    let s1 = stream_with(spec(&[(2, "int")]));
    let s2 = stream_with(spec(&[(3, "char")])); // oversupplies char
    let result = node_input_balance(&node, &[&s1, &s2], &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "char"));
    // int should be balanced — message must not mention int oversupply.
    assert!(!result.messages().iter().any(|m| m.contains("int")));
}

/// Unknown type in node input spec → error.
#[test]
fn node_input_balance_unknown_node_type_is_error() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(1, "GHOST")]), TypeSpec::new());
    let s = stream_with(spec(&[(1, "int")]));
    assert!(is_error(&node_input_balance(&node, &[&s], &dict)));
}

/// Unknown type in incoming stream spec → error.
#[test]
fn node_input_balance_unknown_stream_type_is_error() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(1, "int")]), TypeSpec::new());
    let s = stream_with(spec(&[(1, "GHOST")]));
    assert!(is_error(&node_input_balance(&node, &[&s], &dict)));
}

/// Three incoming streams that together balance exactly.
#[test]
fn node_input_balance_three_streams_balance_exactly() {
    let dict = dict_with(&["int"]);
    let node = process(spec(&[(6, "int")]), TypeSpec::new());
    let s1 = stream_with(spec(&[(1, "int")]));
    let s2 = stream_with(spec(&[(2, "int")]));
    let s3 = stream_with(spec(&[(3, "int")]));
    assert_eq!(
        node_input_balance(&node, &[&s1, &s2, &s3], &dict),
        BalanceResult::Ok
    );
}

// ---------------------------------------------------------------------------
// node_output_balance — spec §4.2 (node output balance)
// ---------------------------------------------------------------------------

/// Spec §4.2 exact balanced output example:
/// Node output provides: (2, int)
/// Stream 1 carries:     (1, int)
/// Stream 2 carries:     (1, int)
/// → Balanced ✓
#[test]
fn node_output_balance_spec_example_balanced() {
    let dict = dict_with(&["int"]);
    let node = process(TypeSpec::new(), spec(&[(2, "int")]));
    let s1 = stream_with(spec(&[(1, "int")]));
    let s2 = stream_with(spec(&[(1, "int")]));
    assert_eq!(
        node_output_balance(&node, &[&s1, &s2], &dict),
        BalanceResult::Ok
    );
}

/// Spec §4.2 over-consumed output example:
/// Node output provides: (2, int)
/// Stream 1 carries:     (2, int)
/// Stream 2 carries:     (2, int)
/// → Over-consumed ✗ (warning)
#[test]
fn node_output_balance_spec_example_over_consumed() {
    let dict = dict_with(&["int"]);
    let node = process(TypeSpec::new(), spec(&[(2, "int")]));
    let s1 = stream_with(spec(&[(2, "int")]));
    let s2 = stream_with(spec(&[(2, "int")]));
    let result = node_output_balance(&node, &[&s1, &s2], &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "over-consumed"));
}

/// Under-consumption: node provides (2, int) but only (1, int) consumed.
#[test]
fn node_output_balance_under_consumed_warns() {
    let dict = dict_with(&["int"]);
    let node = process(TypeSpec::new(), spec(&[(2, "int")]));
    let s1 = stream_with(spec(&[(1, "int")]));
    let result = node_output_balance(&node, &[&s1], &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "under-consumed"));
}

/// No outgoing streams, non-empty output spec → under-consumption warning.
#[test]
fn node_output_balance_no_streams_with_spec_warns() {
    let dict = dict_with(&["int"]);
    let node = process(TypeSpec::new(), spec(&[(2, "int")]));
    let result = node_output_balance(&node, &[], &dict);
    assert!(is_warning(&result));
}

/// No outgoing streams, empty output spec (Sink node) → ok.
#[test]
fn node_output_balance_sink_node_no_streams_is_ok() {
    let dict = dict_with(&["int"]);
    let node = Node::new(NodeKind::Sink, "Snk");
    let result = node_output_balance(&node, &[], &dict);
    assert_eq!(result, BalanceResult::Ok);
}

/// Single outgoing stream — exact match.
#[test]
fn node_output_balance_single_stream_exact_match() {
    let dict = dict_with(&["int"]);
    let node = process(TypeSpec::new(), spec(&[(5, "int")]));
    let s = stream_with(spec(&[(5, "int")]));
    assert_eq!(node_output_balance(&node, &[&s], &dict), BalanceResult::Ok);
}

/// Multi-primitive: one primitive over-consumed, one balanced.
#[test]
fn node_output_balance_partial_over_consumption() {
    let dict = dict_with(&["int", "char"]);
    let node = process(TypeSpec::new(), spec(&[(2, "int"), (2, "char")]));
    let s1 = stream_with(spec(&[(2, "int")]));
    let s2 = stream_with(spec(&[(3, "char")])); // over-consumes char
    let result = node_output_balance(&node, &[&s1, &s2], &dict);
    assert!(is_warning(&result));
    assert!(message_contains(&result, "char"));
    assert!(!result.messages().iter().any(|m| m.contains("int")));
}

/// Unknown type in node output spec → error.
#[test]
fn node_output_balance_unknown_node_type_is_error() {
    let dict = dict_with(&["int"]);
    let node = process(TypeSpec::new(), spec(&[(1, "GHOST")]));
    let s = stream_with(spec(&[(1, "int")]));
    assert!(is_error(&node_output_balance(&node, &[&s], &dict)));
}

/// Unknown type in outgoing stream spec → error.
#[test]
fn node_output_balance_unknown_stream_type_is_error() {
    let dict = dict_with(&["int"]);
    let node = process(TypeSpec::new(), spec(&[(1, "int")]));
    let s = stream_with(spec(&[(1, "GHOST")]));
    assert!(is_error(&node_output_balance(&node, &[&s], &dict)));
}

// ---------------------------------------------------------------------------
// Composite type specs in balance checks
// ---------------------------------------------------------------------------

/// Structural equality: composites with different names but same primitive
/// content count as balanced.
#[test]
fn node_input_balance_composite_spec_flattens_correctly() {
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new(
        "Word",
        TypeSpec::from_pairs([(2u32, "u32")]),
    ))
    .unwrap();

    // Node requires 4 u32 (expressed as 2 Words).
    let node = process(spec(&[(2, "Word")]), TypeSpec::new());
    // Stream supplies 4 u32 directly.
    let s = stream_with(spec(&[(4, "u32")]));
    assert_eq!(node_input_balance(&node, &[&s], &dict), BalanceResult::Ok);
}

#[test]
fn stream_balance_composite_vs_primitive_equivalent() {
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new(
        "Pair",
        TypeSpec::from_pairs([(2u32, "u32")]),
    ))
    .unwrap();

    // Source output: (1, Pair) = 2 u32.  Stream carries (2, u32) directly.
    let src = source_with_output(spec(&[(1, "Pair")]));
    let tgt = sink_with_input(spec(&[(2, "u32")]));
    let s = stream_with(spec(&[(2, "u32")]));
    assert_eq!(stream_balance(&s, &src, &tgt, &dict), BalanceResult::Ok);
}

// ---------------------------------------------------------------------------
// Finding::from_balance_result
// ---------------------------------------------------------------------------

use dataFlowClaude::validation::balance::Finding;

#[test]
fn finding_from_ok_is_empty() {
    let findings = Finding::from_balance_result(Uuid::new_v4(), BalanceResult::Ok);
    assert!(findings.is_empty());
}

#[test]
fn finding_from_warning_has_correct_severity() {
    let id = Uuid::new_v4();
    let findings =
        Finding::from_balance_result(id, BalanceResult::Warning(vec!["w1".into(), "w2".into()]));
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.severity == Severity::Warning));
    assert!(findings.iter().all(|f| f.component_id == id));
}

#[test]
fn finding_from_error_has_correct_severity() {
    let id = Uuid::new_v4();
    let findings = Finding::from_balance_result(id, BalanceResult::Error(vec!["e1".into()]));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].severity, Severity::Error);
}
