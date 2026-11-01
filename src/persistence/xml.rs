//! XML serialisation and deserialisation for [`Project`] (spec §7).
//!
//! Uses [`quick_xml`]'s event-based API directly — not the serde derive —
//! because the schema requires precise control over attribute ordering,
//! optional attribute omission, and the flat-diagram-list structure.
//!
//! # Public API
//!
//! | Function | Purpose |
//! |---|---|
//! | [`save_project`] | `Project` → UTF-8 XML `String` |
//! | [`save_project_to_file`] | `Project` → XML file |
//! | [`load_project`] | XML `str` → `Project` |
//! | [`load_project_from_file`] | XML file → `Project` |
//!
//! # Error handling
//!
//! All errors are returned as [`PersistenceError`].  No panics on malformed
//! input — every parse failure produces `Malformed(reason)`.

use std::collections::HashMap;
use std::io::Write;

use quick_xml::{
    events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event},
    Reader, Writer,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    model::{
        diagram::{Diagram, ViewportState},
        dictionary::DataDictionary,
        node::{Node, NodeKind},
        project::Project,
        stream::{Stream, StreamGroup},
        types::{CompositeType, PrimitiveType, TypeEntry, TypeSpec},
    },
    persistence::schema::*,
};

// ---------------------------------------------------------------------------
// PersistenceError
// ---------------------------------------------------------------------------

/// Errors that can arise during file I/O or XML parsing.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// Low-level I/O failure (file read/write).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// XML syntax error from quick-xml.
    #[error("XML error: {0}")]
    Xml(#[from] quick_xml::Error),

    /// XML attribute parsing error (separate type in quick-xml ≥ 0.31).
    #[error("XML attribute error: {0}")]
    Attr(#[from] quick_xml::events::attributes::AttrError),

    /// The file's schema version is not supported by this build.
    #[error("unsupported schema version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },

    /// A required attribute or element was missing or malformed.
    #[error("malformed project file: {0}")]
    Malformed(String),

    /// A UUID attribute could not be parsed.
    #[error("invalid UUID '{raw}': {source}")]
    InvalidUuid {
        raw: String,
        #[source]
        source: uuid::Error,
    },

    /// An integer attribute could not be parsed.
    #[error("invalid integer '{raw}': {source}")]
    InvalidInt {
        raw: String,
        #[source]
        source: std::num::ParseIntError,
    },

    /// A float attribute could not be parsed.
    #[error("invalid float '{raw}': {source}")]
    InvalidFloat {
        raw: String,
        #[source]
        source: std::num::ParseFloatError,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Serialise `project` to an XML string.
pub fn save_project(project: &Project) -> Result<String, PersistenceError> {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    // XML declaration.
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

    // <project schema_version="1" root_diagram_id="...">
    let mut project_el = BytesStart::new(EL_PROJECT);
    project_el.push_attribute((
        ATTR_SCHEMA_VERSION,
        project.schema_version.to_string().as_str(),
    ));
    project_el.push_attribute((
        ATTR_ROOT_DIAGRAM_ID,
        project.root_diagram_id.hyphenated().to_string().as_str(),
    ));
    w.write_event(Event::Start(project_el))?;

    // <dictionary> ... </dictionary>
    write_dictionary(&mut w, &project.data_dictionary)?;

    // <diagrams> ... </diagrams>
    w.write_event(Event::Start(BytesStart::new(EL_DIAGRAMS)))?;
    // Root diagram first, then the rest in stable order.
    let root_id = project.root_diagram_id;
    if let Some(root) = project.diagrams.get(&root_id) {
        write_diagram(&mut w, root, root_id)?;
    }
    let mut other_ids: Vec<Uuid> = project
        .diagrams
        .keys()
        .filter(|&&id| id != root_id)
        .copied()
        .collect();
    other_ids.sort(); // deterministic output
    for id in other_ids {
        if let Some(diag) = project.diagrams.get(&id) {
            write_diagram(&mut w, diag, id)?;
        }
    }
    w.write_event(Event::End(BytesEnd::new(EL_DIAGRAMS)))?;

    // <orphaned> ... </orphaned>
    if project.orphaned_diagram_ids.is_empty() {
        w.write_event(Event::Empty(BytesStart::new(EL_ORPHANED)))?;
    } else {
        w.write_event(Event::Start(BytesStart::new(EL_ORPHANED)))?;
        for &id in &project.orphaned_diagram_ids {
            let mut ref_el = BytesStart::new(EL_REF);
            ref_el.push_attribute((ATTR_ID, id.hyphenated().to_string().as_str()));
            w.write_event(Event::Empty(ref_el))?;
        }
        w.write_event(Event::End(BytesEnd::new(EL_ORPHANED)))?;
    }

    w.write_event(Event::End(BytesEnd::new(EL_PROJECT)))?;

    Ok(String::from_utf8(buf).expect("XML output is always valid UTF-8"))
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
    Loader::new(xml).load()
}

/// Read the file at `path` and parse it as a project.
pub fn load_project_from_file(path: &std::path::Path) -> Result<Project, PersistenceError> {
    let xml = std::fs::read_to_string(path).map_err(PersistenceError::Io)?;
    load_project(&xml)
}

// ---------------------------------------------------------------------------
// Serialiser helpers
// ---------------------------------------------------------------------------

fn write_dictionary<W: Write>(
    w: &mut Writer<W>,
    dict: &DataDictionary,
) -> Result<(), PersistenceError> {
    if dict.primitives.is_empty() && dict.composites.is_empty() {
        w.write_event(Event::Empty(BytesStart::new(EL_DICTIONARY)))?;
        return Ok(());
    }
    w.write_event(Event::Start(BytesStart::new(EL_DICTIONARY)))?;

    for prim in &dict.primitives {
        let mut el = BytesStart::new(EL_PRIMITIVE);
        el.push_attribute((ATTR_NAME, prim.name()));
        w.write_event(Event::Empty(el))?;
    }
    for comp in &dict.composites {
        let mut el = BytesStart::new(EL_COMPOSITE);
        el.push_attribute((ATTR_NAME, comp.name()));
        w.write_event(Event::Start(el))?;
        write_type_entries(w, &comp.spec.entries)?;
        w.write_event(Event::End(BytesEnd::new(EL_COMPOSITE)))?;
    }

    w.write_event(Event::End(BytesEnd::new(EL_DICTIONARY)))?;
    Ok(())
}

fn write_type_entries<W: Write>(
    w: &mut Writer<W>,
    entries: &[TypeEntry],
) -> Result<(), PersistenceError> {
    for entry in entries {
        let mut el = BytesStart::new(EL_ENTRY);
        el.push_attribute((ATTR_COUNT, entry.count.to_string().as_str()));
        el.push_attribute((ATTR_TYPE_REF, entry.type_ref.as_str()));
        w.write_event(Event::Empty(el))?;
    }
    Ok(())
}

fn write_type_spec<W: Write>(
    w: &mut Writer<W>,
    tag: &str,
    spec: &TypeSpec,
) -> Result<(), PersistenceError> {
    if spec.is_empty() {
        // Omit empty specs entirely — the deserialiser treats absence as empty.
        return Ok(());
    }
    w.write_event(Event::Start(BytesStart::new(tag)))?;
    write_type_entries(w, &spec.entries)?;
    w.write_event(Event::End(BytesEnd::new(tag)))?;
    Ok(())
}

fn write_diagram<W: Write>(
    w: &mut Writer<W>,
    diag: &Diagram,
    diag_id: Uuid,
) -> Result<(), PersistenceError> {
    let id_str = diag_id.hyphenated().to_string();
    let mut el = BytesStart::new(EL_DIAGRAM);
    el.push_attribute((ATTR_ID, id_str.as_str()));
    el.push_attribute((ATTR_NAME, diag.name.as_str()));
    if let Some(pid) = diag.parent_process_id {
        el.push_attribute((
            ATTR_PARENT_PROCESS_ID,
            pid.hyphenated().to_string().as_str(),
        ));
    }
    w.write_event(Event::Start(el))?;

    // <viewport>
    let mut vp = BytesStart::new(EL_VIEWPORT);
    vp.push_attribute((ATTR_ZOOM, diag.viewport.zoom.to_string().as_str()));
    vp.push_attribute((ATTR_SCROLL_X, diag.viewport.scroll.0.to_string().as_str()));
    vp.push_attribute((ATTR_SCROLL_Y, diag.viewport.scroll.1.to_string().as_str()));
    w.write_event(Event::Empty(vp))?;

    // <nodes>
    write_nodes(w, diag)?;

    // <streams>
    write_streams(w, diag)?;

    // <groups>
    write_groups(w, diag)?;

    w.write_event(Event::End(BytesEnd::new(EL_DIAGRAM)))?;
    Ok(())
}

fn write_nodes<W: Write>(w: &mut Writer<W>, diag: &Diagram) -> Result<(), PersistenceError> {
    if diag.nodes.is_empty() {
        w.write_event(Event::Empty(BytesStart::new(EL_NODES)))?;
        return Ok(());
    }
    w.write_event(Event::Start(BytesStart::new(EL_NODES)))?;

    // Sort by ID for deterministic output.
    let mut nodes: Vec<&Node> = diag.nodes.values().collect();
    nodes.sort_by_key(|n| n.id);

    for node in nodes {
        let id_str = node.id.hyphenated().to_string();
        let kind_str = match node.kind {
            NodeKind::Source => NODE_KIND_SOURCE,
            NodeKind::Sink => NODE_KIND_SINK,
            NodeKind::Process => NODE_KIND_PROCESS,
        };
        let pos = diag.positions.get(&node.id).copied().unwrap_or((0.0, 0.0));

        let mut el = BytesStart::new(EL_NODE);
        el.push_attribute((ATTR_ID, id_str.as_str()));
        el.push_attribute((ATTR_KIND, kind_str));
        el.push_attribute((ATTR_NAME, node.name.as_str()));
        el.push_attribute((ATTR_X, pos.0.to_string().as_str()));
        el.push_attribute((ATTR_Y, pos.1.to_string().as_str()));
        if node.is_boundary {
            el.push_attribute((ATTR_IS_BOUNDARY, BOOL_TRUE));
        }
        if let Some(sid) = node.sub_diagram_id {
            el.push_attribute((ATTR_SUB_DIAGRAM_ID, sid.hyphenated().to_string().as_str()));
        }

        // Node has no children only when both specs are empty.
        let has_children = !node.input_spec.is_empty() || !node.output_spec.is_empty();
        if has_children {
            w.write_event(Event::Start(el))?;
            write_type_spec(w, EL_INPUT_SPEC, &node.input_spec)?;
            write_type_spec(w, EL_OUTPUT_SPEC, &node.output_spec)?;
            w.write_event(Event::End(BytesEnd::new(EL_NODE)))?;
        } else {
            w.write_event(Event::Empty(el))?;
        }
    }

    w.write_event(Event::End(BytesEnd::new(EL_NODES)))?;
    Ok(())
}

fn write_streams<W: Write>(w: &mut Writer<W>, diag: &Diagram) -> Result<(), PersistenceError> {
    if diag.streams.is_empty() {
        w.write_event(Event::Empty(BytesStart::new(EL_STREAMS)))?;
        return Ok(());
    }
    w.write_event(Event::Start(BytesStart::new(EL_STREAMS)))?;

    let mut streams: Vec<&Stream> = diag.streams.values().collect();
    streams.sort_by_key(|s| s.id);

    for stream in streams {
        let id_str = stream.id.hyphenated().to_string();
        let src_str = stream.source_node_id.hyphenated().to_string();
        let tgt_str = stream.target_node_id.hyphenated().to_string();

        let mut el = BytesStart::new(EL_STREAM);
        el.push_attribute((ATTR_ID, id_str.as_str()));
        el.push_attribute((ATTR_SOURCE_NODE_ID, src_str.as_str()));
        el.push_attribute((ATTR_TARGET_NODE_ID, tgt_str.as_str()));
        if let Some(gid) = stream.group_id {
            el.push_attribute((ATTR_GROUP_ID, gid.hyphenated().to_string().as_str()));
        }
        if stream.is_dangling {
            el.push_attribute((ATTR_IS_DANGLING, BOOL_TRUE));
        }

        if stream.type_spec.is_empty() {
            w.write_event(Event::Empty(el))?;
        } else {
            w.write_event(Event::Start(el))?;
            write_type_spec(w, EL_TYPE_SPEC, &stream.type_spec)?;
            w.write_event(Event::End(BytesEnd::new(EL_STREAM)))?;
        }
    }

    w.write_event(Event::End(BytesEnd::new(EL_STREAMS)))?;
    Ok(())
}

fn write_groups<W: Write>(w: &mut Writer<W>, diag: &Diagram) -> Result<(), PersistenceError> {
    if diag.stream_groups.is_empty() {
        w.write_event(Event::Empty(BytesStart::new(EL_GROUPS)))?;
        return Ok(());
    }
    w.write_event(Event::Start(BytesStart::new(EL_GROUPS)))?;

    let mut groups: Vec<&StreamGroup> = diag.stream_groups.values().collect();
    groups.sort_by_key(|g| g.id);

    for group in groups {
        let gid_str = group.id.hyphenated().to_string();
        let color_str = format!(
            "{:02x}{:02x}{:02x}",
            group.color[0], group.color[1], group.color[2]
        );
        let mut el = BytesStart::new(EL_GROUP);
        el.push_attribute((ATTR_ID, gid_str.as_str()));
        el.push_attribute((ATTR_NAME, group.name.as_str()));
        el.push_attribute((ATTR_COLOR, color_str.as_str()));

        if group.member_stream_ids.is_empty() {
            w.write_event(Event::Empty(el))?;
        } else {
            w.write_event(Event::Start(el))?;
            for &sid in &group.member_stream_ids {
                let mut mem = BytesStart::new(EL_MEMBER);
                mem.push_attribute((ATTR_STREAM_ID, sid.hyphenated().to_string().as_str()));
                w.write_event(Event::Empty(mem))?;
            }
            w.write_event(Event::End(BytesEnd::new(EL_GROUP)))?;
        }
    }

    w.write_event(Event::End(BytesEnd::new(EL_GROUPS)))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Deserialiser — state machine
// ---------------------------------------------------------------------------

/// Intermediate representation built during parsing.
struct ParsedDiagram {
    id: Uuid,
    name: String,
    parent_process_id: Option<Uuid>,
    viewport: ViewportState,
    nodes: HashMap<Uuid, Node>,
    positions: HashMap<Uuid, (f32, f32)>,
    streams: HashMap<Uuid, Stream>,
    stream_groups: HashMap<Uuid, StreamGroup>,
}

impl ParsedDiagram {
    fn into_diagram(self) -> Diagram {
        let mut d = Diagram::new(self.name);
        d.id = self.id;
        d.parent_process_id = self.parent_process_id;
        d.viewport = self.viewport;
        d.nodes = self.nodes;
        d.positions = self.positions;
        d.streams = self.streams;
        d.stream_groups = self.stream_groups;
        d
    }
}

/// The full deserialisation state.
struct Loader<'a> {
    reader: Reader<&'a [u8]>,
}

impl<'a> Loader<'a> {
    fn new(xml: &'a str) -> Self {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        Self { reader }
    }

    fn load(mut self) -> Result<Project, PersistenceError> {
        let mut buf = Vec::new();

        // --- Find <project> and read its attributes ---
        let mut root_diagram_id: Option<Uuid> = None;
        let mut schema_version: Option<u32> = None;

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) if e.name().as_ref() == EL_PROJECT.as_bytes() => {
                    for attr in e.attributes() {
                        let attr = attr.map_err(PersistenceError::Attr)?;
                        let key = std::str::from_utf8(attr.key.as_ref()).map_err(|_| {
                            PersistenceError::Malformed("non-UTF8 attribute key".into())
                        })?;
                        let val = attr.unescape_value().map_err(PersistenceError::Xml)?;
                        match key {
                            ATTR_SCHEMA_VERSION => schema_version = Some(parse_u32(&val)?),
                            ATTR_ROOT_DIAGRAM_ID => root_diagram_id = Some(parse_uuid(&val)?),
                            _ => {}
                        }
                    }
                    break;
                }
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "missing <project> root element".into(),
                    ))
                }
                _ => {}
            }
        }

        let schema_version = schema_version.ok_or_else(|| {
            PersistenceError::Malformed("missing schema_version attribute".into())
        })?;
        if schema_version != SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedVersion {
                found: schema_version,
                expected: SCHEMA_VERSION,
            });
        }
        let root_diagram_id = root_diagram_id.ok_or_else(|| {
            PersistenceError::Malformed("missing root_diagram_id attribute".into())
        })?;

        // --- Parse children of <project> ---
        let mut dict = DataDictionary::new();
        let mut diagrams: HashMap<Uuid, Diagram> = HashMap::new();
        let mut orphaned_ids: Vec<Uuid> = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                // <dictionary> with children
                Event::Start(ref e) if e.name().as_ref() == EL_DICTIONARY.as_bytes() => {
                    dict = self.parse_dictionary(e)?;
                }
                // <dictionary/> empty — leave dict as default empty
                Event::Empty(ref e) if e.name().as_ref() == EL_DICTIONARY.as_bytes() => {}
                // <diagrams>
                Event::Start(ref e) if e.name().as_ref() == EL_DIAGRAMS.as_bytes() => {
                    diagrams = self.parse_diagrams()?;
                }
                // <orphaned> with children
                Event::Start(ref e) if e.name().as_ref() == EL_ORPHANED.as_bytes() => {
                    orphaned_ids = self.parse_orphaned()?;
                }
                // <orphaned/> empty
                Event::Empty(ref e) if e.name().as_ref() == EL_ORPHANED.as_bytes() => {}
                Event::End(ref e) if e.name().as_ref() == EL_PROJECT.as_bytes() => break,
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(Project {
            schema_version,
            data_dictionary: dict,
            root_diagram_id,
            diagrams,
            orphaned_diagram_ids: orphaned_ids,
        })
    }

    // -----------------------------------------------------------------------
    // Dictionary parsing
    // -----------------------------------------------------------------------

    fn parse_dictionary(
        &mut self,
        _start: &BytesStart,
    ) -> Result<DataDictionary, PersistenceError> {
        let mut dict = DataDictionary::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) if e.name().as_ref() == EL_COMPOSITE.as_bytes() => {
                    let comp = self.parse_composite(e)?;
                    dict.composites.push(comp);
                }
                Event::Empty(ref e) => {
                    match e.name().as_ref() {
                        b if b == EL_PRIMITIVE.as_bytes() => {
                            let name = req_attr(e, ATTR_NAME)?;
                            dict.primitives.push(PrimitiveType::new(name));
                        }
                        b if b == EL_COMPOSITE.as_bytes() => {
                            // Empty composite (no entries).
                            let name = req_attr(e, ATTR_NAME)?;
                            dict.composites
                                .push(CompositeType::new(name, TypeSpec::new()));
                        }
                        _ => {}
                    }
                }
                Event::End(ref e) if e.name().as_ref() == EL_DICTIONARY.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <dictionary>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(dict)
    }

    fn parse_composite(&mut self, start: &BytesStart) -> Result<CompositeType, PersistenceError> {
        let name = req_attr(start, ATTR_NAME)?;
        let entries = self.parse_entries(EL_COMPOSITE)?;
        let mut spec = TypeSpec::new();
        for e in entries {
            spec.entries.push(e);
        }
        Ok(CompositeType::new(name, spec))
    }

    // -----------------------------------------------------------------------
    // Diagram parsing
    // -----------------------------------------------------------------------

    fn parse_diagrams(&mut self) -> Result<HashMap<Uuid, Diagram>, PersistenceError> {
        let mut diagrams = HashMap::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) if e.name().as_ref() == EL_DIAGRAM.as_bytes() => {
                    let parsed = self.parse_diagram(e)?;
                    let id = parsed.id;
                    diagrams.insert(id, parsed.into_diagram());
                }
                Event::Empty(ref e) if e.name().as_ref() == EL_DIAGRAM.as_bytes() => {
                    // Diagram with no children (unusual but valid).
                    let parsed = ParsedDiagram {
                        id: parse_uuid(&req_attr(e, ATTR_ID)?)?,
                        name: req_attr(e, ATTR_NAME)?,
                        parent_process_id: opt_attr(e, ATTR_PARENT_PROCESS_ID)?
                            .map(|s| parse_uuid(&s))
                            .transpose()?,
                        viewport: ViewportState::default(),
                        nodes: HashMap::new(),
                        positions: HashMap::new(),
                        streams: HashMap::new(),
                        stream_groups: HashMap::new(),
                    };
                    let id = parsed.id;
                    diagrams.insert(id, parsed.into_diagram());
                }
                Event::End(ref e) if e.name().as_ref() == EL_DIAGRAMS.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <diagrams>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(diagrams)
    }

    fn parse_diagram(&mut self, start: &BytesStart) -> Result<ParsedDiagram, PersistenceError> {
        let id = parse_uuid(&req_attr(start, ATTR_ID)?)?;
        let name = req_attr(start, ATTR_NAME)?;
        let parent_process_id = opt_attr(start, ATTR_PARENT_PROCESS_ID)?
            .map(|s| parse_uuid(&s))
            .transpose()?;

        let mut viewport = ViewportState::default();
        let mut nodes: HashMap<Uuid, Node> = HashMap::new();
        let mut positions: HashMap<Uuid, (f32, f32)> = HashMap::new();
        let mut streams: HashMap<Uuid, Stream> = HashMap::new();
        let mut stream_groups: HashMap<Uuid, StreamGroup> = HashMap::new();

        let mut buf = Vec::new();
        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Empty(ref e) => {
                    match e.name().as_ref() {
                        b if b == EL_VIEWPORT.as_bytes() => {
                            viewport = parse_viewport(e)?;
                        }
                        b if b == EL_NODES.as_bytes()
                            || b == EL_STREAMS.as_bytes()
                            || b == EL_GROUPS.as_bytes() =>
                        {
                            // Empty containers — nothing to parse.
                        }
                        _ => {}
                    }
                }
                Event::Start(ref e) => match e.name().as_ref() {
                    b if b == EL_NODES.as_bytes() => {
                        let (n, p) = self.parse_nodes()?;
                        nodes = n;
                        positions = p;
                    }
                    b if b == EL_STREAMS.as_bytes() => {
                        streams = self.parse_streams()?;
                    }
                    b if b == EL_GROUPS.as_bytes() => {
                        stream_groups = self.parse_groups()?;
                    }
                    _ => self.skip_element()?,
                },
                Event::End(ref e) if e.name().as_ref() == EL_DIAGRAM.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <diagram>".into(),
                    ))
                }
                _ => {}
            }
        }

        Ok(ParsedDiagram {
            id,
            name,
            parent_process_id,
            viewport,
            nodes,
            positions,
            streams,
            stream_groups,
        })
    }

    // -----------------------------------------------------------------------
    // Node parsing
    // -----------------------------------------------------------------------

    fn parse_nodes(
        &mut self,
    ) -> Result<(HashMap<Uuid, Node>, HashMap<Uuid, (f32, f32)>), PersistenceError> {
        let mut nodes: HashMap<Uuid, Node> = HashMap::new();
        let mut positions: HashMap<Uuid, (f32, f32)> = HashMap::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) if e.name().as_ref() == EL_NODE.as_bytes() => {
                    let (node, pos) = self.parse_node(e)?;
                    positions.insert(node.id, pos);
                    nodes.insert(node.id, node);
                }
                Event::Empty(ref e) if e.name().as_ref() == EL_NODE.as_bytes() => {
                    let (node, pos) = parse_node_attrs(e)?;
                    positions.insert(node.id, pos);
                    nodes.insert(node.id, node);
                }
                Event::End(ref e) if e.name().as_ref() == EL_NODES.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <nodes>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok((nodes, positions))
    }

    fn parse_node(&mut self, start: &BytesStart) -> Result<(Node, (f32, f32)), PersistenceError> {
        let (mut node, pos) = parse_node_attrs(start)?;
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) => match e.name().as_ref() {
                    b if b == EL_INPUT_SPEC.as_bytes() => {
                        node.input_spec = self.parse_type_spec(EL_INPUT_SPEC)?;
                    }
                    b if b == EL_OUTPUT_SPEC.as_bytes() => {
                        node.output_spec = self.parse_type_spec(EL_OUTPUT_SPEC)?;
                    }
                    _ => self.skip_element()?,
                },
                Event::Empty(ref e) => {
                    match e.name().as_ref() {
                        b if b == EL_INPUT_SPEC.as_bytes() => {}  // empty spec
                        b if b == EL_OUTPUT_SPEC.as_bytes() => {} // empty spec
                        _ => {}
                    }
                }
                Event::End(ref e) if e.name().as_ref() == EL_NODE.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <node>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok((node, pos))
    }

    // -----------------------------------------------------------------------
    // Stream parsing
    // -----------------------------------------------------------------------

    fn parse_streams(&mut self) -> Result<HashMap<Uuid, Stream>, PersistenceError> {
        let mut streams: HashMap<Uuid, Stream> = HashMap::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) if e.name().as_ref() == EL_STREAM.as_bytes() => {
                    let stream = self.parse_stream(e)?;
                    streams.insert(stream.id, stream);
                }
                Event::Empty(ref e) if e.name().as_ref() == EL_STREAM.as_bytes() => {
                    let stream = parse_stream_attrs(e)?;
                    streams.insert(stream.id, stream);
                }
                Event::End(ref e) if e.name().as_ref() == EL_STREAMS.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <streams>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(streams)
    }

    fn parse_stream(&mut self, start: &BytesStart) -> Result<Stream, PersistenceError> {
        let mut stream = parse_stream_attrs(start)?;
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) if e.name().as_ref() == EL_TYPE_SPEC.as_bytes() => {
                    stream.type_spec = self.parse_type_spec(EL_TYPE_SPEC)?;
                }
                Event::Empty(ref e) if e.name().as_ref() == EL_TYPE_SPEC.as_bytes() => {}
                Event::End(ref e) if e.name().as_ref() == EL_STREAM.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <stream>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(stream)
    }

    // -----------------------------------------------------------------------
    // Group parsing
    // -----------------------------------------------------------------------

    fn parse_groups(&mut self) -> Result<HashMap<Uuid, StreamGroup>, PersistenceError> {
        let mut groups: HashMap<Uuid, StreamGroup> = HashMap::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(ref e) if e.name().as_ref() == EL_GROUP.as_bytes() => {
                    let group = self.parse_group(e)?;
                    groups.insert(group.id, group);
                }
                Event::Empty(ref e) if e.name().as_ref() == EL_GROUP.as_bytes() => {
                    let id = parse_uuid(&req_attr(e, ATTR_ID)?)?;
                    let name = req_attr(e, ATTR_NAME)?;
                    let color = parse_color_opt(opt_attr(e, ATTR_COLOR)?);
                    let mut g = StreamGroup::new(name, vec![]);
                    g.id = id;
                    g.color = color;
                    groups.insert(id, g);
                }
                Event::End(ref e) if e.name().as_ref() == EL_GROUPS.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <groups>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(groups)
    }

    fn parse_group(&mut self, start: &BytesStart) -> Result<StreamGroup, PersistenceError> {
        let id = parse_uuid(&req_attr(start, ATTR_ID)?)?;
        let name = req_attr(start, ATTR_NAME)?;
        let color = parse_color_opt(opt_attr(start, ATTR_COLOR)?);
        let mut members: Vec<Uuid> = Vec::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Empty(ref e) if e.name().as_ref() == EL_MEMBER.as_bytes() => {
                    let sid = parse_uuid(&req_attr(e, ATTR_STREAM_ID)?)?;
                    members.push(sid);
                }
                Event::End(ref e) if e.name().as_ref() == EL_GROUP.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <group>".into(),
                    ))
                }
                _ => {}
            }
        }

        let mut g = StreamGroup::new(name, members);
        g.id = id;
        g.color = color;
        Ok(g)
    }

    // -----------------------------------------------------------------------
    // TypeSpec and entries
    // -----------------------------------------------------------------------

    fn parse_type_spec(&mut self, closing_tag: &str) -> Result<TypeSpec, PersistenceError> {
        let entries = self.parse_entries(closing_tag)?;
        let mut spec = TypeSpec::new();
        for e in entries {
            spec.entries.push(e);
        }
        Ok(spec)
    }

    fn parse_entries(&mut self, closing_tag: &str) -> Result<Vec<TypeEntry>, PersistenceError> {
        let mut entries: Vec<TypeEntry> = Vec::new();
        let closing = closing_tag.as_bytes();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Empty(ref e) if e.name().as_ref() == EL_ENTRY.as_bytes() => {
                    let count_str = req_attr(e, ATTR_COUNT)?;
                    let type_ref = req_attr(e, ATTR_TYPE_REF)?;
                    let count = parse_u32(&count_str)?;
                    entries.push(TypeEntry::new(count, type_ref));
                }
                Event::End(ref e) if e.name().as_ref() == closing => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(format!(
                        "unexpected EOF looking for </{closing_tag}>"
                    )))
                }
                _ => {}
            }
        }
        Ok(entries)
    }

    // -----------------------------------------------------------------------
    // Orphaned IDs
    // -----------------------------------------------------------------------

    fn parse_orphaned(&mut self) -> Result<Vec<Uuid>, PersistenceError> {
        let mut ids: Vec<Uuid> = Vec::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Empty(ref e) if e.name().as_ref() == EL_REF.as_bytes() => {
                    let id = parse_uuid(&req_attr(e, ATTR_ID)?)?;
                    ids.push(id);
                }
                Event::End(ref e) if e.name().as_ref() == EL_ORPHANED.as_bytes() => break,
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF inside <orphaned>".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(ids)
    }

    // -----------------------------------------------------------------------
    // Utility: skip unknown element and all its children
    // -----------------------------------------------------------------------

    fn skip_element(&mut self) -> Result<(), PersistenceError> {
        let mut depth = 1usize;
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match self.reader.read_event_into(&mut buf)? {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Event::Eof => {
                    return Err(PersistenceError::Malformed(
                        "unexpected EOF while skipping element".into(),
                    ))
                }
                _ => {}
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Stateless attribute parsers (take &BytesStart, return typed values)
// ---------------------------------------------------------------------------

fn parse_node_attrs(e: &BytesStart) -> Result<(Node, (f32, f32)), PersistenceError> {
    let id = parse_uuid(&req_attr(e, ATTR_ID)?)?;
    let name = req_attr(e, ATTR_NAME)?;
    let kind_str = req_attr(e, ATTR_KIND)?;
    let x = parse_f32(&req_attr(e, ATTR_X)?)?;
    let y = parse_f32(&req_attr(e, ATTR_Y)?)?;
    let is_boundary = opt_attr(e, ATTR_IS_BOUNDARY)?
        .map(|s| s == BOOL_TRUE)
        .unwrap_or(false);
    let sub_diagram_id = opt_attr(e, ATTR_SUB_DIAGRAM_ID)?
        .map(|s| parse_uuid(&s))
        .transpose()?;

    let kind = match kind_str.as_str() {
        NODE_KIND_SOURCE => NodeKind::Source,
        NODE_KIND_SINK => NodeKind::Sink,
        NODE_KIND_PROCESS => NodeKind::Process,
        other => {
            return Err(PersistenceError::Malformed(format!(
                "unknown node kind '{other}'"
            )))
        }
    };

    let node = Node {
        id,
        name,
        kind,
        input_spec: TypeSpec::new(),
        output_spec: TypeSpec::new(),
        is_boundary,
        sub_diagram_id,
    };
    Ok((node, (x, y)))
}

fn parse_stream_attrs(e: &BytesStart) -> Result<Stream, PersistenceError> {
    let id = parse_uuid(&req_attr(e, ATTR_ID)?)?;
    let source_node_id = parse_uuid(&req_attr(e, ATTR_SOURCE_NODE_ID)?)?;
    let target_node_id = parse_uuid(&req_attr(e, ATTR_TARGET_NODE_ID)?)?;
    let group_id = opt_attr(e, ATTR_GROUP_ID)?
        .map(|s| parse_uuid(&s))
        .transpose()?;
    let is_dangling = opt_attr(e, ATTR_IS_DANGLING)?
        .map(|s| s == BOOL_TRUE)
        .unwrap_or(false);

    Ok(Stream {
        id,
        source_node_id,
        target_node_id,
        type_spec: TypeSpec::new(),
        group_id,
        is_dangling,
    })
}

fn parse_viewport(e: &BytesStart) -> Result<ViewportState, PersistenceError> {
    let zoom = parse_f32(&req_attr(e, ATTR_ZOOM)?)?;
    let scroll_x = parse_f32(&req_attr(e, ATTR_SCROLL_X)?)?;
    let scroll_y = parse_f32(&req_attr(e, ATTR_SCROLL_Y)?)?;
    Ok(ViewportState {
        zoom,
        scroll: (scroll_x, scroll_y),
    })
}

// ---------------------------------------------------------------------------
// Low-level attribute helpers
// ---------------------------------------------------------------------------

/// Extract a required attribute value as a `String`.
fn req_attr(e: &BytesStart, name: &str) -> Result<String, PersistenceError> {
    for attr in e.attributes() {
        let attr = attr.map_err(PersistenceError::Attr)?;
        if attr.key.as_ref() == name.as_bytes() {
            return Ok(attr
                .unescape_value()
                .map_err(PersistenceError::Xml)?
                .into_owned());
        }
    }
    Err(PersistenceError::Malformed(format!(
        "missing required attribute '{name}' on <{}>",
        std::str::from_utf8(e.name().as_ref()).unwrap_or("?")
    )))
}

/// Extract an optional attribute value as a `String`.
fn opt_attr(e: &BytesStart, name: &str) -> Result<Option<String>, PersistenceError> {
    for attr in e.attributes() {
        let attr = attr.map_err(PersistenceError::Attr)?;
        if attr.key.as_ref() == name.as_bytes() {
            return Ok(Some(
                attr.unescape_value()
                    .map_err(PersistenceError::Xml)?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

/// Parse an optional hex-RGB colour attribute (e.g. `"ff8040"`) into `[u8; 3]`.
/// Returns the default grey when the attribute is absent or malformed.
fn parse_color_opt(raw: Option<String>) -> [u8; 3] {
    raw.and_then(|s| {
        if s.len() == 6 {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some([r, g, b])
        } else {
            None
        }
    })
    .unwrap_or_else(crate::model::stream::StreamGroup::default_color)
}

fn parse_uuid(s: &str) -> Result<Uuid, PersistenceError> {
    Uuid::parse_str(s).map_err(|e| PersistenceError::InvalidUuid {
        raw: s.to_owned(),
        source: e,
    })
}

fn parse_u32(s: &str) -> Result<u32, PersistenceError> {
    s.parse::<u32>().map_err(|e| PersistenceError::InvalidInt {
        raw: s.to_owned(),
        source: e,
    })
}

fn parse_f32(s: &str) -> Result<f32, PersistenceError> {
    s.parse::<f32>()
        .map_err(|e| PersistenceError::InvalidFloat {
            raw: s.to_owned(),
            source: e,
        })
}
