# Implementation Plan — Data Flow Diagramming Tool
## Claude-Assisted Build Guide

---

## How to Use This Document

Each phase below is designed to fit within a single Claude session (or a small number of sessions). For each session:

1. **Start a fresh Claude conversation**
2. **Paste the prompt template** at the top of the phase
3. **Attach the files listed** under "Context to provide"
4. **Ask Claude to produce the module** — review, compile, fix any errors (feeding errors back to Claude in the same session)
5. **Commit the working code** before starting the next phase

Never start a new phase until the previous one compiles and its tests pass. Claude cannot run your code — you are the build system.

---

## Guiding Principles for Every Session

Paste this block at the start of every Claude conversation:

```
You are implementing a Rust desktop application — a data flow diagramming tool.
The UI uses egui. Persistence is XML. The project structure is described in the
spec and module map I will provide. Write idiomatic Rust: prefer strong types over
strings, avoid unwrap() in non-test code, use thiserror for error types. When you
are uncertain about a design decision, ask rather than assume. Do not rewrite
modules I have not asked you to touch. You do not care about clean code principles, prefer efficiency. You will provide documentation throughout the code you write (compilable with cargo doc)
```

---

## Module Map

```
dataflow/
├── src/
│   ├── main.rs                  — app entry point, egui event loop
│   ├── app.rs                   — top-level AppState, coordinates all subsystems
│   │
│   ├── model/
│   │   ├── mod.rs
│   │   ├── types.rs             — PrimitiveType, CompositeType, TypeExpr, TypeSpec
│   │   ├── dictionary.rs        — DataDictionary, CRUD, rename/delete propagation
│   │   ├── node.rs              — Node, NodeKind (Source/Sink/Process)
│   │   ├── stream.rs            — Stream, StreamGroup
│   │   ├── diagram.rs           — Diagram, canvas positions, viewport state
│   │   └── project.rs           — Project, diagram tree, orphaned list
│   │
│   ├── validation/
│   │   ├── mod.rs
│   │   ├── flatten.rs           — recursive TypeSpec → primitive multiset
│   │   ├── balance.rs           — stream balance, node input/output balance
│   │   └── validator.rs         — incremental + full validation, ValidationResult
│   │
│   ├── persistence/
│   │   ├── mod.rs
│   │   ├── schema.rs            — XML schema constants, version
│   │   └── xml.rs               — serialise/deserialise Project ↔ XML
│   │
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── canvas.rs            — egui canvas: draw nodes/streams, drag/drop, zoom/pan
│   │   ├── tree_pane.rs         — left-hand project tree
│   │   ├── spec_panel.rs        — type specification editor (nodes and streams)
│   │   ├── dictionary_panel.rs  — data dictionary editor
│   │   ├── toolbar.rs           — top bar: new, open, save, auto-layout, full validate
│   │   └── dialogs.rs           — modal dialogs: rename, create group, set sub-diagram
│   │
│   └── undo/
│       ├── mod.rs
│       └── history.rs           — UndoStack, Command trait, compound actions
│
├── tests/
│   ├── type_equality.rs
│   ├── validation.rs
│   └── persistence.rs
│
└── Cargo.toml
```

---

## Phase 0 — Project Scaffold

**Goal:** Compilable project skeleton with all modules declared but empty.

**Claude prompt:**
```
Using the module map I'll provide, create a Cargo.toml and the full src/ directory
skeleton for a Rust egui desktop app. Each module file should compile but contain
only stub types and todo!() function bodies. Include dependencies: egui, eframe,
quick-xml, serde, thiserror, uuid. Generate the Cargo.toml with appropriate
features for eframe on Windows and Linux.
```

**Context to provide:** Module map (above).

**You do:** Confirm it compiles with `cargo build`. Fix any dependency version issues.

**Acceptance criteria:**
- `cargo build` succeeds with warnings only (no errors)
- All module files exist

---

## Phase 1 — Core Type Model

**Goal:** Complete, tested implementation of the type system and data dictionary.

**Session 1a — Type definitions**

```
Implement src/model/types.rs. Requirements:
- PrimitiveType(String) newtype
- TypeSpec: a multiset of (u32, TypeRef) pairs where TypeRef is a name string
- Structural equality for TypeSpec: flatten recursively through the dictionary to
  primitive counts, then compare. The name has no semantic weight.
- A method TypeSpec::flatten(dict) -> HashMap<String, u32> that resolves all
  nested composites recursively. Return an error if a type name is not found.
- CompositeType: a named TypeSpec
- Prohibit circular type definitions: when adding a composite, check that none
  of its members (recursively) reference the type being created.
Write unit tests in tests/type_equality.rs covering:
- Multiset order independence
- Structural equality across different names
- Circular definition rejection
- Nested composite flattening
```

**Context to provide:** Spec sections 2 (Core Concepts), module map.

**Session 1b — Data dictionary**

```
Implement src/model/dictionary.rs. Requirements:
- DataDictionary holds a Vec of primitives and a Vec of composites
- Add primitive: reject duplicate names (error)
- Add composite: reject duplicate names (error), reject circular definitions (error)
- Rename: update the name and patch all references in all other composite
  definitions that mention the old name. Return the list of patched type names.
- Delete: return the deleted entry. Callers are responsible for propagating the
  error state to referencing components (dictionary does not own diagrams).
- Lookup by name, list all names
```

**Context to provide:** Completed types.rs, spec section 2.

**Acceptance criteria (both sessions):**
- All tests in tests/type_equality.rs pass
- No unwrap() in non-test code

---

## Phase 2 — Graph Model

**Goal:** Node, stream, diagram, and project data structures with no UI or validation.

**Session 2a — Nodes and streams**

```
Implement src/model/node.rs and src/model/stream.rs. Requirements:

Node:
- Unique ID (uuid::Uuid)
- NodeKind enum: Source (outputs only), Sink (inputs only), Process (both)
- Name (String)
- input_spec: TypeSpec (empty for Source)
- output_spec: TypeSpec (empty for Sink)
- is_boundary: bool (locked auto-generated nodes in sub-diagrams)
- optional sub_diagram_id: Option<Uuid> (for Process nodes only)

Stream:
- Unique ID (Uuid)
- source_node_id, target_node_id: Uuid
- type_spec: TypeSpec
- group_id: Option<Uuid>
- is_dangling: bool (true when source or target node has been deleted)

StreamGroup:
- Unique ID (Uuid)
- name: String
- member_stream_ids: Vec<Uuid>
```

**Context to provide:** Completed types.rs, dictionary.rs, spec section 3.

**Session 2b — Diagram and project**

```
Implement src/model/diagram.rs and src/model/project.rs. Requirements:

Diagram:
- Unique ID (Uuid)
- name: String
- nodes: HashMap<Uuid, Node>
- streams: HashMap<Uuid, Stream>
- stream_groups: HashMap<Uuid, StreamGroup>
- positions: HashMap<Uuid, (f32, f32)>   — canvas XY per node
- viewport: ViewportState { zoom: f32, scroll: (f32, f32) }
- parent_process_id: Option<Uuid>   — None for root diagram

Project:
- data_dictionary: DataDictionary
- root_diagram_id: Uuid
- diagrams: HashMap<Uuid, Diagram>
- orphaned_diagram_ids: Vec<Uuid>
- schema_version: u32   — always written as 1 for now

Helper methods on Project:
- create_sub_diagram(process_node_id) -> Result<Uuid>
  Creates a diagram, generates locked boundary nodes from the process I/O,
  links the process node's sub_diagram_id.
- orphan_sub_diagram(diagram_id)
  Unlinks from parent process, moves to orphaned list.
- attach_orphan(orphan_diagram_id, process_node_id) -> Result<()>
  Links the orphan to the process. Sets boundary nodes to error state if
  the process I/O does not match.
- update_boundary_nodes(diagram_id) -> Vec<Uuid>
  Rebuilds locked boundary nodes to match the parent process I/O.
  Returns IDs of streams that became dangling as a result.
```

**Context to provide:** Completed types.rs, dictionary.rs, node.rs, stream.rs, spec sections 3 and 6.

**Acceptance criteria:**
- `cargo build` succeeds
- create_sub_diagram produces correct boundary nodes for a simple test case (write this test yourself inline)

---

## Phase 3 — Validation

**Goal:** Complete validation logic, fully tested, with no UI dependency.

**Session 3a — Flattening**

```
Implement src/validation/flatten.rs. Requirements:
- flatten(type_spec: &TypeSpec, dict: &DataDictionary) -> Result<HashMap<String,u32>>
  Recursively resolves all composite types to their primitive components.
  Returns Err if any referenced type name is not in the dictionary.
- Two TypeSpecs are structurally equal if their flatten() results are equal.
Write tests covering nested composites, missing type error, structural equality.
```

**Session 3b — Balance checker**

```
Implement src/validation/balance.rs. Requirements:

stream_balance(stream, source_node, target_node, dict) -> BalanceResult
  Flatten the stream's type_spec.
  Check source_node output_spec flattened contains all stream types (quantity match).
  Check target_node input_spec flattened contains all stream types (quantity match).
  Return warnings for each direction that fails.

node_input_balance(node, incoming_streams, dict) -> BalanceResult
  Sum flattened type contributions of all incoming streams.
  Compare to node input_spec flattened.
  Undersupply -> warning. Oversupply -> warning. Exact match -> ok.

node_output_balance(node, outgoing_streams, dict) -> BalanceResult
  Sum flattened type contributions of all outgoing streams.
  Compare to node output_spec flattened.
  Undersupply -> warning. Oversupply -> warning. Exact match -> ok.

BalanceResult should carry: ok | warning(Vec<String>) | error(Vec<String>)
Write tests for all cases in the spec including the (2,int) oversupply example.
```

**Context to provide:** Completed model modules, flatten.rs, spec section 4.

**Session 3c — Validator**

```
Implement src/validation/validator.rs. Requirements:

ValidationState: a HashMap<Uuid, ComponentState> where ComponentState is
  Ok | Warning(Vec<String>) | Error(Vec<String>)

Validator holds a ref to Project and a ValidationState.

validate_incremental(changed_id: Uuid)
  Re-validates: the changed component + all directly connected components.

validate_full()
  Re-validates every node and stream in the current diagram.

On dictionary rename: mark all components referencing that name for re-validation.
On dictionary delete: mark all referencing components as Error immediately.
On node delete: mark connected streams as Error (dangling) immediately.
```

**Context to provide:** All model modules, flatten.rs, balance.rs, spec section 4.

**Acceptance criteria:**
- All tests in tests/validation.rs pass
- Validator has no egui dependency

---

## Phase 4 — Undo / Redo

**Goal:** Global command stack that wraps all model mutations.

```
Implement src/undo/history.rs. Requirements:

Command trait:
  fn apply(&self, project: &mut Project) -> Result<()>
  fn undo(&self, project: &mut Project) -> Result<()>
  fn description(&self) -> &str

UndoStack:
  - past: Vec<Box<dyn Command>>
  - future: Vec<Box<dyn Command>>
  - execute(cmd): applies and pushes to past, clears future
  - undo(): pops past, calls undo(), pushes to future
  - redo(): pops future, calls apply(), pushes to past

Compound commands: a CompoundCommand holds Vec<Box<dyn Command>> and
applies/undoes them in order/reverse. Used for operations that touch
both parent and sub-diagram (e.g. updating a process I/O).

Implement concrete commands for:
- AddNode, DeleteNode
- AddStream, DeleteStream
- EditNodeSpec (stores before/after TypeSpec)
- EditStreamSpec
- AddType, DeleteType, RenameType (dictionary operations)
- CreateSubDiagram
- OrphanSubDiagram, AttachOrphan
- CreateStreamGroup, DeleteStreamGroup, EditStreamGroup
```

**Context to provide:** All model modules, spec sections 3, 4, 6.

**Acceptance criteria:**
- Undo/redo of every command returns project to prior state
- Write a test: add node → add stream → undo → undo → state matches initial

---

## Phase 5 — Persistence

**Goal:** Save and load a complete Project to/from XML.

**Session 5a — Schema definition**

```
Define the XML schema in src/persistence/schema.rs as documented string constants.
The schema must cover all fields described in the spec section 7. Write the schema
as an annotated example XML document in a doc comment — this is the reference for
the serialiser/deserialiser.
```

**Context to provide:** Completed model modules, spec section 7.

**Session 5b — Serialiser / deserialiser**

```
Implement src/persistence/xml.rs using quick-xml. Requirements:
- serialise(project: &Project) -> Result<String>
- deserialise(xml: &str) -> Result<Project>
All Uuid fields serialise as hyphenated strings.
schema_version is written as an attribute on the root element.
The data dictionary serialises before diagrams.
Viewport state and positions are included per diagram.
Write tests in tests/persistence.rs:
- Round-trip: serialise then deserialise, compare all fields
- Schema version attribute is present
- Missing type reference on load produces a recoverable error, not a panic
```

**Context to provide:** Completed model modules, schema.rs, spec section 7.

**Acceptance criteria:**
- Round-trip test passes for a project with: 2 types, 3 nodes, 2 streams, 1 sub-diagram
- `cargo test` fully green

---

## Phase 6 — UI: Foundation

**Goal:** Running egui window with app state wired up, no canvas yet.

**Session 6a — AppState and main loop**

```
Implement src/app.rs and src/main.rs. Requirements:
AppState holds:
  - project: Project
  - validator: Validator
  - undo_stack: UndoStack
  - active_diagram_id: Uuid
  - ui state flags (which panels are open, selection state)

Wire up eframe::run_native with AppState as the eframe::App implementor.
The update() method should call each UI panel's show() method.
No canvas drawing yet — just a blank central panel and a left panel stub.
Confirm it opens a window and does not crash.
```

**Session 6b — Toolbar**

```
Implement src/ui/toolbar.rs. A top egui menu bar / toolbar with:
- File: New Project, Open, Save, Save As
- Edit: Undo, Redo (greyed out when stack is empty)
- View: Auto Layout, Full Validate
- Label showing the current diagram name and path in the tree
Wire file actions to placeholder functions for now (open/save will call persistence
in phase 7). Wire undo/redo to UndoStack.
```

**Session 6c — Tree pane**

```
Implement src/ui/tree_pane.rs. An egui left panel showing:
- The project name at the root
- The base diagram
- Sub-diagrams nested under their parent process node name
- An "Orphaned" section (visible only when orphaned_diagram_ids is non-empty)
Each diagram entry is clickable — sets AppState::active_diagram_id.
Each entry shows a warning or error badge if the diagram contains unresolved issues.
Right-click on a diagram entry: Rename.
Right-click on a Process node in the tree: "Create sub-diagram", "Set sub-diagram"
  (the latter only if orphaned diagrams exist).
```

**Context to provide:** Completed model, validation, undo modules. App state design.

---

## Phase 7 — UI: Panels

**Goal:** Data dictionary editor and specification panel, fully wired to model and undo stack.

**Session 7a — Dictionary panel**

```
Implement src/ui/dictionary_panel.rs. An egui side panel or window showing:
- List of all types (primitives and composites) with their definitions
- "Add primitive" button → text input for name → dispatches AddType command
- "Add composite" button → opens a sub-form: name + list of (count, type) entries
  with + / - controls and a type picker from existing names
- Each entry has: Rename button (dispatches RenameType), Delete button
  (dispatches DeleteType, shows confirmation if the type is in use)
- Inline error if a duplicate name is attempted
```

**Session 7b — Specification panel**

```
Implement src/ui/spec_panel.rs. A panel that opens when the user selects a node
or stream. Shows:
- Component name (editable, dispatches rename command)
- For nodes: NodeKind label (not editable for boundary nodes)
- Input spec section (hidden for Source nodes and boundary Source nodes)
- Output spec section (hidden for Sink nodes and boundary Sink nodes)
- For streams: single type spec section
Each spec section shows a list of (count, type_name) entries:
  - Type name label
  - − button (decrements count, removes entry at 0)
  - Count display
  - + button (increments count)
  - "Add type" button opens a picker of all dictionary types
If the same type is added twice, merge counts.
Dispatch EditNodeSpec or EditStreamSpec commands on every change.
Boundary nodes show all controls as read-only.
```

**Session 7c — Dialogs**

```
Implement src/ui/dialogs.rs. Modal dialogs using egui Windows:
- RenameDialog(current_name) -> Option<String>
- CreateGroupDialog -> Option<String>   (just a name input + confirm/cancel)
- SetSubDiagramDialog(orphan_list: Vec<(Uuid,String)>) -> Option<Uuid>
  Shows a list of orphaned diagram names, user picks one.
- ConfirmDeleteDialog(message) -> bool
Each dialog is shown conditionally based on a flag in AppState.
```

**Context to provide:** All completed modules, spec sections 3, 5.

---

## Phase 8 — UI: Canvas

**Goal:** The main canvas — the most complex phase. Split into four focused sessions.

**Session 8a — Drawing nodes and streams**

```
Implement the drawing layer in src/ui/canvas.rs using egui Painter.
For the active diagram, draw:
- Each node as a rounded rectangle. Label with name and kind (Source/Sink/Process).
  Source = one colour, Sink = another, Process = another. Boundary nodes have a
  distinct visual treatment (e.g. dashed border).
- Each stream as a horizontal/vertical orthogonal path (two-segment L-shape minimum)
  from source node edge to target node edge.
- Stream groups as parallel offset lines along the same path.
- Validation state overlaid: warning icon (amber) or error icon (red) on affected
  components. Ok state has no indicator.
Positions come from diagram.positions. No interaction yet.
```

**Session 8b — Zoom, pan and selection**

```
Add to canvas.rs:
- Zoom: scroll wheel scales around the mouse position. Store zoom in ViewportState.
- Pan: middle-mouse drag or space+drag moves the viewport.
- Click to select a node or stream (highlight with a selection outline).
  Store selected_id: Option<Uuid> in AppState.
- Click on empty canvas deselects.
- Opening the spec panel is triggered by double-clicking a node or stream,
  or by selecting and pressing Enter.
Coordinate system: maintain a transform between screen space and canvas space.
All hit testing uses canvas space.
```

**Session 8c — Node drag and drop**

```
Add to canvas.rs:
- Dragging a node moves it. On release, dispatch a MoveNode command
  (store before/after position, undoable).
- Dragging from the node palette (a small floating panel listing Source/Sink/Process)
  onto the canvas creates a new node at the drop position. Dispatch AddNode.
- Nodes snap to a coarse grid (16px) when dropped, to keep layout tidy.
```

**Session 8d — Stream creation and connection**

```
Add to canvas.rs:
- Each node displays a small drag handle on its edges (visible on hover).
- Dragging from a handle begins stream creation — draw a preview line following
  the mouse.
- Dropping on another node's edge (not the same node) completes the connection.
  Dispatch AddStream, then trigger incremental validation.
- Dropping on empty canvas cancels the stream.
- Stream endpoints can be dragged after creation to reconnect.
- Right-clicking a node shows context menu: Delete, Open spec, Create sub-diagram
  (Process only), Set sub-diagram (Process only, if orphans exist).
- Right-clicking a stream shows: Delete, Open spec.
- Right-clicking with multiple streams selected shows: Create group.
```

**Context to provide:** All completed modules plus sessions 8a–8c output for 8d.

**Acceptance criteria:**
- Can create a project with nodes and streams entirely through the UI
- Validation warnings/errors appear correctly on canvas
- Undo/redo works for all canvas interactions

---

## Phase 9 — File I/O and Auto Layout

**Session 9a — Wire up persistence**

```
Wire src/persistence/xml.rs into the toolbar File actions in src/ui/toolbar.rs.
- New Project: replace AppState::project with a fresh Project, clear undo stack.
- Open: show a file picker (use rfd crate), load XML, replace project, clear undo.
- Save / Save As: serialise project, write to file.
Store the current file path in AppState so Save knows where to write.
```

**Session 9b — Auto layout**

```
Implement auto-layout in a new src/layout.rs module.
Algorithm: simple layered grid placement.
1. Topological sort nodes (ignore back-edges for cycles).
2. Assign each node a column based on its longest input path length.
3. Within each column, assign rows in the order encountered.
4. Position: x = column * (node_width + h_gap), y = row * (node_height + v_gap)
5. Return a HashMap<Uuid, (f32,f32)> of new positions.
Apply as a compound undo command that stores all before/after positions.
Wire to the toolbar "Auto Layout" button.
```

**Context to provide:** Completed persistence module, AppState, toolbar.

---

## Phase 10 — Draw.io Export

```
Implement src/persistence/drawio.rs. Requirements:
- export_diagram(diagram: &Diagram, project: &Project) -> Result<String>
  Exports the currently active diagram (not the full tree, unless spec is updated).
- Node mapping: Source → draw.io ellipse, Sink → draw.io rounded rect,
  Process → draw.io rectangle. Label = node name.
- Streams → draw.io edges with label showing the stream's type spec summary.
- Stream groups → draw.io edge groups if supported, otherwise visually offset.
- Use the node's Uuid as the draw.io cell id (stable across saves).
- Wire to File → Export → draw.io in the toolbar.
draw.io XML format reference: mxGraph XML. Use the compressed=0 variant for
human-readable output.
```

**Context to provide:** Completed model, persistence modules. draw.io XML format reference (paste a sample draw.io file for Claude to follow).

---

## Phase 11 — Polish and Edge Cases

These are best handled as individual small sessions, one topic each:

| Session | Topic | Prompt focus |
|---|---|---|
| 11a | Error state display | Ensure every error/warning path is visually covered; audit all ValidationState transitions |
| 11b | Boundary node cascade | Test and harden update_boundary_nodes; verify undo of a parent I/O change restores sub-diagram correctly |
| 11c | Orphan re-attachment | Test SetSubDiagram with matching and mismatching boundary nodes; verify error state clears on resolution |
| 11d | Dictionary delete propagation | Verify deleting a type in use correctly errors all referencing nodes/streams across all diagram levels |
| 11e | Rename propagation | Verify renaming a type updates all spec panels, tree pane, and canvas labels live |
| 11f | Multi-select | Box-select multiple nodes; move as a group; delete as a group |
| 11g | Copy/paste | Copy selected nodes+streams; paste creates new UUIDs, preserves relative positions |
| 11h | Window state persistence | Save/restore tree pane width, last file path, window size using a sidecar config file |

---

## Dependency Reference

```toml
[dependencies]
eframe = { version = "0.27", features = ["default"] }
egui = "0.27"
uuid = { version = "1", features = ["v4"] }
quick-xml = { version = "0.31", features = ["serialize"] }
serde = { version = "1", features = ["derive"] }
thiserror = "1"
rfd = "0.14"          # file dialogs

[dev-dependencies]
pretty_assertions = "1"
```

> Check crates.io for the latest compatible versions before starting. Pin versions in Cargo.lock from phase 0.

---

## Session Handoff Template

Use this at the start of any session that continues a previous phase:

```
I am continuing implementation of a Rust data flow diagramming tool.
Here is the project specification: [paste dataflow_spec.md]
Here is the module map: [paste module map from this document]
Here are the files relevant to this session: [paste file contents]
The previous session produced: [brief description of what was completed]
Today's goal: [paste the session prompt from this document]
```

---

## Risk Log

| Risk | Mitigation |
|---|---|
| Canvas hit testing complexity grows with stream routing | Keep stream paths simple (2-segment L-shape) until phase 11; defer bezier curves |
| egui immediate-mode conflicts with complex drag state | Isolate all drag state in a DragState struct in AppState; never infer drag state from widget position |
| Undo stack memory growth on large diagrams | Add a max stack depth (e.g. 200 commands) with oldest-first eviction |
| quick-xml deserialisation of deeply nested types | Test round-trip from phase 5 onward; add schema_version migration path from day one |
| Claude context limit mid-phase | Each session prompt lists exactly which files to paste; keep sessions to one module |
