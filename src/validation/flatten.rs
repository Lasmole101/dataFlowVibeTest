//! Public re-export of the recursive [`TypeSpec`] flattening logic (spec §4.1).
//!
//! The canonical implementation lives in
//! [`crate::model::types::flatten_type_into`], which is `pub(crate)` so that
//! both [`crate::model::types::TypeSpec::flatten`] and this module can call
//! it without duplicating code.
//!
//! Validation code that needs to flatten a spec should import
//! [`flatten_spec`] from here rather than calling `TypeSpec::flatten`
//! directly, so that the validation layer has a clean seam to extend later
//! (e.g. adding cycle-detection guards for corrupted persisted data).

use std::collections::HashMap;

use crate::model::{
    dictionary::DataDictionary,
    types::{flatten_type_into, TypeError, TypeSpec},
};

/// Flatten `spec` through `dict` into a map of `primitive_name → count`.
///
/// All composite references are recursively expanded until only primitive
/// names remain.  The result is the canonical form used for structural
/// equality comparisons (spec §4.1).
///
/// # Errors
/// Returns [`TypeError::UnknownType`] for the first name not found in `dict`.
pub fn flatten_spec(
    spec: &TypeSpec,
    dict: &DataDictionary,
) -> Result<HashMap<String, u32>, TypeError> {
    // Delegate to the shared implementation in the model layer.
    spec.flatten(dict)
}
