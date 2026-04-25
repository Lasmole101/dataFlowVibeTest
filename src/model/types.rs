//! Core type-system primitives.
//!
//! The type system follows the rules in spec §2:
//!
//! * A **primitive** is a named leaf with no internal structure.
//! * A **composite** is a named multiset of `(count, TypeRef)` pairs.
//! * Two type expressions are **equal** when they flatten to the same
//!   multiset of primitive counts, regardless of name.
//!
//! [`TypeSpec`] is the workhorse: it appears on every node and stream as
//! the description of what data they carry or accept.
//!
//! # Flattening
//!
//! Flattening resolves every [`TypeRef`] to its constituent primitives by
//! recursing through the [`DataDictionary`].  The result is a
//! `HashMap<String, u32>` mapping each primitive name to its total count.
//! This map is the canonical form used for structural equality tests.
//!
//! # Circular-definition detection
//!
//! [`check_circular`] performs a depth-first walk of a candidate composite's
//! members *before* it is inserted into the dictionary.  It is called by
//! [`crate::model::dictionary::DataDictionary::add_composite`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::dictionary::DataDictionary;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can arise during type-system operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TypeError {
    /// A referenced type name does not exist in the data dictionary.
    #[error("unknown type '{0}'")]
    UnknownType(String),

    /// Adding this composite would create a cycle in the type graph.
    #[error("circular type definition involving '{0}'")]
    CircularDefinition(String),

    /// A type with this name already exists in the dictionary.
    #[error("type name '{0}' is already in use")]
    DuplicateName(String),
}

// ---------------------------------------------------------------------------
// PrimitiveType
// ---------------------------------------------------------------------------

/// A named leaf type with no internal structure (e.g. `uint32_t`, `char`).
///
/// The name is the entire definition.  Primitive identity is by name, but
/// [`TypeSpec`] structural equality ignores names entirely.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrimitiveType(pub String);

impl PrimitiveType {
    /// Create a new primitive with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The declared name of this primitive.
    pub fn name(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// TypeRef
// ---------------------------------------------------------------------------

/// A reference to a type by name.
///
/// Resolved against the [`DataDictionary`] at validation/flatten time.
/// May refer to either a primitive or a composite.
pub type TypeRef = String;

// ---------------------------------------------------------------------------
// TypeEntry
// ---------------------------------------------------------------------------

/// One entry in a [`TypeSpec`]: a non-zero count and the name of a type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeEntry {
    /// How many instances of this type the entry represents.  Always ≥ 1.
    pub count: u32,
    /// The name of the type (primitive or composite).
    pub type_ref: TypeRef,
}

impl TypeEntry {
    /// Convenience constructor.
    pub fn new(count: u32, type_ref: impl Into<String>) -> Self {
        Self {
            count,
            type_ref: type_ref.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// TypeSpec
// ---------------------------------------------------------------------------

/// A multiset of `(count, TypeRef)` pairs describing what a node port or
/// stream carries.
///
/// The representation is canonical: each type name appears at most once in
/// `entries`; counts are merged on [`add`](TypeSpec::add).  Order within
/// `entries` is insertion order and carries no semantic weight.
///
/// [`PartialEq`] / [`Eq`] on this type compare the raw `entries` Vec — i.e.
/// *syntactic* equality.  For the spec's **structural** equality (names
/// irrelevant, counts flattened through composites) use
/// [`TypeSpec::structurally_equal`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeSpec {
    /// The entries.  Invariant: no two entries share the same `type_ref`.
    pub entries: Vec<TypeEntry>,
}

impl TypeSpec {
    /// Construct an empty spec.
    pub fn new() -> Self {
        Self::default()
    }

    /// Convenience: build a spec from an iterator of `(count, name)` pairs.
    ///
    /// Duplicate names are merged exactly as [`add`](TypeSpec::add) would.
    pub fn from_pairs<S: Into<String>>(pairs: impl IntoIterator<Item = (u32, S)>) -> Self {
        let mut spec = Self::new();
        for (count, name) in pairs {
            spec.add(count, name);
        }
        spec
    }

    /// Add `count` units of `type_ref`.
    ///
    /// If `type_ref` is already present its count is incremented; otherwise a
    /// new entry is appended.  Adding zero units is a no-op.
    pub fn add(&mut self, count: u32, type_ref: impl Into<String>) {
        if count == 0 {
            return;
        }
        let type_ref = type_ref.into();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.type_ref == type_ref) {
            entry.count += count;
        } else {
            self.entries.push(TypeEntry::new(count, type_ref));
        }
    }

    /// Decrement the count of `type_ref` by `amount`.
    ///
    /// Removes the entry entirely when its count reaches zero.
    ///
    /// Returns `false` if the type was not present or `amount` is zero.
    pub fn remove(&mut self, type_ref: &str, amount: u32) -> bool {
        if amount == 0 {
            return false;
        }
        if let Some(pos) = self.entries.iter().position(|e| e.type_ref == type_ref) {
            let entry = &mut self.entries[pos];
            if amount >= entry.count {
                self.entries.remove(pos);
            } else {
                entry.count -= amount;
            }
            true
        } else {
            false
        }
    }

    /// Recursively flatten this spec through `dict` to a map of
    /// `primitive_name → total_count`.
    ///
    /// Every composite reference is expanded until only primitives remain.
    ///
    /// # Errors
    /// Returns [`TypeError::UnknownType`] if any referenced name is absent
    /// from the dictionary.
    pub fn flatten(&self, dict: &DataDictionary) -> Result<HashMap<String, u32>, TypeError> {
        let mut acc: HashMap<String, u32> = HashMap::new();
        for entry in &self.entries {
            flatten_type_into(&entry.type_ref, entry.count, dict, &mut acc)?;
        }
        Ok(acc)
    }

    /// Structural equality: two specs are equal iff they flatten to the same
    /// map of primitive counts.  Type names carry no semantic weight.
    ///
    /// # Errors
    /// Returns [`TypeError::UnknownType`] if either spec references a name
    /// absent from `dict`.
    pub fn structurally_equal(
        &self,
        other: &TypeSpec,
        dict: &DataDictionary,
    ) -> Result<bool, TypeError> {
        Ok(self.flatten(dict)? == other.flatten(dict)?)
    }

    /// Returns `true` if this spec contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total count of all entries (sum of all counts, not number of entries).
    pub fn total_count(&self) -> u32 {
        self.entries.iter().map(|e| e.count).sum()
    }

    /// Iterate over `(count, type_ref)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &str)> {
        self.entries.iter().map(|e| (e.count, e.type_ref.as_str()))
    }
}

// ---------------------------------------------------------------------------
// Flattening — private recursive implementation
// ---------------------------------------------------------------------------

/// Recursively expand `type_name` (scaled by `multiplier`) into `acc`.
///
/// * If `type_name` is a primitive, increments `acc[type_name]` by
///   `multiplier`.
/// * If `type_name` is a composite, recurses into each of its entries,
///   multiplying counts.
/// * If `type_name` is unknown, returns [`TypeError::UnknownType`].
///
/// This function is the single implementation used by both
/// [`TypeSpec::flatten`] and the public re-export in
/// [`crate::validation::flatten`].
pub(crate) fn flatten_type_into(
    type_name: &str,
    multiplier: u32,
    dict: &DataDictionary,
    acc: &mut HashMap<String, u32>,
) -> Result<(), TypeError> {
    if dict.primitive(type_name).is_some() {
        // Base case: it's a primitive — accumulate.
        *acc.entry(type_name.to_owned()).or_insert(0) += multiplier;
        Ok(())
    } else if let Some(composite) = dict.composite(type_name) {
        // Recursive case: expand each member of the composite.
        // Clone the entries so we don't hold an immutable borrow of `dict`
        // while we recurse (which also needs `dict`).
        let entries: Vec<(u32, String)> = composite
            .spec
            .entries
            .iter()
            .map(|e| (e.count, e.type_ref.clone()))
            .collect();
        for (child_count, child_ref) in entries {
            flatten_type_into(&child_ref, multiplier * child_count, dict, acc)?;
        }
        Ok(())
    } else {
        Err(TypeError::UnknownType(type_name.to_owned()))
    }
}

// ---------------------------------------------------------------------------
// CompositeType
// ---------------------------------------------------------------------------

/// A named composite type: a name plus the multiset that defines it.
///
/// Stored in the [`DataDictionary`].  Circular definitions are rejected by
/// [`check_circular`] before insertion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeType {
    /// Declared name, unique within the dictionary.
    pub name: String,
    /// The type specification that defines this composite's structure.
    pub spec: TypeSpec,
}

impl CompositeType {
    /// Create a new composite type.
    ///
    /// This constructor does **not** check for circular definitions or
    /// duplicate names — those checks are performed by
    /// [`DataDictionary::add_composite`].
    pub fn new(name: impl Into<String>, spec: TypeSpec) -> Self {
        Self {
            name: name.into(),
            spec,
        }
    }

    /// The declared name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

// ---------------------------------------------------------------------------
// Circular-definition detection
// ---------------------------------------------------------------------------

/// Check whether inserting `candidate` into `dict` would introduce a cycle.
///
/// Performs a depth-first walk starting from each member of `candidate.spec`.
/// If the walk ever reaches a composite whose name equals `candidate.name`,
/// a cycle exists.
///
/// Also validates that every type referenced in `candidate.spec` (directly or
/// transitively) already exists in the dictionary.
///
/// # Errors
/// * [`TypeError::CircularDefinition`] — a cycle was detected.
/// * [`TypeError::UnknownType`] — a referenced type does not exist.
///
/// # Note
/// This function only reads from `dict`; `candidate` is **not** yet inserted.
pub fn check_circular(candidate: &CompositeType, dict: &DataDictionary) -> Result<(), TypeError> {
    // `visited` prevents re-expanding the same composite twice in a single
    // walk (not strictly required for correctness since cycles are caught by
    // `ancestors`, but avoids exponential blowup on diamond-shaped graphs).
    let mut visited: HashSet<String> = HashSet::new();
    let mut ancestors: HashSet<String> = HashSet::new();

    // The candidate's own name is the sentinel we look for.
    ancestors.insert(candidate.name.clone());

    for entry in &candidate.spec.entries {
        dfs_check(
            &entry.type_ref,
            &candidate.name,
            dict,
            &mut visited,
            &mut ancestors,
        )?;
    }
    Ok(())
}

/// Depth-first search used by [`check_circular`].
///
/// `root_name` is the candidate being inserted (the name we must not
/// encounter during the walk).  `ancestors` is the current DFS stack.
fn dfs_check(
    type_name: &str,
    root_name: &str,
    dict: &DataDictionary,
    visited: &mut HashSet<String>,
    ancestors: &mut HashSet<String>,
) -> Result<(), TypeError> {
    // If we encounter the root we came from, we have a cycle.
    if type_name == root_name {
        return Err(TypeError::CircularDefinition(root_name.to_owned()));
    }

    // Primitives are leaves — no further traversal needed.
    if dict.primitive(type_name).is_some() {
        return Ok(());
    }

    // Must exist as a composite if it's not a primitive.
    let composite = dict
        .composite(type_name)
        .ok_or_else(|| TypeError::UnknownType(type_name.to_owned()))?;

    // Already fully explored — skip.
    if visited.contains(type_name) {
        return Ok(());
    }

    // Detect back-edges: if `type_name` is already on the DFS stack we have
    // a cycle among existing composites (should not happen in a valid dict,
    // but guard it anyway).
    if ancestors.contains(type_name) {
        return Err(TypeError::CircularDefinition(type_name.to_owned()));
    }

    // Recurse into this composite's members.
    let entries: Vec<String> = composite
        .spec
        .entries
        .iter()
        .map(|e| e.type_ref.clone())
        .collect();
    ancestors.insert(type_name.to_owned());
    for child in &entries {
        dfs_check(child, root_name, dict, visited, ancestors)?;
    }
    ancestors.remove(type_name);
    visited.insert(type_name.to_owned());

    Ok(())
}
