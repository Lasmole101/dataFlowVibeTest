//! XML schema constants and version information.
//!
//! The schema version is written as an attribute on the root `<project>`
//! element.  When loading, the version is checked so that future migration
//! paths can be added without breaking existing files.

/// Current XML schema version.  Increment when making breaking changes to the
/// element structure.
pub const SCHEMA_VERSION: u32 = 1;

/// Root element name.
pub const EL_PROJECT: &str = "project";

/// Data dictionary container element.
pub const EL_DICTIONARY: &str = "dictionary";

/// A primitive type element.
pub const EL_PRIMITIVE: &str = "primitive";

/// A composite type element.
pub const EL_COMPOSITE: &str = "composite";

/// A type entry inside a spec or composite (count + type_ref).
pub const EL_ENTRY: &str = "entry";

/// Diagrams container element.
pub const EL_DIAGRAMS: &str = "diagrams";

/// A single diagram element.
pub const EL_DIAGRAM: &str = "diagram";

/// Orphaned diagrams container element.
pub const EL_ORPHANED: &str = "orphaned";

/// Node element.
pub const EL_NODE: &str = "node";

/// Stream element.
pub const EL_STREAM: &str = "stream";

/// Stream group element.
pub const EL_GROUP: &str = "group";

/// Input specification container element.
pub const EL_INPUT_SPEC: &str = "input_spec";

/// Output specification container element.
pub const EL_OUTPUT_SPEC: &str = "output_spec";

/// Type specification container element (used for streams).
pub const EL_TYPE_SPEC: &str = "type_spec";

/// Viewport element.
pub const EL_VIEWPORT: &str = "viewport";

/// `schema_version` attribute name.
pub const ATTR_VERSION: &str = "schema_version";

/// `id` attribute name.
pub const ATTR_ID: &str = "id";

/// `name` attribute name.
pub const ATTR_NAME: &str = "name";

/// `kind` attribute name (Source / Sink / Process).
pub const ATTR_KIND: &str = "kind";

/// `is_boundary` attribute name.
pub const ATTR_IS_BOUNDARY: &str = "is_boundary";

/// `sub_diagram_id` attribute name.
pub const ATTR_SUB_DIAGRAM_ID: &str = "sub_diagram_id";

/// `source_node_id` attribute name.
pub const ATTR_SOURCE_NODE_ID: &str = "source_node_id";

/// `target_node_id` attribute name.
pub const ATTR_TARGET_NODE_ID: &str = "target_node_id";

/// `group_id` attribute name.
pub const ATTR_GROUP_ID: &str = "group_id";

/// `is_dangling` attribute name.
pub const ATTR_IS_DANGLING: &str = "is_dangling";

/// `count` attribute name (inside a type entry).
pub const ATTR_COUNT: &str = "count";

/// `type_ref` attribute name (inside a type entry).
pub const ATTR_TYPE_REF: &str = "type_ref";

/// `x` position attribute name.
pub const ATTR_X: &str = "x";

/// `y` position attribute name.
pub const ATTR_Y: &str = "y";

/// `zoom` attribute name.
pub const ATTR_ZOOM: &str = "zoom";

/// `scroll_x` attribute name.
pub const ATTR_SCROLL_X: &str = "scroll_x";

/// `scroll_y` attribute name.
pub const ATTR_SCROLL_Y: &str = "scroll_y";

/// `parent_process_id` attribute name.
pub const ATTR_PARENT_PROCESS_ID: &str = "parent_process_id";
