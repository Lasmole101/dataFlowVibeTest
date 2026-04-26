//! Flattening and structural equality for the validation layer (spec §4.1).
//!
//! # Why this module exists
//!
//! The canonical recursive implementation lives in
//! [`crate::model::types::flatten_type_into`], which is `pub(crate)` and
//! called directly by [`crate::model::types::TypeSpec::flatten`].  This
//! module re-exposes that logic through a stable validation-layer API so that:
//!
//! * Validation code imports from `validation::flatten`, not from `model::types`,
//!   keeping the layering explicit.
//! * The seam can be extended (e.g. adding instrumentation, caching, or
//!   corruption guards for malformed persisted data) without touching the model.
//!
//! # Key functions
//!
//! | Function | Purpose |
//! |---|---|
//! | [`flatten_spec`] | Flatten one `TypeSpec` → `HashMap<String, u32>` |
//! | [`structurally_equal`] | Compare two `TypeSpec`s after flattening |
//! | [`sum_specs`] | Merge many flattened maps into one (used by balance checkers) |

use std::collections::HashMap;

use crate::model::{
    dictionary::DataDictionary,
    types::{TypeError, TypeSpec},
};

// ---------------------------------------------------------------------------
// flatten_spec
// ---------------------------------------------------------------------------

/// Flatten `spec` through `dict` into a map of `primitive_name → count`.
///
/// All composite references are recursively expanded until only primitive
/// names remain.  The result is the canonical form used for structural
/// equality comparisons (spec §4.1).
///
/// Flattening an **empty** spec always succeeds and returns an empty map.
///
/// # Errors
/// Returns [`TypeError::UnknownType`] for the first name not found in `dict`.
///
/// # Example
/// ```ignore
/// // dict: uint32_t (primitive), Pair = ((2, uint32_t)) (composite)
/// let flat = flatten_spec(&spec(&[(3, "Pair")]), &dict).unwrap();
/// assert_eq!(flat["uint32_t"], 6); // 3 × 2
/// ```
pub fn flatten_spec(
    spec: &TypeSpec,
    dict: &DataDictionary,
) -> Result<HashMap<String, u32>, TypeError> {
    spec.flatten(dict)
}

// ---------------------------------------------------------------------------
// structurally_equal
// ---------------------------------------------------------------------------

/// Returns `true` if `a` and `b` flatten to the same map of primitive counts.
///
/// This is the spec §4.1 compatibility test: type names carry no semantic
/// weight.  Two specs built from differently-named composites that resolve to
/// the same primitives are considered equal.
///
/// # Errors
/// Returns [`TypeError::UnknownType`] if either spec references a name absent
/// from `dict`.  The error is returned on the first unknown name encountered;
/// the other spec may or may not be valid.
pub fn structurally_equal(
    a: &TypeSpec,
    b: &TypeSpec,
    dict: &DataDictionary,
) -> Result<bool, TypeError> {
    Ok(flatten_spec(a, dict)? == flatten_spec(b, dict)?)
}

// ---------------------------------------------------------------------------
// sum_specs
// ---------------------------------------------------------------------------

/// Merge an iterator of already-flattened spec maps into a single
/// `primitive_name → total_count` map.
///
/// Used by balance checkers to accumulate the combined type contributions of
/// all incoming (or outgoing) streams attached to a node without re-flattening
/// the dictionary on every call.
///
/// # Example
/// ```ignore
/// let incoming: Vec<HashMap<String, u32>> = streams
///     .iter()
///     .map(|s| flatten_spec(&s.type_spec, dict))
///     .collect::<Result<_, _>>()?;
/// let total = sum_specs(incoming.iter());
/// ```
pub fn sum_specs<'a>(maps: impl Iterator<Item = &'a HashMap<String, u32>>) -> HashMap<String, u32> {
    let mut acc: HashMap<String, u32> = HashMap::new();
    for map in maps {
        for (name, count) in map {
            *acc.entry(name.clone()).or_insert(0) += count;
        }
    }
    acc
}

// ---------------------------------------------------------------------------
// subtract_specs
// ---------------------------------------------------------------------------

/// Subtract `rhs` from `lhs`, returning the per-primitive surplus (positive)
/// or deficit (negative as `i64`).
///
/// Used by balance checkers to determine over- or under-supply relative to a
/// node specification:
/// * Positive value → more supply than the spec requires (over-supply warning).
/// * Negative value → less supply than the spec requires (under-supply warning).
/// * Zero → perfectly balanced.
///
/// Primitives present in `rhs` but absent from `lhs` appear with a negative
/// value.  Primitives present in `lhs` but absent from `rhs` appear with a
/// positive value.
pub fn subtract_specs(
    lhs: &HashMap<String, u32>,
    rhs: &HashMap<String, u32>,
) -> HashMap<String, i64> {
    let mut result: HashMap<String, i64> = HashMap::new();

    for (name, &count) in lhs {
        *result.entry(name.clone()).or_insert(0) += count as i64;
    }
    for (name, &count) in rhs {
        *result.entry(name.clone()).or_insert(0) -= count as i64;
    }

    // Drop zero entries — a zero delta is balanced and carries no information.
    result.retain(|_, v| *v != 0);
    result
}

// ---------------------------------------------------------------------------
// is_balanced
// ---------------------------------------------------------------------------

/// Returns `true` if every primitive in `required` is present in `supplied`
/// with an equal count (exact match, no over- or under-supply).
///
/// This is the strict balance predicate used for stream-level checks.
pub fn is_balanced(supplied: &HashMap<String, u32>, required: &HashMap<String, u32>) -> bool {
    subtract_specs(supplied, required).is_empty()
}
