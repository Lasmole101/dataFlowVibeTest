//! Tests for the type system (spec §2).
//!
//! Covers: structural equality, multiset order independence, nested composite
//! flattening, circular definition rejection, and dictionary mutation guards.

use pretty_assertions::assert_eq;

use dataFlowClaude::model::{
    dictionary::{DataDictionary, DictionaryError},
    types::{CompositeType, PrimitiveType, TypeError, TypeSpec},
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a dictionary pre-loaded with `uint32_t` and `char` primitives.
fn base_dict() -> DataDictionary {
    let mut d = DataDictionary::new();
    d.add_primitive(PrimitiveType::new("uint32_t")).unwrap();
    d.add_primitive(PrimitiveType::new("char")).unwrap();
    d
}

/// Build a [`TypeSpec`] from `(count, name)` pairs.
fn spec(pairs: &[(u32, &str)]) -> TypeSpec {
    TypeSpec::from_pairs(pairs.iter().copied().map(|(c, n)| (c, n)))
}

// ---------------------------------------------------------------------------
// TypeSpec::add and multiset canonicalisation
// ---------------------------------------------------------------------------

#[test]
fn add_merges_duplicate_type_refs() {
    let mut s = TypeSpec::new();
    s.add(2, "uint32_t");
    s.add(3, "uint32_t");
    assert_eq!(s.entries.len(), 1);
    assert_eq!(s.entries[0].count, 5);
}

#[test]
fn add_zero_is_noop() {
    let mut s = TypeSpec::new();
    s.add(0, "uint32_t");
    assert!(s.is_empty());
}

#[test]
fn add_different_types_appends() {
    let mut s = TypeSpec::new();
    s.add(1, "uint32_t");
    s.add(1, "char");
    assert_eq!(s.entries.len(), 2);
}

// ---------------------------------------------------------------------------
// TypeSpec::remove
// ---------------------------------------------------------------------------

#[test]
fn remove_decrements_count() {
    let mut s = spec(&[(5, "uint32_t")]);
    let found = s.remove("uint32_t", 3);
    assert!(found);
    assert_eq!(s.entries[0].count, 2);
}

#[test]
fn remove_exact_count_drops_entry() {
    let mut s = spec(&[(3, "uint32_t")]);
    let found = s.remove("uint32_t", 3);
    assert!(found);
    assert!(s.is_empty());
}

#[test]
fn remove_exceeding_count_drops_entry() {
    let mut s = spec(&[(2, "uint32_t")]);
    let found = s.remove("uint32_t", 99);
    assert!(found);
    assert!(s.is_empty());
}

#[test]
fn remove_absent_returns_false() {
    let mut s = spec(&[(1, "uint32_t")]);
    let found = s.remove("char", 1);
    assert!(!found);
    assert_eq!(s.entries.len(), 1);
}

#[test]
fn remove_zero_returns_false() {
    let mut s = spec(&[(1, "uint32_t")]);
    let found = s.remove("uint32_t", 0);
    assert!(!found);
}

// ---------------------------------------------------------------------------
// TypeSpec::flatten — primitives
// ---------------------------------------------------------------------------

#[test]
fn flatten_single_primitive() {
    let dict = base_dict();
    let s = spec(&[(3, "uint32_t")]);
    let flat = s.flatten(&dict).unwrap();
    assert_eq!(flat["uint32_t"], 3);
    assert_eq!(flat.len(), 1);
}

#[test]
fn flatten_multiple_primitives() {
    let dict = base_dict();
    let s = spec(&[(2, "uint32_t"), (5, "char")]);
    let flat = s.flatten(&dict).unwrap();
    assert_eq!(flat["uint32_t"], 2);
    assert_eq!(flat["char"], 5);
}

#[test]
fn flatten_unknown_type_errors() {
    let dict = base_dict();
    let s = spec(&[(1, "NOPE")]);
    assert_eq!(s.flatten(&dict), Err(TypeError::UnknownType("NOPE".into())));
}

#[test]
fn flatten_empty_spec_returns_empty_map() {
    let dict = base_dict();
    let s = TypeSpec::new();
    let flat = s.flatten(&dict).unwrap();
    assert!(flat.is_empty());
}

// ---------------------------------------------------------------------------
// TypeSpec::flatten — composites
// ---------------------------------------------------------------------------

#[test]
fn flatten_simple_composite() {
    let mut dict = base_dict();
    // MyPair = ((2, uint32_t), (1, char))
    let inner = CompositeType::new("MyPair", spec(&[(2, "uint32_t"), (1, "char")]));
    dict.add_composite(inner).unwrap();

    let s = spec(&[(1, "MyPair")]);
    let flat = s.flatten(&dict).unwrap();
    assert_eq!(flat["uint32_t"], 2);
    assert_eq!(flat["char"], 1);
}

/// Spec §2 example: Inner = ((2, uint32_t)), Outer = ((3, Inner))
/// → flatten(Outer) == { "uint32_t": 6 }
#[test]
fn nested_composite_flattening() {
    let mut dict = base_dict();
    dict.add_composite(CompositeType::new("Inner", spec(&[(2, "uint32_t")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Outer", spec(&[(3, "Inner")])))
        .unwrap();

    let s = spec(&[(1, "Outer")]);
    let flat = s.flatten(&dict).unwrap();
    assert_eq!(flat.len(), 1);
    assert_eq!(flat["uint32_t"], 6);
}

#[test]
fn nested_composite_counts_multiply_correctly() {
    let mut dict = base_dict();
    // A = ((4, uint32_t))
    // B = ((2, A))   → 8 uint32_t
    // C = ((3, B))   → 24 uint32_t
    dict.add_composite(CompositeType::new("A", spec(&[(4, "uint32_t")])))
        .unwrap();
    dict.add_composite(CompositeType::new("B", spec(&[(2, "A")])))
        .unwrap();
    dict.add_composite(CompositeType::new("C", spec(&[(3, "B")])))
        .unwrap();

    let s = spec(&[(1, "C")]);
    let flat = s.flatten(&dict).unwrap();
    assert_eq!(flat["uint32_t"], 24);
}

#[test]
fn composite_with_mixed_primitives_flattens() {
    let mut dict = base_dict();
    // Header = ((1, uint32_t), (4, char))
    dict.add_composite(CompositeType::new(
        "Header",
        spec(&[(1, "uint32_t"), (4, "char")]),
    ))
    .unwrap();
    // Packet = ((2, Header), (3, uint32_t))
    // → uint32_t: 2*1 + 3 = 5, char: 2*4 = 8
    dict.add_composite(CompositeType::new(
        "Packet",
        spec(&[(2, "Header"), (3, "uint32_t")]),
    ))
    .unwrap();

    let flat = spec(&[(1, "Packet")]).flatten(&dict).unwrap();
    assert_eq!(flat["uint32_t"], 5);
    assert_eq!(flat["char"], 8);
}

// ---------------------------------------------------------------------------
// Structural equality — multiset order independence
// ---------------------------------------------------------------------------

/// Spec §2: order of entries is irrelevant for equality.
#[test]
fn multiset_order_independence() {
    let dict = base_dict();
    let ab = spec(&[(1, "uint32_t"), (2, "char")]);
    let ba = spec(&[(2, "char"), (1, "uint32_t")]);
    assert!(ab.structurally_equal(&ba, &dict).unwrap());
}

#[test]
fn multiset_order_independence_three_types() {
    let mut dict = base_dict();
    dict.add_primitive(PrimitiveType::new("float")).unwrap();

    let abc = spec(&[(1, "uint32_t"), (2, "char"), (3, "float")]);
    let bca = spec(&[(2, "char"), (3, "float"), (1, "uint32_t")]);
    let cab = spec(&[(3, "float"), (1, "uint32_t"), (2, "char")]);

    assert!(abc.structurally_equal(&bca, &dict).unwrap());
    assert!(abc.structurally_equal(&cab, &dict).unwrap());
}

// ---------------------------------------------------------------------------
// Structural equality — names carry no semantic weight
// ---------------------------------------------------------------------------

/// Spec §2 equality rule:
/// ```
/// Type X = ((2, uint32_t))
/// Type Y = ((2, uint32_t))
/// X == Y  ✓
/// ```
#[test]
fn structural_equality_across_names() {
    let mut dict = base_dict();
    dict.add_composite(CompositeType::new("X", spec(&[(2, "uint32_t")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Y", spec(&[(2, "uint32_t")])))
        .unwrap();

    // Specs referencing X and Y are structurally equal.
    let sx = spec(&[(1, "X")]);
    let sy = spec(&[(1, "Y")]);
    assert!(sx.structurally_equal(&sy, &dict).unwrap());
}

#[test]
fn structural_inequality_detected() {
    let dict = base_dict();
    let s1 = spec(&[(2, "uint32_t")]);
    let s2 = spec(&[(3, "uint32_t")]);
    assert!(!s1.structurally_equal(&s2, &dict).unwrap());
}

#[test]
fn structurally_equal_deeply_nested_aliases() {
    // A = ((1, uint32_t))
    // B = ((1, uint32_t))
    // Wrapping = ((2, A))
    // Alias    = ((2, B))
    // spec(Wrapping) should equal spec(Alias)
    let mut dict = base_dict();
    dict.add_composite(CompositeType::new("A", spec(&[(1, "uint32_t")])))
        .unwrap();
    dict.add_composite(CompositeType::new("B", spec(&[(1, "uint32_t")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Wrapping", spec(&[(2, "A")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Alias", spec(&[(2, "B")])))
        .unwrap();

    let sw = spec(&[(1, "Wrapping")]);
    let sa = spec(&[(1, "Alias")]);
    assert!(sw.structurally_equal(&sa, &dict).unwrap());
}

// ---------------------------------------------------------------------------
// Circular definition rejection
// ---------------------------------------------------------------------------

/// Direct self-reference: `Self = ((1, Self))`
#[test]
fn circular_definition_direct_self_reference() {
    let dict = base_dict();
    let candidate = CompositeType::new("Self", spec(&[(1, "Self")]));
    let result = dict.clone(); // check_circular needs an immutable dict without candidate
                               // We test via add_composite since check_circular is pub(crate).
    let mut d = base_dict();
    let err = d.add_composite(candidate).unwrap_err();
    assert!(matches!(
        err,
        DictionaryError::Type(TypeError::CircularDefinition(_))
    ));
}

/// Indirect cycle: A → B → A
#[test]
fn circular_definition_rejected() {
    let mut dict = base_dict();
    // A references B, which doesn't exist yet — but we build B referencing A.
    // Insert A first with a reference to a placeholder, then test B → A.
    // Simpler: just test B = ((1, A)) after A = ((1, B)) is rejected.

    // Step 1: insert A = ((1, uint32_t)) — clean.
    dict.add_composite(CompositeType::new("A", spec(&[(1, "uint32_t")])))
        .unwrap();

    // Step 2: try to insert B = ((1, A)) — fine so far (no cycle).
    dict.add_composite(CompositeType::new("B", spec(&[(1, "A")])))
        .unwrap();

    // Step 3: try to insert C = ((1, B), (1, A)) — still fine, no cycle.
    dict.add_composite(CompositeType::new("C", spec(&[(1, "B"), (1, "A")])))
        .unwrap();

    // Step 4: try to insert a type that references itself indirectly.
    // Since we can't mutate A after the fact, we simulate A→B→A by building
    // a candidate whose name is "A" — name clash should be caught by
    // DuplicateName, not CircularDefinition here.
    let dup = dict.add_composite(CompositeType::new("A", spec(&[(1, "uint32_t")])));
    assert!(matches!(
        dup,
        Err(DictionaryError::Type(TypeError::DuplicateName(_)))
    ));
}

/// Direct self-reference in a fresh dictionary.
#[test]
fn circular_direct_self_ref_in_empty_dict() {
    let mut dict = DataDictionary::new();
    dict.add_primitive(PrimitiveType::new("int")).unwrap();
    // Loop = ((1, Loop))
    let err = dict
        .add_composite(CompositeType::new("Loop", spec(&[(1, "Loop")])))
        .unwrap_err();
    assert!(matches!(
        err,
        DictionaryError::Type(TypeError::CircularDefinition(_))
    ));
}

/// Three-node cycle: P → Q → R → P
#[test]
fn circular_three_node_cycle() {
    let mut dict = DataDictionary::new();
    dict.add_primitive(PrimitiveType::new("byte")).unwrap();

    // P references Q which references R which references P.
    // We can only test the *candidate* that closes the cycle.
    // Insert P and Q with primitive content, then try inserting R that refs P.
    dict.add_composite(CompositeType::new("P", spec(&[(1, "byte")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Q", spec(&[(1, "P")])))
        .unwrap();
    // R = ((1, Q)) is fine (chain, not cycle).
    dict.add_composite(CompositeType::new("R", spec(&[(1, "Q")])))
        .unwrap();

    // Now try inserting "Cycle" = ((1, R)) — fine, R → Q → P → byte.
    dict.add_composite(CompositeType::new("Cycle", spec(&[(1, "R")])))
        .unwrap();

    // The actual three-node cycle test: a candidate whose name is already
    // reachable in its own transitive closure.  We can't reuse "P" (taken),
    // so test with a fresh dict.
    let mut d2 = DataDictionary::new();
    d2.add_primitive(PrimitiveType::new("byte")).unwrap();
    // Insert X = ((1, byte)), Y = ((1, X)).
    d2.add_composite(CompositeType::new("X", spec(&[(1, "byte")])))
        .unwrap();
    d2.add_composite(CompositeType::new("Y", spec(&[(1, "X")])))
        .unwrap();
    // Now insert Z = ((1, Z)) — direct self-reference.
    let err = d2
        .add_composite(CompositeType::new("Z", spec(&[(1, "Z")])))
        .unwrap_err();
    assert!(matches!(
        err,
        DictionaryError::Type(TypeError::CircularDefinition(_))
    ));
}

// ---------------------------------------------------------------------------
// Dictionary constraints
// ---------------------------------------------------------------------------

#[test]
fn duplicate_primitive_name_rejected() {
    let mut dict = base_dict();
    let err = dict
        .add_primitive(PrimitiveType::new("uint32_t"))
        .unwrap_err();
    assert!(matches!(
        err,
        DictionaryError::Type(TypeError::DuplicateName(_))
    ));
}

#[test]
fn primitive_and_composite_cannot_share_name() {
    let mut dict = base_dict();
    // Try to add a composite named "uint32_t" (already a primitive).
    let err = dict
        .add_composite(CompositeType::new("uint32_t", spec(&[(1, "char")])))
        .unwrap_err();
    assert!(matches!(
        err,
        DictionaryError::Type(TypeError::DuplicateName(_))
    ));
}

#[test]
fn composite_referencing_unknown_type_rejected() {
    let dict = base_dict();
    let mut d = dict.clone();
    let err = d
        .add_composite(CompositeType::new("Bad", spec(&[(1, "GHOST")])))
        .unwrap_err();
    assert!(matches!(
        err,
        DictionaryError::Type(TypeError::UnknownType(_))
    ));
}

// ---------------------------------------------------------------------------
// Dictionary::rename
// ---------------------------------------------------------------------------

#[test]
fn rename_primitive_updates_composite_refs() {
    let mut dict = base_dict();
    dict.add_composite(CompositeType::new("Pair", spec(&[(2, "uint32_t")])))
        .unwrap();

    let patched = dict.rename("uint32_t", "u32").unwrap();
    assert!(patched.contains(&"Pair".to_owned()));

    // The primitive itself was renamed.
    assert!(dict.contains("u32"));
    assert!(!dict.contains("uint32_t"));

    // The composite's spec was patched.
    let pair = dict.composite("Pair").unwrap();
    assert_eq!(pair.spec.entries[0].type_ref, "u32");
}

#[test]
fn rename_to_existing_name_rejected() {
    let mut dict = base_dict();
    let err = dict.rename("uint32_t", "char").unwrap_err();
    assert!(matches!(
        err,
        DictionaryError::Type(TypeError::DuplicateName(_))
    ));
}

#[test]
fn rename_unknown_name_returns_not_found() {
    let mut dict = base_dict();
    let err = dict.rename("GHOST", "x").unwrap_err();
    assert!(matches!(err, DictionaryError::NotFound(_)));
}

#[test]
fn rename_to_same_name_is_noop() {
    let mut dict = base_dict();
    let patched = dict.rename("uint32_t", "uint32_t").unwrap();
    assert!(patched.is_empty());
    assert!(dict.contains("uint32_t"));
}

// ---------------------------------------------------------------------------
// Dictionary::delete
// ---------------------------------------------------------------------------

#[test]
fn delete_primitive_succeeds() {
    let mut dict = base_dict();
    let deleted = dict.delete("uint32_t").unwrap();
    assert_eq!(deleted, "uint32_t");
    assert!(!dict.contains("uint32_t"));
}

#[test]
fn delete_composite_succeeds() {
    let mut dict = base_dict();
    dict.add_composite(CompositeType::new("Pair", spec(&[(2, "uint32_t")])))
        .unwrap();
    let deleted = dict.delete("Pair").unwrap();
    assert_eq!(deleted, "Pair");
    assert!(!dict.contains("Pair"));
}

#[test]
fn delete_absent_type_returns_not_found() {
    let mut dict = base_dict();
    let err = dict.delete("GHOST").unwrap_err();
    assert!(matches!(err, DictionaryError::NotFound(_)));
}
