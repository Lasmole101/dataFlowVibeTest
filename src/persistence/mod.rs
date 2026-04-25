//! File persistence subsystem (spec §7).
//!
//! Projects are saved and loaded as XML.  The schema version is embedded in
//! the root element so that future migrations can be detected.

pub mod schema;
pub mod xml;
