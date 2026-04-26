//! Validation orchestrator (spec §4.4).
//!
//! # Architecture
//!
//! [`ValidationCache`] is the single source of truth for validation state.
//! It stores a [`BalanceResult`] per component UUID and derives the
//! rolled-up [`ValidationState`] badge from it on demand.
//!
//! Validation is triggered in three ways:
//!
//! | Trigger | Method | Scope |
//! |---------|--------|-------|
//! | Any component edit | [`validate_incremental`] | Changed component + direct neighbours |
//! | Toolbar button | [`validate_full`] | Every node and stream in every diagram |
//! | Type deleted from dictionary | [`on_type_deleted`] | All referencing components → Error immediately |
//! | Type renamed in dictionary | [`on_type_renamed`] | All referencing components → re-validate |
//! | Node deleted | [`on_node_deleted`] | Connected streams → Error (dangling) immediately |
//!
//! # Ownership
//!
//! `ValidationCache` is owned by [`crate::app::AppState`] alongside
//! `Project`.  Methods receive `&Project` (and optionally a `diagram_id`) as
//! parameters rather than holding a reference, keeping borrow-checker
//! compatibility with the mutable `AppState`.
//!
//! # Component identity
//!
//! Both nodes and streams are identified by their [`Uuid`].  The cache does
//! not distinguish between them; callers look up UUIDs in the diagram they
//! are working with.
//!
//! [`validate_incremental`]: ValidationCache::validate_incremental
//! [`validate_full`]: ValidationCache::validate_full
//! [`on_type_deleted`]: ValidationCache::on_type_deleted
//! [`on_type_renamed`]: ValidationCache::on_type_renamed
//! [`on_node_deleted`]: ValidationCache::on_node_deleted

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::{
    model::{diagram::Diagram, project::Project, stream::Stream},
    validation::balance::{
        node_input_balance, node_output_balance, stream_balance, BalanceResult, Finding, Severity,
    },
};

// ---------------------------------------------------------------------------
// ValidationState  (rolled-up badge level)
// ---------------------------------------------------------------------------

/// The rolled-up validation badge shown on the canvas and tree pane.
///
/// Derived from the [`BalanceResult`] stored in the cache; computed on demand
/// by [`ValidationCache::state`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValidationState {
    /// No findings; component is clean.
    #[default]
    Ok,
    /// At least one warning finding; no errors.
    Warning,
    /// At least one error finding (takes precedence over warnings).
    Error,
}

impl ValidationState {
    /// Combine two states: `Error > Warning > Ok`.
    pub fn merge(self, other: ValidationState) -> ValidationState {
        match (self, other) {
            (ValidationState::Error, _) | (_, ValidationState::Error) => ValidationState::Error,
            (ValidationState::Warning, _) | (_, ValidationState::Warning) => {
                ValidationState::Warning
            }
            _ => ValidationState::Ok,
        }
    }

    fn from_result(result: &BalanceResult) -> ValidationState {
        match result {
            BalanceResult::Ok => ValidationState::Ok,
            BalanceResult::Warning(_) => ValidationState::Warning,
            BalanceResult::Error(_) => ValidationState::Error,
        }
    }
}

// ---------------------------------------------------------------------------
// ValidationCache
// ---------------------------------------------------------------------------

/// Project-wide store of validation results, keyed by component UUID.
///
/// Stores one [`BalanceResult`] per component (node or stream).  The canvas
/// and tree pane call [`state`](ValidationCache::state) to get the rolled-up
/// [`ValidationState`] badge and [`findings`](ValidationCache::findings) to
/// get the tooltip messages.
#[derive(Debug, Default)]
pub struct ValidationCache {
    /// The most-recent balance result for each component.
    results: HashMap<Uuid, BalanceResult>,
}

impl ValidationCache {
    // -----------------------------------------------------------------------
    // Read API (called every frame by the UI)
    // -----------------------------------------------------------------------

    /// Return the rolled-up [`ValidationState`] for a component.
    ///
    /// Returns [`ValidationState::Ok`] for components that have never been
    /// validated or have no findings.
    pub fn state(&self, id: Uuid) -> ValidationState {
        self.results
            .get(&id)
            .map(ValidationState::from_result)
            .unwrap_or_default()
    }

    /// Return the human-readable messages for a component (for UI tooltips).
    ///
    /// Returns an empty slice for `Ok` components or unknown IDs.
    pub fn messages(&self, id: Uuid) -> &[String] {
        self.results
            .get(&id)
            .map(BalanceResult::messages)
            .unwrap_or(&[])
    }

    /// Convert the stored result to [`Finding`]s (for compatibility with
    /// code that consumes the old `Vec<Finding>` API).
    pub fn findings(&self, id: Uuid) -> Vec<Finding> {
        match self.results.get(&id) {
            None => vec![],
            Some(r) => Finding::from_balance_result(id, r.clone()),
        }
    }

    /// Returns `true` if any component in the cache has an error.
    pub fn has_errors(&self) -> bool {
        self.results
            .values()
            .any(|r| matches!(r, BalanceResult::Error(_)))
    }

    /// Returns `true` if any component in the cache has a warning or error.
    pub fn has_warnings(&self) -> bool {
        self.results
            .values()
            .any(|r| !matches!(r, BalanceResult::Ok))
    }

    // -----------------------------------------------------------------------
    // Incremental validation (spec §4.4)
    // -----------------------------------------------------------------------

    /// Re-validate `changed_id` and all components directly connected to it.
    ///
    /// The validator scans all diagrams in `project` to locate `changed_id`,
    /// then validates:
    /// * If `changed_id` is a **node**: the node (input + output balance),
    ///   every stream touching it (stream balance), and every node at the
    ///   other end of those streams (their balance is now affected too).
    /// * If `changed_id` is a **stream**: the stream (stream balance) and its
    ///   source and target nodes.
    ///
    /// Components that cannot be located (e.g. just deleted) are cleared from
    /// the cache.
    pub fn validate_incremental(&mut self, changed_id: Uuid, project: &Project) {
        // Find the diagram containing changed_id.
        let Some((diagram_id, diagram)) = find_component(project, changed_id) else {
            // Component no longer exists — remove its stale entry.
            self.results.remove(&changed_id);
            return;
        };

        // Collect the set of component IDs to re-validate.
        let mut to_validate: HashSet<Uuid> = HashSet::new();
        to_validate.insert(changed_id);

        if diagram.nodes.contains_key(&changed_id) {
            // changed_id is a node — include all touching streams and their
            // other endpoints.
            for stream in diagram.streams_for_node(changed_id) {
                to_validate.insert(stream.id);
                to_validate.insert(stream.source_node_id);
                to_validate.insert(stream.target_node_id);
            }
        } else if let Some(stream) = diagram.streams.get(&changed_id) {
            // changed_id is a stream — include its endpoints.
            to_validate.insert(stream.source_node_id);
            to_validate.insert(stream.target_node_id);
        }

        // Re-validate every component in the set.
        for id in to_validate {
            self.validate_component(id, diagram, project);
        }
    }

    // -----------------------------------------------------------------------
    // Full validation (spec §4.4)
    // -----------------------------------------------------------------------

    /// Re-validate every node and stream in every diagram in the project.
    ///
    /// Called from the "Full Validate" toolbar button.  Clears all existing
    /// results first so stale entries for deleted components are not retained.
    pub fn validate_full(&mut self, project: &Project) {
        self.results.clear();

        for diagram in project.diagrams.values() {
            for &node_id in diagram.nodes.keys() {
                self.validate_component(node_id, diagram, project);
            }
            for &stream_id in diagram.streams.keys() {
                self.validate_component(stream_id, diagram, project);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Dictionary event hooks (spec §4.3, §2)
    // -----------------------------------------------------------------------

    /// Called immediately when a type is deleted from the data dictionary.
    ///
    /// Injects an `Error` result for every node and stream in `project` that
    /// references `deleted_type_name` in any of its specs.  This runs before
    /// the deletion is committed to the undo stack so the UI sees the error
    /// state instantly.
    ///
    /// Balance functions cannot be called here because the type is already
    /// absent from the dictionary; the error is injected directly.
    pub fn on_type_deleted(&mut self, deleted_type_name: &str, project: &Project) {
        for diagram in project.diagrams.values() {
            for node in diagram.nodes.values() {
                if spec_references(&node.input_spec, deleted_type_name)
                    || spec_references(&node.output_spec, deleted_type_name)
                {
                    self.inject_error(
                        node.id,
                        format!(
                            "type '{deleted_type_name}' was deleted from the \
                             data dictionary"
                        ),
                    );
                }
            }
            for stream in diagram.streams.values() {
                if spec_references(&stream.type_spec, deleted_type_name) {
                    self.inject_error(
                        stream.id,
                        format!(
                            "type '{deleted_type_name}' was deleted from the \
                             data dictionary"
                        ),
                    );
                }
            }
        }
    }

    /// Called after a type is renamed in the data dictionary.
    ///
    /// The rename has already been applied (dictionary entries updated) before
    /// this is called.  Re-validates every component that referenced the old
    /// name — they now reference the new name, so the results will usually
    /// improve (clear) unless the new name introduced a different problem.
    pub fn on_type_renamed(&mut self, _old_name: &str, new_name: &str, project: &Project) {
        // After renaming, all affected specs now carry `new_name`.  Collect
        // the diagram IDs and component IDs that need re-validation.
        let mut to_validate: Vec<(Uuid, Uuid)> = Vec::new(); // (diagram_id, component_id)

        for (diagram_id, diagram) in &project.diagrams {
            for node in diagram.nodes.values() {
                if spec_references(&node.input_spec, new_name)
                    || spec_references(&node.output_spec, new_name)
                {
                    to_validate.push((*diagram_id, node.id));
                }
            }
            for stream in diagram.streams.values() {
                if spec_references(&stream.type_spec, new_name) {
                    to_validate.push((*diagram_id, stream.id));
                }
            }
        }

        for (diagram_id, component_id) in to_validate {
            if let Some(diagram) = project.diagrams.get(&diagram_id) {
                self.validate_component(component_id, diagram, project);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Node deletion hook (spec §4.3)
    // -----------------------------------------------------------------------

    /// Called immediately when a node is deleted.
    ///
    /// Injects `Error` findings for every stream in `project` that was
    /// connected to `deleted_node_id`.  The streams themselves are not
    /// removed from the diagram — they remain as dangling streams with an
    /// error badge.
    ///
    /// `dangled_stream_ids` is the list returned by
    /// [`Diagram::remove_node`](crate::model::diagram::Diagram::remove_node).
    pub fn on_node_deleted(&mut self, deleted_node_id: Uuid, dangled_stream_ids: &[Uuid]) {
        // Clear the deleted node's own entry.
        self.results.remove(&deleted_node_id);

        for &stream_id in dangled_stream_ids {
            self.inject_error(
                stream_id,
                format!("stream is dangling: connected node {deleted_node_id} was deleted"),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Misc
    // -----------------------------------------------------------------------

    /// Clear all cached results (used when loading a new project).
    pub fn clear_all(&mut self) {
        self.results.clear();
    }

    /// Remove the cached result for a single component (used when a component
    /// is deleted and should no longer appear in the cache).
    pub fn remove(&mut self, id: Uuid) {
        self.results.remove(&id);
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Validate a single component and store the result.
    ///
    /// Dispatches to the appropriate balance function(s) based on whether
    /// `id` is a node or a stream.  Dangling streams are detected and reported
    /// as errors immediately, bypassing the balance functions.
    fn validate_component(&mut self, id: Uuid, diagram: &Diagram, project: &Project) {
        let dict = &project.data_dictionary;

        if let Some(node) = diagram.nodes.get(&id) {
            // Nodes: run input balance + output balance, merge results.
            let incoming: Vec<&Stream> = diagram.incoming_streams(id).collect();
            let outgoing: Vec<&Stream> = diagram.outgoing_streams(id).collect();

            let input_result = node_input_balance(node, &incoming, dict);
            let output_result = node_output_balance(node, &outgoing, dict);
            let merged = input_result.merge(output_result);
            self.results.insert(id, merged);
        } else if let Some(stream) = diagram.streams.get(&id) {
            // Dangling streams bypass balance checks entirely.
            if stream.is_dangling {
                self.inject_error(
                    id,
                    "stream is dangling: one or both endpoint nodes have been deleted".to_owned(),
                );
                return;
            }

            // Locate source and target nodes.
            let src = diagram.nodes.get(&stream.source_node_id);
            let tgt = diagram.nodes.get(&stream.target_node_id);

            let result = match (src, tgt) {
                (Some(s), Some(t)) => stream_balance(stream, s, t, dict),
                (None, _) => BalanceResult::Error(vec![format!(
                    "stream source node {} not found in diagram",
                    stream.source_node_id
                )]),
                (_, None) => BalanceResult::Error(vec![format!(
                    "stream target node {} not found in diagram",
                    stream.target_node_id
                )]),
            };
            self.results.insert(id, result);
        } else {
            // Component not found — remove stale entry if any.
            self.results.remove(&id);
        }
    }

    /// Inject a single error message for `id`, merging with any existing
    /// result at that ID.
    fn inject_error(&mut self, id: Uuid, message: String) {
        let existing = self.results.remove(&id).unwrap_or(BalanceResult::Ok);
        let new_error = BalanceResult::Error(vec![message]);
        self.results.insert(id, existing.merge(new_error));
    }
}

// ---------------------------------------------------------------------------
// Module-level helpers
// ---------------------------------------------------------------------------

/// Find the diagram that contains `component_id` (as a node or stream) and
/// return `(diagram_id, &Diagram)`.  Returns `None` if not found in any
/// diagram.
fn find_component(project: &Project, component_id: Uuid) -> Option<(Uuid, &Diagram)> {
    project.diagrams.iter().find_map(|(&did, diagram)| {
        if diagram.nodes.contains_key(&component_id) || diagram.streams.contains_key(&component_id)
        {
            Some((did, diagram))
        } else {
            None
        }
    })
}

/// Returns `true` if any entry in `spec` has `type_ref == name`.
///
/// This is a direct string match on the `type_ref` field — it catches
/// only *direct* references.  Indirect references (a composite that
/// *contains* the named type) are not detected here; re-flattening after
/// the mutation will surface those as `UnknownType` errors naturally.
fn spec_references(spec: &crate::model::types::TypeSpec, name: &str) -> bool {
    spec.entries.iter().any(|e| e.type_ref == name)
}
