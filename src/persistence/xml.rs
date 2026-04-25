//! XML serialisation and deserialisation for [`Project`] (spec §7).
//!
//! Uses [`quick_xml`] for low-level XML writing and reading.  The schema
//! version is checked on load; an unsupported version returns an error rather
//! than silently misreading the file.

use thiserror::Error;

use crate::model::project::Project;

// ---------------------------------------------------------------------------
// PersistenceError
// ---------------------------------------------------------------------------

/// Errors that can arise during file I/O.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// Low-level I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// XML parse or write error.
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),

    /// The file's schema version is not supported by this build.
    #[error("unsupported schema version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },

    /// A required attribute or element was missing or malformed.
    #[error("malformed project file: {0}")]
    Malformed(String),

    /// A UUID in the file could not be parsed.
    #[error("invalid UUID '{0}': {1}")]
    InvalidUuid(String, uuid::Error),
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Serialise `project` to an XML string.
pub fn save_project(project: &Project) -> Result<String, PersistenceError> {
    todo!("Phase 5 — xml::save_project")
}

/// Serialise `project` and write it to the file at `path`.
pub fn save_project_to_file(
    project: &Project,
    path: &std::path::Path,
) -> Result<(), PersistenceError> {
    let xml = save_project(project)?;
    std::fs::write(path, xml).map_err(PersistenceError::Io)
}

/// Parse an XML string and reconstruct a [`Project`].
pub fn load_project(xml: &str) -> Result<Project, PersistenceError> {
    todo!("Phase 5 — xml::load_project")
}

/// Read the file at `path` and parse it as a project.
pub fn load_project_from_file(path: &std::path::Path) -> Result<Project, PersistenceError> {
    let xml = std::fs::read_to_string(path).map_err(PersistenceError::Io)?;
    load_project(&xml)
}
