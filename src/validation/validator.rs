//! Validation orchestrator (spec §4.4).
//!
//! [`ValidationCache`] stores the most-recent findings for every component in
//! the project.  The canvas and tree pane read this cache to display overlaid
//! error/warning badges without re-running validation on every paint.
//!
//! Validation is triggered in two ways:
//! * **Incremental** — called automatically after any mutation; validates only
//!   the changed component and its direct neighbours.
//! * **Full** — called from the toolbar "Full Validate" button; validates
//!   every component in the project.

use std::collections::HashMap;

use uuid::Uuid;

use crate::{
    model::project::Project,
    validation::balance::{Finding, Severity},
};

// ---------------------------------------------------------------------------
// ValidationState
// ---------------------------------------------------------------------------

/// The rolled-up validation state for a single component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationState {
    /// No findings; component is clean.
    #[default]
    Ok,
    /// At least one warning finding.
    Warning,
    /// At least one error finding (takes precedence over Warning).
    Error,
}

impl ValidationState {
    /// Combine two states: Error > Warning > Ok.
    pub fn merge(self, other: ValidationState) -> ValidationState {
        match (self, other) {
            (ValidationState::Error, _) | (_, ValidationState::Error) => ValidationState::Error,
            (ValidationState::Warning, _) | (_, ValidationState::Warning) => {
                ValidationState::Warning
            }
            _ => ValidationState::Ok,
        }
    }
}

// ---------------------------------------------------------------------------
// ValidationCache
// ---------------------------------------------------------------------------

/// Project-wide store of validation findings, keyed by component ID.
#[derive(Debug, Default)]
pub struct ValidationCache {
    /// Raw findings per component ID.
    findings: HashMap<Uuid, Vec<Finding>>,
    /// Rolled-up state per component ID (derived from `findings`).
    states: HashMap<Uuid, ValidationState>,
}

impl ValidationCache {
    /// Return the rolled-up [`ValidationState`] for a component.
    /// Returns [`ValidationState::Ok`] if the component has no findings.
    pub fn state(&self, id: Uuid) -> ValidationState {
        self.states.get(&id).copied().unwrap_or_default()
    }

    /// Return all raw findings for a component.
    pub fn findings(&self, id: Uuid) -> &[Finding] {
        self.findings.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Run incremental validation: re-validate `changed_id` and all
    /// components directly connected to it in the active diagram.
    pub fn validate_incremental(&mut self, changed_id: Uuid, project: &Project, diagram_id: Uuid) {
        todo!("Phase 3 — ValidationCache::validate_incremental")
    }

    /// Run full validation across the entire project.
    pub fn validate_full(&mut self, project: &Project) {
        todo!("Phase 3 — ValidationCache::validate_full")
    }

    /// Clear all findings for a specific component (used before re-validating
    /// it).
    fn clear(&mut self, id: Uuid) {
        self.findings.remove(&id);
        self.states.remove(&id);
    }

    /// Insert findings for a component and recompute its rolled-up state.
    fn insert(&mut self, id: Uuid, new_findings: Vec<Finding>) {
        let state = new_findings.iter().fold(ValidationState::Ok, |acc, f| {
            acc.merge(match f.severity {
                Severity::Warning => ValidationState::Warning,
                Severity::Error => ValidationState::Error,
            })
        });
        self.findings.insert(id, new_findings);
        self.states.insert(id, state);
    }
}
