//! Tests for `validation::flatten` (spec §4.1).
//!
//! These tests exercise the *validation-layer* public API
//! (`flatten_spec`, `structurally_equal`, `sum_specs`, `subtract_specs`,
//! `is_balanced`) rather than `TypeSpec::flatten` directly.
//!
//! Basic flatten behaviour is already covered in `type_equality.rs`.  This
//! file focuses on:
//! * Edge cases the validation layer must handle correctly.
//! * Helpers that only exist at the validation layer (`sum_specs`, etc.).
//! * Regression guards for tricky graph shapes (diamond, very deep nesting).

use std::collections::HashMap;

use dataFlowClaude::{
    model::{
        dictionary::DataDictionary,
        types::{CompositeType, PrimitiveType, TypeError, TypeSpec},
    },
    validation::flatten::{
        flatten_spec, is_balanced, structurally_equal, subtract_specs, sum_specs,
    },
};

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

fn flat(pairs: &[(&str, u32)]) -> HashMap<String, u32> {
    pairs.iter().map(|&(k, v)| (k.to_owned(), v)).collect()
}

// ---------------------------------------------------------------------------
// flatten_spec — basic cases (validation-layer API)
// ---------------------------------------------------------------------------

#[test]
fn flatten_empty_spec_returns_empty_map() {
    let dict = dict_with(&["u32"]);
    let result = flatten_spec(&TypeSpec::new(), &dict).unwrap();
    assert!(result.is_empty());
}

#[test]
fn flatten_single_primitive() {
    let dict = dict_with(&["u32"]);
    let result = flatten_spec(&spec(&[(4, "u32")]), &dict).unwrap();
    assert_eq!(result, flat(&[("u32", 4)]));
}

#[test]
fn flatten_multiple_primitives() {
    let dict = dict_with(&["u32", "char"]);
    let result = flatten_spec(&spec(&[(2, "u32"), (5, "char")]), &dict).unwrap();
    assert_eq!(result, flat(&[("u32", 2), ("char", 5)]));
}

#[test]
fn flatten_unknown_type_returns_error() {
    let dict = dict_with(&["u32"]);
    assert_eq!(
        flatten_spec(&spec(&[(1, "GHOST")]), &dict),
        Err(TypeError::UnknownType("GHOST".into()))
    );
}

// ---------------------------------------------------------------------------
// flatten_spec — composite expansion
// ---------------------------------------------------------------------------

#[test]
fn flatten_simple_composite() {
    let mut dict = dict_with(&["u32", "char"]);
    dict.add_composite(CompositeType::new("Pair", spec(&[(1, "u32"), (1, "char")])))
        .unwrap();
    let result = flatten_spec(&spec(&[(1, "Pair")]), &dict).unwrap();
    assert_eq!(result, flat(&[("u32", 1), ("char", 1)]));
}

#[test]
fn flatten_scaled_composite() {
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new("Word", spec(&[(2, "u32")])))
        .unwrap();
    // 3 × Word = 3 × 2 × u32 = 6
    let result = flatten_spec(&spec(&[(3, "Word")]), &dict).unwrap();
    assert_eq!(result, flat(&[("u32", 6)]));
}

/// Spec §2 example: Inner = ((2, u32)), Outer = ((3, Inner)) → 6 u32.
#[test]
fn flatten_nested_composites() {
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new("Inner", spec(&[(2, "u32")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Outer", spec(&[(3, "Inner")])))
        .unwrap();
    let result = flatten_spec(&spec(&[(1, "Outer")]), &dict).unwrap();
    assert_eq!(result, flat(&[("u32", 6)]));
}

/// Five levels deep: counts must multiply correctly at each level.
#[test]
fn flatten_deeply_nested_composites() {
    let mut dict = dict_with(&["byte"]);
    // L1 = ((2, byte))     → 2 byte
    // L2 = ((2, L1))       → 4 byte
    // L3 = ((2, L2))       → 8 byte
    // L4 = ((2, L3))       → 16 byte
    // L5 = ((2, L4))       → 32 byte
    dict.add_composite(CompositeType::new("L1", spec(&[(2, "byte")])))
        .unwrap();
    dict.add_composite(CompositeType::new("L2", spec(&[(2, "L1")])))
        .unwrap();
    dict.add_composite(CompositeType::new("L3", spec(&[(2, "L2")])))
        .unwrap();
    dict.add_composite(CompositeType::new("L4", spec(&[(2, "L3")])))
        .unwrap();
    dict.add_composite(CompositeType::new("L5", spec(&[(2, "L4")])))
        .unwrap();
    let result = flatten_spec(&spec(&[(1, "L5")]), &dict).unwrap();
    assert_eq!(result, flat(&[("byte", 32)]));
}

/// Diamond-shaped composite graph: A referenced by both B and C, with D
/// referencing both B and C.  Counts must be additive, not deduplicated.
///
/// ```text
/// A = ((1, u32))
/// B = ((2, A))   → 2 u32
/// C = ((3, A))   → 3 u32
/// D = ((1, B), (1, C))  → 2+3 = 5 u32
/// ```
#[test]
fn flatten_diamond_composite_graph() {
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new("A", spec(&[(1, "u32")])))
        .unwrap();
    dict.add_composite(CompositeType::new("B", spec(&[(2, "A")])))
        .unwrap();
    dict.add_composite(CompositeType::new("C", spec(&[(3, "A")])))
        .unwrap();
    dict.add_composite(CompositeType::new("D", spec(&[(1, "B"), (1, "C")])))
        .unwrap();
    let result = flatten_spec(&spec(&[(1, "D")]), &dict).unwrap();
    assert_eq!(result, flat(&[("u32", 5)]));
}

/// A composite that mixes a directly referenced primitive with a composite
/// that also references the same primitive.  Counts from both paths must sum.
#[test]
fn flatten_direct_and_indirect_primitive_paths_sum() {
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new("Inner", spec(&[(4, "u32")])))
        .unwrap();
    // Spec: (2, u32) + (1, Inner) = 2 + 4 = 6 u32
    let s = spec(&[(2, "u32"), (1, "Inner")]);
    let result = flatten_spec(&s, &dict).unwrap();
    assert_eq!(result, flat(&[("u32", 6)]));
}

/// Unknown type nested *inside* a composite — error must propagate.
#[test]
fn flatten_unknown_type_inside_composite_errors() {
    let mut dict = dict_with(&["u32"]);
    // Manually build a composite with a broken member without going through
    // add_composite (which would reject the unknown reference).
    // We can't do this via the dictionary API (it validates), so instead test
    // a TypeSpec that directly references an unknown name.
    let s = spec(&[(1, "MISSING")]);
    assert_eq!(
        flatten_spec(&s, &dict),
        Err(TypeError::UnknownType("MISSING".into()))
    );
}

// ---------------------------------------------------------------------------
// structurally_equal
// ---------------------------------------------------------------------------

#[test]
fn structurally_equal_same_primitives_different_order() {
    let dict = dict_with(&["u32", "char"]);
    let a = spec(&[(1, "u32"), (2, "char")]);
    let b = spec(&[(2, "char"), (1, "u32")]);
    assert!(structurally_equal(&a, &b, &dict).unwrap());
}

#[test]
fn structurally_equal_composites_with_different_names() {
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new("X", spec(&[(2, "u32")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Y", spec(&[(2, "u32")])))
        .unwrap();
    let sx = spec(&[(1, "X")]);
    let sy = spec(&[(1, "Y")]);
    assert!(structurally_equal(&sx, &sy, &dict).unwrap());
}

#[test]
fn structurally_equal_detects_inequality() {
    let dict = dict_with(&["u32", "char"]);
    let a = spec(&[(2, "u32")]);
    let b = spec(&[(3, "u32")]);
    assert!(!structurally_equal(&a, &b, &dict).unwrap());
}

#[test]
fn structurally_equal_different_primitives_not_equal() {
    let dict = dict_with(&["u32", "char"]);
    let a = spec(&[(1, "u32")]);
    let b = spec(&[(1, "char")]);
    assert!(!structurally_equal(&a, &b, &dict).unwrap());
}

#[test]
fn structurally_equal_empty_specs_are_equal() {
    let dict = dict_with(&["u32"]);
    assert!(structurally_equal(&TypeSpec::new(), &TypeSpec::new(), &dict).unwrap());
}

#[test]
fn structurally_equal_empty_vs_nonempty_is_false() {
    let dict = dict_with(&["u32"]);
    let a = TypeSpec::new();
    let b = spec(&[(1, "u32")]);
    assert!(!structurally_equal(&a, &b, &dict).unwrap());
}

#[test]
fn structurally_equal_returns_error_on_unknown_lhs() {
    let dict = dict_with(&["u32"]);
    let a = spec(&[(1, "GHOST")]);
    let b = spec(&[(1, "u32")]);
    assert_eq!(
        structurally_equal(&a, &b, &dict),
        Err(TypeError::UnknownType("GHOST".into()))
    );
}

#[test]
fn structurally_equal_nested_alias_equivalence() {
    // A = ((1, u32)), B = ((1, u32))
    // Wrap2A = ((2, A)), Wrap2B = ((2, B))
    // spec(Wrap2A) must equal spec(Wrap2B).
    let mut dict = dict_with(&["u32"]);
    dict.add_composite(CompositeType::new("A", spec(&[(1, "u32")])))
        .unwrap();
    dict.add_composite(CompositeType::new("B", spec(&[(1, "u32")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Wrap2A", spec(&[(2, "A")])))
        .unwrap();
    dict.add_composite(CompositeType::new("Wrap2B", spec(&[(2, "B")])))
        .unwrap();
    let sw = spec(&[(1, "Wrap2A")]);
    let sb = spec(&[(1, "Wrap2B")]);
    assert!(structurally_equal(&sw, &sb, &dict).unwrap());
}

// ---------------------------------------------------------------------------
// sum_specs
// ---------------------------------------------------------------------------

#[test]
fn sum_specs_empty_iterator_returns_empty() {
    let result = sum_specs(std::iter::empty());
    assert!(result.is_empty());
}

#[test]
fn sum_specs_single_map_is_identity() {
    let m = flat(&[("u32", 3), ("char", 1)]);
    let result = sum_specs(std::iter::once(&m));
    assert_eq!(result, m);
}

#[test]
fn sum_specs_accumulates_counts_for_same_primitive() {
    let m1 = flat(&[("u32", 2)]);
    let m2 = flat(&[("u32", 3)]);
    let result = sum_specs([&m1, &m2].iter().copied());
    assert_eq!(result["u32"], 5);
}

#[test]
fn sum_specs_merges_disjoint_primitives() {
    let m1 = flat(&[("u32", 2)]);
    let m2 = flat(&[("char", 4)]);
    let result = sum_specs([&m1, &m2].iter().copied());
    assert_eq!(result["u32"], 2);
    assert_eq!(result["char"], 4);
}

#[test]
fn sum_specs_three_overlapping_maps() {
    let m1 = flat(&[("u32", 1), ("char", 2)]);
    let m2 = flat(&[("u32", 3)]);
    let m3 = flat(&[("char", 1), ("float", 5)]);
    let result = sum_specs([&m1, &m2, &m3].iter().copied());
    assert_eq!(result["u32"], 4); // 1 + 3
    assert_eq!(result["char"], 3); // 2 + 1
    assert_eq!(result["float"], 5);
}

// ---------------------------------------------------------------------------
// subtract_specs
// ---------------------------------------------------------------------------

#[test]
fn subtract_specs_identical_maps_empty_result() {
    let m = flat(&[("u32", 4), ("char", 2)]);
    let result = subtract_specs(&m, &m);
    assert!(result.is_empty(), "identical maps must subtract to empty");
}

#[test]
fn subtract_specs_surplus_is_positive() {
    let supply = flat(&[("u32", 5)]);
    let demand = flat(&[("u32", 3)]);
    let result = subtract_specs(&supply, &demand);
    assert_eq!(result["u32"], 2);
}

#[test]
fn subtract_specs_deficit_is_negative() {
    let supply = flat(&[("u32", 1)]);
    let demand = flat(&[("u32", 4)]);
    let result = subtract_specs(&supply, &demand);
    assert_eq!(result["u32"], -3);
}

#[test]
fn subtract_specs_type_only_in_rhs_is_negative() {
    let supply: HashMap<String, u32> = HashMap::new();
    let demand = flat(&[("u32", 2)]);
    let result = subtract_specs(&supply, &demand);
    assert_eq!(result["u32"], -2);
}

#[test]
fn subtract_specs_type_only_in_lhs_is_positive() {
    let supply = flat(&[("char", 3)]);
    let demand: HashMap<String, u32> = HashMap::new();
    let result = subtract_specs(&supply, &demand);
    assert_eq!(result["char"], 3);
}

#[test]
fn subtract_specs_zero_entries_dropped() {
    let m = flat(&[("u32", 2)]);
    let result = subtract_specs(&m, &m);
    assert!(!result.contains_key("u32"), "zero delta must be dropped");
}

// ---------------------------------------------------------------------------
// is_balanced
// ---------------------------------------------------------------------------

#[test]
fn is_balanced_exact_match_is_true() {
    let m = flat(&[("u32", 3), ("char", 1)]);
    assert!(is_balanced(&m, &m));
}

#[test]
fn is_balanced_surplus_is_false() {
    let supply = flat(&[("u32", 4)]);
    let demand = flat(&[("u32", 3)]);
    assert!(!is_balanced(&supply, &demand));
}

#[test]
fn is_balanced_deficit_is_false() {
    let supply = flat(&[("u32", 2)]);
    let demand = flat(&[("u32", 3)]);
    assert!(!is_balanced(&supply, &demand));
}

#[test]
fn is_balanced_extra_primitive_in_supply_is_false() {
    let supply = flat(&[("u32", 2), ("char", 1)]);
    let demand = flat(&[("u32", 2)]);
    assert!(!is_balanced(&supply, &demand));
}

#[test]
fn is_balanced_extra_primitive_in_demand_is_false() {
    let supply = flat(&[("u32", 2)]);
    let demand = flat(&[("u32", 2), ("char", 1)]);
    assert!(!is_balanced(&supply, &demand));
}

#[test]
fn is_balanced_both_empty_is_true() {
    assert!(is_balanced(&HashMap::new(), &HashMap::new()));
}
