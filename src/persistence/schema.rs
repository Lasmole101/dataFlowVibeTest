//! XML schema constants and the authoritative schema reference document.
//!
//! # Schema reference
//!
//! The annotated example below is the **single source of truth** for the
//! file format.  The serialiser (`xml.rs`) must produce documents that
//! conform to this structure; the deserialiser must accept them.
//!
//! Every element name and attribute name used by the serialiser/deserialiser
//! is declared as a `pub const` in this module.  The serialiser and
//! deserialiser import only from here — no string literals elsewhere.
//!
//! ## Design decisions
//!
//! * **Flat diagram list, not nested tree.** All diagrams (root, sub-diagrams,
//!   orphaned) are stored as siblings inside `<diagrams>`.  Each `<diagram>`
//!   carries a `parent_process_id` attribute (omitted for root and orphaned).
//!   The reader reconstructs the logical tree from those pointers.  This
//!   avoids recursive serialisation and matches the in-memory representation.
//!
//! * **Optional attributes are omitted when absent.**  `sub_diagram_id`,
//!   `parent_process_id`, and stream `group_id` are not written when `None`.
//!   The deserialiser treats absence as `None`.  Boolean flags that default
//!   to `false` (e.g. `is_boundary`, `is_dangling`) are omitted when false.
//!
//! * **Positions are per-node attributes** inside each `<node>` element,
//!   not a separate `<positions>` section.  This keeps all node data in one
//!   place and avoids ID-keyed lookup during deserialisation.
//!
//! * **Stream group members are stored as `<member>` children** of `<group>`,
//!   not as a flat attribute.  This avoids comma-delimited UUID lists and
//!   makes the format more readable and extensible.
//!
//! * **TypeSpec entries are `<entry>` children** of their parent element
//!   (`<input_spec>`, `<output_spec>`, `<type_spec>`, or `<composite>`).
//!   Each carries `count` and `type_ref` attributes.
//!
//! * **Orphaned diagram IDs** are recorded as `<ref>` children of an
//!   `<orphaned>` element inside `<project>`, separate from the `<diagrams>`
//!   list.  This makes the orphaned set explicit without relying on the
//!   absence of a `parent_process_id`.
//!
//! ## Annotated example document
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <project schema_version="1"
//!          root_diagram_id="11111111-1111-1111-1111-111111111111">
//!
//!   <!-- ═══════════════════════════════════════════════════════════
//!        DATA DICTIONARY
//!        All primitive and composite type definitions.
//!        Shared by every diagram in the project.
//!        ═══════════════════════════════════════════════════════════ -->
//!   <dictionary>
//!
//!     <!-- Primitive types: name is the only field. -->
//!     <primitive name="uint32_t"/>
//!     <primitive name="char"/>
//!     <primitive name="float"/>
//!
//!     <!-- Composite types: one <entry> child per multiset member.
//!          count="N" means N instances of type_ref per composite instance.
//!          Members may reference other composites (but not cyclically). -->
//!     <composite name="MyPacket">
//!       <entry count="3" type_ref="uint32_t"/>
//!       <entry count="2" type_ref="char"/>
//!     </composite>
//!
//!     <composite name="Frame">
//!       <entry count="1" type_ref="MyPacket"/>
//!       <entry count="1" type_ref="float"/>
//!     </composite>
//!
//!   </dictionary>
//!
//!   <!-- ═══════════════════════════════════════════════════════════
//!        DIAGRAMS
//!        Flat list of all diagrams: root, sub-diagrams, and orphaned.
//!        The tree structure is reconstructed from parent_process_id.
//!        ═══════════════════════════════════════════════════════════ -->
//!   <diagrams>
//!
//!     <!-- Root diagram: no parent_process_id attribute. -->
//!     <diagram id="11111111-1111-1111-1111-111111111111"
//!              name="Root">
//!
//!       <viewport zoom="1.0" scroll_x="0.0" scroll_y="0.0"/>
//!
//!       <nodes>
//!
//!         <!-- Source node: kind="Source", no input_spec children.
//!              x and y are canvas-space coordinates. -->
//!         <node id="aaaa0001-0000-0000-0000-000000000000"
//!               kind="Source"
//!               name="Sensor"
//!               x="40.0" y="100.0">
//!           <!-- Sources have no input_spec. -->
//!           <output_spec>
//!             <entry count="1" type_ref="MyPacket"/>
//!           </output_spec>
//!         </node>
//!
//!         <!-- Process node: kind="Process", has both specs.
//!              sub_diagram_id links to a child diagram (omitted if none). -->
//!         <node id="aaaa0002-0000-0000-0000-000000000000"
//!               kind="Process"
//!               name="Filter"
//!               x="400.0" y="100.0"
//!               sub_diagram_id="22222222-2222-2222-2222-222222222222">
//!           <input_spec>
//!             <entry count="1" type_ref="MyPacket"/>
//!           </input_spec>
//!           <output_spec>
//!             <entry count="1" type_ref="Frame"/>
//!           </output_spec>
//!         </node>
//!
//!         <!-- Sink node: kind="Sink", no output_spec children. -->
//!         <node id="aaaa0003-0000-0000-0000-000000000000"
//!               kind="Sink"
//!               name="Logger"
//!               x="760.0" y="100.0">
//!           <input_spec>
//!             <entry count="1" type_ref="Frame"/>
//!           </input_spec>
//!           <!-- Sinks have no output_spec. -->
//!         </node>
//!
//!       </nodes>
//!
//!       <streams>
//!
//!         <!-- Stream: source_node_id → target_node_id.
//!              group_id is omitted when the stream has no group.
//!              is_dangling is omitted when false. -->
//!         <stream id="bbbb0001-0000-0000-0000-000000000000"
//!                 source_node_id="aaaa0001-0000-0000-0000-000000000000"
//!                 target_node_id="aaaa0002-0000-0000-0000-000000000000"
//!                 group_id="cccc0001-0000-0000-0000-000000000000">
//!           <type_spec>
//!             <entry count="1" type_ref="MyPacket"/>
//!           </type_spec>
//!         </stream>
//!
//!         <!-- Dangling stream: source was deleted.
//!              is_dangling="true" is written only when true. -->
//!         <stream id="bbbb0002-0000-0000-0000-000000000000"
//!                 source_node_id="aaaa0002-0000-0000-0000-000000000000"
//!                 target_node_id="aaaa0003-0000-0000-0000-000000000000"
//!                 is_dangling="true">
//!           <type_spec>
//!             <entry count="1" type_ref="Frame"/>
//!           </type_spec>
//!         </stream>
//!
//!       </streams>
//!
//!       <groups>
//!
//!         <!-- Stream group: display-only; members listed as <member> children. -->
//!         <group id="cccc0001-0000-0000-0000-000000000000"
//!                name="DataBus">
//!           <member stream_id="bbbb0001-0000-0000-0000-000000000000"/>
//!         </group>
//!
//!       </groups>
//!
//!     </diagram>
//!
//!     <!-- Sub-diagram: parent_process_id links back to the process node
//!          that owns this diagram.  Boundary nodes carry is_boundary="true". -->
//!     <diagram id="22222222-2222-2222-2222-222222222222"
//!              name="Filter (sub)"
//!              parent_process_id="aaaa0002-0000-0000-0000-000000000000">
//!
//!       <viewport zoom="1.5" scroll_x="-100.0" scroll_y="0.0"/>
//!
//!       <nodes>
//!
//!         <!-- Locked boundary source: is_boundary="true".
//!              Cannot be renamed or deleted by the user.
//!              Carries the parent process input_spec as its output_spec. -->
//!         <node id="dddd0001-0000-0000-0000-000000000000"
//!               kind="Source"
//!               name="in_0"
//!               x="40.0" y="80.0"
//!               is_boundary="true">
//!           <output_spec>
//!             <entry count="1" type_ref="MyPacket"/>
//!           </output_spec>
//!         </node>
//!
//!         <!-- Internal process node inside the sub-diagram. -->
//!         <node id="dddd0002-0000-0000-0000-000000000000"
//!               kind="Process"
//!               name="Unpack"
//!               x="400.0" y="80.0">
//!           <input_spec>
//!             <entry count="1" type_ref="MyPacket"/>
//!           </input_spec>
//!           <output_spec>
//!             <entry count="1" type_ref="Frame"/>
//!           </output_spec>
//!         </node>
//!
//!         <!-- Locked boundary sink: carries the parent process output_spec
//!              as its input_spec. -->
//!         <node id="dddd0003-0000-0000-0000-000000000000"
//!               kind="Sink"
//!               name="out_0"
//!               x="800.0" y="80.0"
//!               is_boundary="true">
//!           <input_spec>
//!             <entry count="1" type_ref="Frame"/>
//!           </input_spec>
//!         </node>
//!
//!       </nodes>
//!
//!       <!-- Empty containers are written as self-closing elements. -->
//!       <streams/>
//!       <groups/>
//!
//!     </diagram>
//!
//!     <!-- Orphaned sub-diagram: no parent_process_id attribute.
//!          Listed in <orphaned> below; stored here for completeness. -->
//!     <diagram id="33333333-3333-3333-3333-333333333333"
//!              name="OldProcess (sub)">
//!
//!       <viewport zoom="1.0" scroll_x="0.0" scroll_y="0.0"/>
//!       <nodes/>
//!       <streams/>
//!       <groups/>
//!
//!     </diagram>
//!
//!   </diagrams>
//!
//!   <!-- ═══════════════════════════════════════════════════════════
//!        ORPHANED DIAGRAM REFERENCES
//!        IDs of sub-diagrams whose parent process has been deleted.
//!        These diagrams are also present in <diagrams> above.
//!        ═══════════════════════════════════════════════════════════ -->
//!   <orphaned>
//!     <ref id="33333333-3333-3333-3333-333333333333"/>
//!   </orphaned>
//!
//! </project>
//! ```
//!
//! ## Minimal valid document (empty project)
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <project schema_version="1"
//!          root_diagram_id="ffffffff-ffff-ffff-ffff-ffffffffffff">
//!   <dictionary/>
//!   <diagrams>
//!     <diagram id="ffffffff-ffff-ffff-ffff-ffffffffffff" name="Root">
//!       <viewport zoom="1.0" scroll_x="0.0" scroll_y="0.0"/>
//!       <nodes/>
//!       <streams/>
//!       <groups/>
//!     </diagram>
//!   </diagrams>
//!   <orphaned/>
//! </project>
//! ```

// ---------------------------------------------------------------------------
// Schema version
// ---------------------------------------------------------------------------

/// Current XML schema version.
///
/// Written as the `schema_version` attribute on the root `<project>` element.
/// Increment when making breaking changes to element structure or attribute
/// semantics.  The deserialiser rejects files with an unsupported version.
pub const SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Element names
// ---------------------------------------------------------------------------

/// `<project>` — root element.
pub const EL_PROJECT: &str = "project";

/// `<dictionary>` — contains all primitive and composite type definitions.
pub const EL_DICTIONARY: &str = "dictionary";

/// `<primitive>` — a named leaf type; carries only a `name` attribute.
pub const EL_PRIMITIVE: &str = "primitive";

/// `<composite>` — a named composite type; carries a `name` attribute and
/// one or more `<entry>` children.
pub const EL_COMPOSITE: &str = "composite";

/// `<entry>` — one multiset entry inside a type spec or composite definition.
/// Carries `count` and `type_ref` attributes.
pub const EL_ENTRY: &str = "entry";

/// `<diagrams>` — flat container for all diagram elements (root,
/// sub-diagrams, and orphaned diagrams).
pub const EL_DIAGRAMS: &str = "diagrams";

/// `<diagram>` — a single canvas (root or sub or orphaned).
pub const EL_DIAGRAM: &str = "diagram";

/// `<orphaned>` — container of `<ref>` elements listing the IDs of
/// sub-diagrams whose parent process has been deleted.
pub const EL_ORPHANED: &str = "orphaned";

/// `<ref>` — a bare UUID reference inside `<orphaned>`.
/// Carries a single `id` attribute.
pub const EL_REF: &str = "ref";

/// `<nodes>` — container for all `<node>` elements within a `<diagram>`.
pub const EL_NODES: &str = "nodes";

/// `<node>` — a single node on the canvas.
pub const EL_NODE: &str = "node";

/// `<streams>` — container for all `<stream>` elements within a `<diagram>`.
pub const EL_STREAMS: &str = "streams";

/// `<stream>` — a directed connection between two nodes.
pub const EL_STREAM: &str = "stream";

/// `<groups>` — container for all `<group>` elements within a `<diagram>`.
pub const EL_GROUPS: &str = "groups";

/// `<group>` — a display-only stream group; contains `<member>` children.
pub const EL_GROUP: &str = "group";

/// `<member>` — a stream membership record inside a `<group>`.
/// Carries a `stream_id` attribute.
pub const EL_MEMBER: &str = "member";

/// `<input_spec>` — the input type specification of a node.
/// Contains zero or more `<entry>` children.
pub const EL_INPUT_SPEC: &str = "input_spec";

/// `<output_spec>` — the output type specification of a node.
/// Contains zero or more `<entry>` children.
pub const EL_OUTPUT_SPEC: &str = "output_spec";

/// `<type_spec>` — the type specification of a stream.
/// Contains zero or more `<entry>` children.
pub const EL_TYPE_SPEC: &str = "type_spec";

/// `<viewport>` — saved camera state for a diagram.
/// Carries `zoom`, `scroll_x`, and `scroll_y` attributes.
pub const EL_VIEWPORT: &str = "viewport";

// ---------------------------------------------------------------------------
// Attribute names
// ---------------------------------------------------------------------------

/// `schema_version` — integer schema version on `<project>`.
pub const ATTR_SCHEMA_VERSION: &str = "schema_version";

/// `root_diagram_id` — UUID of the root diagram on `<project>`.
pub const ATTR_ROOT_DIAGRAM_ID: &str = "root_diagram_id";

/// `id` — stable UUID on `<diagram>`, `<node>`, `<stream>`, `<group>`,
/// and `<ref>`.
pub const ATTR_ID: &str = "id";

/// `name` — display name on `<diagram>`, `<node>`, `<primitive>`,
/// `<composite>`, and `<group>`.
pub const ATTR_NAME: &str = "name";

/// `parent_process_id` — UUID of the process node that owns a sub-diagram.
/// Omitted for the root diagram and orphaned diagrams.
pub const ATTR_PARENT_PROCESS_ID: &str = "parent_process_id";

/// `kind` — node kind string on `<node>`.  One of the `NODE_KIND_*` constants.
pub const ATTR_KIND: &str = "kind";

/// `x` — horizontal canvas position of a node (f32, canvas units).
pub const ATTR_X: &str = "x";

/// `y` — vertical canvas position of a node (f32, canvas units).
pub const ATTR_Y: &str = "y";

/// `is_boundary` — `"true"` when the node is a locked boundary node.
/// Omitted (treated as `false`) when absent.
pub const ATTR_IS_BOUNDARY: &str = "is_boundary";

/// `sub_diagram_id` — UUID of the sub-diagram owned by a Process node.
/// Omitted when the process has no sub-diagram.
pub const ATTR_SUB_DIAGRAM_ID: &str = "sub_diagram_id";

/// `source_node_id` — UUID of the stream's source node on `<stream>`.
pub const ATTR_SOURCE_NODE_ID: &str = "source_node_id";

/// `target_node_id` — UUID of the stream's target node on `<stream>`.
pub const ATTR_TARGET_NODE_ID: &str = "target_node_id";

/// `group_id` — UUID of the display group this stream belongs to.
/// Omitted when the stream has no group.
pub const ATTR_GROUP_ID: &str = "group_id";

/// `is_dangling` — `"true"` when a stream's source or target node has been
/// deleted.  Omitted (treated as `false`) when absent.
pub const ATTR_IS_DANGLING: &str = "is_dangling";

/// `stream_id` — UUID reference on a `<member>` element inside a `<group>`.
pub const ATTR_STREAM_ID: &str = "stream_id";

/// `color` — hex RGB color (e.g. `"ff8040"`) assigned to a stream group.
/// Omitted for old files; readers fall back to a default grey.
pub const ATTR_COLOR: &str = "color";

/// `count` — instance count on an `<entry>` element (positive integer).
pub const ATTR_COUNT: &str = "count";

/// `type_ref` — type name reference on an `<entry>` element.
pub const ATTR_TYPE_REF: &str = "type_ref";

/// `zoom` — zoom factor on `<viewport>` (f32; 1.0 = 100%).
pub const ATTR_ZOOM: &str = "zoom";

/// `scroll_x` — horizontal scroll offset on `<viewport>` (f32, canvas units).
pub const ATTR_SCROLL_X: &str = "scroll_x";

/// `scroll_y` — vertical scroll offset on `<viewport>` (f32, canvas units).
pub const ATTR_SCROLL_Y: &str = "scroll_y";

// ---------------------------------------------------------------------------
// NodeKind serialisation values
// ---------------------------------------------------------------------------

/// Serialised string for [`crate::model::node::NodeKind::Source`].
pub const NODE_KIND_SOURCE: &str = "Source";

/// Serialised string for [`crate::model::node::NodeKind::Sink`].
pub const NODE_KIND_SINK: &str = "Sink";

/// Serialised string for [`crate::model::node::NodeKind::Process`].
pub const NODE_KIND_PROCESS: &str = "Process";

// ---------------------------------------------------------------------------
// Boolean serialisation values
// ---------------------------------------------------------------------------

/// Serialised `true` value for boolean attributes (`is_boundary`, `is_dangling`).
pub const BOOL_TRUE: &str = "true";

/// Serialised `false` value — not written; absence implies false.
/// Defined for documentation clarity and use in deserialiser comparisons.
pub const BOOL_FALSE: &str = "false";
