//! Project model — the root of the entire document (spec §7).
//!
//! A [`Project`] owns the [`DataDictionary`] and the full tree of
//! [`Diagram`]s.  It is the unit of file persistence: one project = one XML
//! file.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{diagram::Diagram, dictionary::DataDictionary};

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

/// The root document object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Shared type dictionary for all diagrams.
    pub data_dictionary: DataDictionary,
    /// ID of the top-level diagram (always present).
    pub root_diagram_id: Uuid,
    /// All diagrams in this project, keyed by their ID.
    /// Includes the root, all sub-diagrams, and orphaned diagrams.
    pub diagrams: HashMap<Uuid, Diagram>,
    /// IDs of sub-diagrams whose parent process node has been deleted
    /// (spec §6.4).  Displayed under the "Orphaned" directory in the tree pane.
    pub orphaned_diagram_ids: Vec<Uuid>,
}

impl Project {
    /// Create a new project with an empty data dictionary and a blank root
    /// diagram.
    pub fn new() -> Self {
        let root = Diagram::new("Root");
        let root_id = root.id;
        let mut diagrams = HashMap::new();
        diagrams.insert(root_id, root);

        Self {
            data_dictionary: DataDictionary::new(),
            root_diagram_id: root_id,
            diagrams,
            orphaned_diagram_ids: Vec::new(),
        }
    }

    /// Look up a diagram by ID.
    pub fn diagram(&self, id: Uuid) -> Option<&Diagram> {
        self.diagrams.get(&id)
    }

    /// Look up a diagram mutably by ID.
    pub fn diagram_mut(&mut self, id: Uuid) -> Option<&mut Diagram> {
        self.diagrams.get_mut(&id)
    }

    /// Move a sub-diagram to the orphaned list (called when its parent process
    /// node is deleted).
    pub fn orphan_diagram(&mut self, diagram_id: Uuid) {
        todo!("Phase 2 — Project::orphan_diagram")
    }

    /// Attach an orphaned sub-diagram to a new parent process node.
    /// Returns an error string if the diagram ID is unknown or not orphaned.
    pub fn attach_diagram(
        &mut self,
        diagram_id: Uuid,
        new_parent_process_id: Uuid,
    ) -> Result<(), String> {
        todo!("Phase 2 — Project::attach_diagram")
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}
