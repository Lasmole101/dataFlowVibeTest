//! The project-wide data dictionary (spec §2).
//!
//! All diagrams at every level share a single [`DataDictionary`].
//! Types may only be defined here, never inline on a node or stream.
//!
//! # Invariants
//! * All names (primitive and composite combined) are unique.
//! * Composite types are acyclic; circular definitions are rejected on
//!   creation by delegating to [`crate::model::types::check_circular`].
//! * Renaming a type propagates to all composite definitions that reference
//!   the old name.
//!
//! # Mutation responsibility split
//! The dictionary is responsible only for its own consistency.  When a type is
//! deleted, the dictionary returns the deleted name; callers (commands in the
//! undo stack) are responsible for placing any referencing nodes or streams
//! into an error state.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::types::{check_circular, CompositeType, PrimitiveType, TypeError};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by dictionary mutation operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DictionaryError {
    /// Wraps a [`TypeError`] from the type-system layer.
    #[error(transparent)]
    Type(#[from] TypeError),

    /// The requested name does not exist in the dictionary.
    #[error("type '{0}' not found")]
    NotFound(String),
}

// ---------------------------------------------------------------------------
// DataDictionary
// ---------------------------------------------------------------------------

/// Holds all primitive and composite type definitions for the project.
///
/// Primitives and composites occupy the same name-space; no primitive may
/// share a name with a composite.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataDictionary {
    /// All primitive types, in insertion order.
    pub primitives: Vec<PrimitiveType>,
    /// All composite types, in insertion order.
    pub composites: Vec<CompositeType>,
}

impl DataDictionary {
    /// Create an empty dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Returns `true` if a type with the given name exists (primitive or
    /// composite).
    pub fn contains(&self, name: &str) -> bool {
        self.primitive(name).is_some() || self.composite(name).is_some()
    }

    /// Look up a primitive by name.  Returns `None` if the name is absent or
    /// belongs to a composite.
    pub fn primitive(&self, name: &str) -> Option<&PrimitiveType> {
        self.primitives.iter().find(|p| p.name() == name)
    }

    /// Look up a composite by name.  Returns `None` if the name is absent or
    /// belongs to a primitive.
    pub fn composite(&self, name: &str) -> Option<&CompositeType> {
        self.composites.iter().find(|c| c.name() == name)
    }

    /// Iterate over all type names — primitives first, then composites — in
    /// insertion order.
    pub fn all_names(&self) -> impl Iterator<Item = &str> {
        self.primitives
            .iter()
            .map(|p| p.name())
            .chain(self.composites.iter().map(|c| c.name()))
    }

    // -----------------------------------------------------------------------
    // Mutations
    // -----------------------------------------------------------------------

    /// Add a primitive type.
    ///
    /// # Errors
    /// Returns [`TypeError::DuplicateName`] if the name is already in use
    /// (by either a primitive or a composite).
    pub fn add_primitive(&mut self, primitive: PrimitiveType) -> Result<(), DictionaryError> {
        if self.contains(primitive.name()) {
            return Err(TypeError::DuplicateName(primitive.0.clone()).into());
        }
        self.primitives.push(primitive);
        Ok(())
    }

    /// Add a composite type.
    ///
    /// Validates the candidate before insertion:
    /// 1. Duplicate-name check across the whole dictionary.
    /// 2. Unknown-reference check (every `TypeRef` in the spec must already
    ///    exist in the dictionary).
    /// 3. Circular-definition check (DFS via [`check_circular`]).
    ///
    /// # Errors
    /// * [`TypeError::DuplicateName`] — name already in use.
    /// * [`TypeError::UnknownType`] — a member references an absent name.
    /// * [`TypeError::CircularDefinition`] — insertion would create a cycle.
    pub fn add_composite(&mut self, composite: CompositeType) -> Result<(), DictionaryError> {
        if self.contains(composite.name()) {
            return Err(TypeError::DuplicateName(composite.name.clone()).into());
        }
        // check_circular also validates that all referenced types exist.
        check_circular(&composite, self)?;
        self.composites.push(composite);
        Ok(())
    }

    /// Rename a type (primitive or composite).
    ///
    /// After renaming the type itself, patches every composite definition that
    /// references the old name in its spec entries.
    ///
    /// Returns the names of composites whose specs were patched (may be
    /// empty if nothing referenced the renamed type).
    ///
    /// # Errors
    /// * [`TypeError::DuplicateName`] — the new name is already in use.
    /// * [`DictionaryError::NotFound`] — the old name does not exist.
    pub fn rename(
        &mut self,
        old_name: &str,
        new_name: &str,
    ) -> Result<Vec<String>, DictionaryError> {
        if !self.contains(old_name) {
            return Err(DictionaryError::NotFound(old_name.to_owned()));
        }
        if old_name != new_name && self.contains(new_name) {
            return Err(TypeError::DuplicateName(new_name.to_owned()).into());
        }
        if old_name == new_name {
            return Ok(vec![]);
        }

        // Rename the type itself.
        if let Some(p) = self.primitives.iter_mut().find(|p| p.name() == old_name) {
            p.0 = new_name.to_owned();
        } else if let Some(c) = self.composites.iter_mut().find(|c| c.name() == old_name) {
            c.name = new_name.to_owned();
        }

        // Patch all composite specs that reference the old name.
        let mut patched: Vec<String> = Vec::new();
        for composite in &mut self.composites {
            // Skip the just-renamed composite itself — it's already updated.
            if composite.name == new_name {
                continue;
            }
            let mut changed = false;
            for entry in &mut composite.spec.entries {
                if entry.type_ref == old_name {
                    entry.type_ref = new_name.to_owned();
                    changed = true;
                }
            }
            if changed {
                patched.push(composite.name.clone());
            }
        }
        Ok(patched)
    }

    /// Delete a type by name.
    ///
    /// The dictionary does **not** track diagram references.  Callers are
    /// responsible for placing any referencing nodes or streams into an error
    /// state after a successful delete.
    ///
    /// Returns the name of the deleted type on success.
    ///
    /// # Errors
    /// Returns [`DictionaryError::NotFound`] if the name does not exist.
    pub fn delete(&mut self, name: &str) -> Result<String, DictionaryError> {
        // Try primitives first.
        if let Some(pos) = self.primitives.iter().position(|p| p.name() == name) {
            let deleted = self.primitives.remove(pos);
            return Ok(deleted.0);
        }
        // Try composites.
        if let Some(pos) = self.composites.iter().position(|c| c.name() == name) {
            let deleted = self.composites.remove(pos);
            return Ok(deleted.name);
        }
        Err(DictionaryError::NotFound(name.to_owned()))
    }
}
