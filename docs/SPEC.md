# Data Flow Diagramming Tool — Project Specification

## 1. Overview

A standalone desktop data flow diagramming tool compiled for Windows and Linux.

| Property | Value |
|---|---|
| Language | Rust |
| UI framework | egui |
| Persistence | XML (schema version 1) |
| Target platforms | Windows, Linux (including WSL) |

---

## 2. Core Concepts

### 2.1 Type System

The project maintains a single **data dictionary** at the project root. All diagrams at every level share this dictionary. Types may not be defined anywhere else.

There are two kinds of type:

**Primitive type** — a named leaf with no internal structure. The name is the entire definition.
```
uint32_t
char
float
```

**Composite type** — a named multiset of (count, type) pairs. Order is irrelevant; the multiset is the identity.
```
MyPacket = ((3, uint32_t), (2, char))
```

Composite types may nest other composite types. Circular type definitions are not permitted and must be rejected on creation.

#### Equality rule
Two types are equal if their multisets are structurally equal when flattened recursively to primitive counts. The name carries no semantic weight.

```
Type X = ((2, uint32_t))
Type Y = ((2, uint32_t))
X == Y   ✓
```

#### Dictionary constraints
- Names must be unique across both primitives and composites. An error is shown if the user attempts to create a type with a name already in use.
- Renaming a type propagates to all composite definitions that reference the old name. Returns the list of composite names that were patched.
- Deleting a type places all referencing components into an **error state**. The dictionary does not own diagram references; callers are responsible for propagating the error state.

#### Implementation decisions (phases 1a/1b)
- `TypeSpec` is the canonical multiset representation used on every node and stream. It enforces a single entry per `type_ref` name; duplicate additions merge counts.
- Adding zero units to a `TypeSpec` is a no-op.
- `TypeSpec::flatten()` delegates to `pub(crate) flatten_type_into()` in `model/types.rs`. The `validation::flatten::flatten_spec()` function is the public API for the validation layer and delegates to the same implementation. This keeps the model self-contained while giving the validation layer a clean seam to extend.
- `check_circular()` uses DFS with an ancestor stack (cycle detection) and a visited set (diamond-graph deduplication). It is called by `DataDictionary::add_composite` before insertion.
- `TypeError` and `DictionaryError` both derive `PartialEq + Eq` to support `assert_eq!` in tests.
- `DataDictionary::rename` returns `Vec<String>` of patched composite names so undo commands know exactly what changed.
- `DataDictionary::delete` returns the deleted name; callers own error propagation.

---

## 3. Diagram Elements

### 3.1 Nodes

Each node has a **kind**, a **name**, an `input_spec` and an `output_spec` (both `TypeSpec`).

| Node kind | Inputs | Outputs | Sub-diagram |
|---|---|---|---|
| Source | None (empty input_spec) | ≥ 1 | No |
| Sink | ≥ 1 | None (empty output_spec) | No |
| Process | ≥ 1 | ≥ 1 | Optional (1-to-1) |

Nodes cannot connect to themselves (enforced at `Diagram::add_stream`).

#### Implementation decisions (phase 2)
- `Node` derives `PartialEq` so `RemovedNode` (which contains a `Node`) can also derive `PartialEq`, enabling `assert_eq!` on `Result<RemovedNode, DiagramError>` in tests.
- `Node::rename()` returns `bool` (false for boundary nodes) rather than `Result` — the guard is simple and callers always check.
- `Node::attach_subdiagram()` returns `bool` — false if the node is not a Process kind.
- `NodeKind` implements `Display` (label strings: "Source", "Sink", "Process").
- `is_renameable()` and `is_deletable()` are the canonical guard methods for UI and command layers.

### 3.2 Streams

A stream is a point-to-point, unidirectional connection between two nodes. It carries a `TypeSpec` that may be edited independently of the nodes at either end.

- Multiple streams may leave or enter a single node (fan-out and fan-in are permitted).
- Cycles in the graph are permitted.
- Self-loops (source == target) are rejected by `Diagram::add_stream` with `DiagramError::SelfLoop`.

#### Implementation decisions (phase 2)
- `Stream` derives `PartialEq` for test ergonomics.
- `Stream::mark_dangling()` is the only way to set `is_dangling = true`; it is called exclusively by `Diagram::remove_node`.
- `StreamGroup::new()` silently deduplicates member IDs to maintain the no-duplicate invariant.
- `StreamGroup::add_stream()` is idempotent — returns `false` if the stream is already a member.

### 3.3 Stream Groups

Streams may be grouped for display purposes only. Grouping has no effect on validation or semantics.

**Creating a group:** Select two or more streams → right-click → "Create group" → enter a name. Grouped streams are displayed as parallel lines.

**Editing a group:** Streams may be added to or removed from an existing group. If all member streams are deleted the group dissolves automatically.

---

## 4. Validation

### 4.1 Compatibility

Type compatibility uses **structural equality**: two type expressions are compatible if they flatten to the same multiset of primitives. Names are irrelevant.

#### Implementation decisions (phase 3a)
- `validation::flatten::flatten_spec()` is the authoritative public API for the validation layer. All validation code flattens through this function rather than calling `TypeSpec::flatten` directly.
- `validation::flatten::structurally_equal()` is a convenience wrapper at the validation layer that calls `flatten_spec` on both specs and compares the results. Validation code uses this rather than `TypeSpec::structurally_equal`.
- `validation::flatten::sum_specs()` merges an iterator of flattened maps into a single primitive-count map. Used by balance checkers to sum all incoming or outgoing stream specs without repeated allocation.
- Flattening an empty `TypeSpec` always returns an empty map (not an error).
- The flattening implementation (`flatten_type_into`) clones composite entries before recursing to avoid holding an immutable borrow on the dictionary during mutable accumulator updates.
- Diamond-shaped composite graphs (one type referenced by two different paths) produce correct accumulated counts because the accumulator is additive, not set-based.

### 4.2 Balance Rules

Validation checks whether nodes and streams are **balanced**. Unbalanced components display a **warning**. Missing types or dangling connections display an **error**. Warnings and errors are shown locally on the affected component.

#### Stream balance
A stream is unbalanced if its source node does not output all the types the stream carries, or if its target node does not accept all the types the stream carries. Comparison is done by flattening to primitive counts.

#### Node input balance
Sum the type contributions of all incoming streams. The total must exactly match the node's input specification — no more, no less. Oversupply is a warning.

```
Node input requires: (2, int)
Stream 1 carries:    (1, int)
Stream 2 carries:    (1, int)
→ Balanced ✓

Stream 1 carries:    (2, int)
Stream 2 carries:    (2, int)
→ Oversupplied ✗ (warning)
```

#### Node output balance
Sum the type contributions of all outgoing streams. The total must not exceed the node's output specification — over-consumption is a warning, and the same primitive unit of output may not be consumed more than once across all outgoing streams.

```
Node output provides: (2, int)
Stream 1 carries:     (1, int)
Stream 2 carries:     (1, int)
→ Balanced ✓

Stream 1 carries:     (2, int)
Stream 2 carries:     (2, int)
→ Over-consumed ✗ (warning)
```

Under-consumption (outgoing streams consume fewer units than the node outputs) is also a warning, consistent with the input balance rule.

### 4.3 Error States

| Condition | State |
|---|---|
| Type used by component is deleted from dictionary | Error on all referencing components |
| Node is deleted leaving connected streams | Error on the dangling streams |
| Sub-diagram boundary mismatch after re-attachment | Error on the mismatched boundary nodes |

### 4.4 Validation Triggers

- **Incremental validation** runs automatically on every change. Scope: the changed component plus all directly connected components.
- **Full validation** is available via a UI button and checks the entire project.

#### Implementation decisions (phase 3b)

**Type separation — `BalanceResult` vs `Finding`.**
Balance functions return `BalanceResult { Ok | Warning(Vec<String>) | Error(Vec<String>) }`. `Finding { component_id, severity, message }` is the storage type held by `ValidationCache`. `Finding::from_balance_result(id, result)` converts between them. This keeps balance functions pure (no UUID threading) and trivially testable in isolation.

**Stream balance is containment, not exact match.**
`stream_balance` warns when a stream *over-draws* from its source (stream needs more than the source declares) or *over-delivers* to its target (stream carries more than the target accepts). It does **not** warn when a source outputs more than a single stream carries — that is legitimate fan-out, checked at the node level by `node_output_balance`. Conflating these two checks would produce spurious warnings on every fan-out graph.

**`subtract_specs` sign convention.**
`subtract_specs(supplied, required)` → positive = surplus, negative = deficit. Zero entries are dropped. This single convention covers both node input balance (supply vs demand) and node output balance (consumed vs provided) by choosing which argument is `lhs`.

**`Severity` derives `PartialOrd + Ord`** so the validator can sort findings for display and `Error > Warning` comparisons work without explicit `match` arms.

**Dangling stream handling.**
Balance functions assume non-dangling inputs. The orchestration layer (`validator`) skips balance checks for dangling components and emits an `Error` finding directly, before calling any balance function.

**Private helper functions** (`fmt_flat`, `fmt_delta`, `warnings_from_delta`) are defined in `balance.rs` with `#[allow(dead_code)]`. They will be used by the validator UI layer (phase 7) for consistent tooltip formatting and are not removed to avoid rewriting them later.

#### Implementation decisions (phase 3c)

**`ValidationCache` stores `BalanceResult` directly**, not a parallel `(Vec<Finding>, ValidationState)` pair. The old stub held two redundant maps; the new implementation holds one `HashMap<Uuid, BalanceResult>` and derives `ValidationState` on demand via `ValidationState::from_result`. This eliminates a consistency hazard where the two maps could diverge.

**`validate_incremental` does not take a `diagram_id` parameter.** The old stub signature was `validate_incremental(changed_id, project, diagram_id)`. The new signature is `validate_incremental(changed_id, project)`. The validator scans all diagrams to locate `changed_id` itself via the private `find_component` helper. This removes a footgun: passing the wrong `diagram_id` would silently miss the component.

**Incremental scope for a node**: the changed node + every stream touching it + every node at the other end of those streams. This ensures that a node spec edit propagates to its immediate graph neighbourhood without requiring a full re-validation.

**Incremental scope for a stream**: the changed stream + its source node + its target node.

**`on_type_deleted` injects errors directly** without calling balance functions, because the type no longer exists in the dictionary — `flatten_spec` would return `UnknownType` for every component. The error message names the deleted type. Only *direct* spec references (`type_ref == name`) are scanned; indirect references through composites will surface naturally as `UnknownType` errors when balance functions next run on those components.

**`on_type_renamed` re-validates after the rename is already committed.** The dictionary has already patched its composite entries; the caller is responsible for patching node and stream specs before calling this hook. The hook then scans for specs referencing the *new* name and re-validates them. Renamed components will usually resolve to `Ok` unless the rename introduced a new structural mismatch.

**`on_node_deleted` is called with `dangled_stream_ids`** (the return value of `Diagram::remove_node`). This avoids a second scan of the diagram. The deleted node's own cache entry is cleared; the dangled stream IDs receive `Error` injections.

**`spec_references` is a direct string match** on `type_ref` fields. It does not follow composite references transitively. This is intentional: transitive references through composites will be caught as `UnknownType` errors by the balance functions on their next run. A transitive scan here would couple the validator to the dictionary structure unnecessarily.

### 5.1 Canvas

The main editing area is a free-form canvas supporting:

- Drag and drop placement of nodes
- Stream creation by dragging a stream endpoint to a target node
- Manual repositioning of all elements
- Zoom and pan

### 5.2 Auto Layout

An auto-layout option arranges nodes in **orthogonal style** using the simplest available grid-routing method. Running auto-layout is undoable.

### 5.3 Specification Panel

Every component (node or stream) has a specification panel opened by the user on demand. The panel shows:

- The current type specification (list of (count, type) entries)
- A button to add a type from the data dictionary
- `+` / `−` controls on each entry to adjust the count
- Removing a type entry when its count reaches zero

If the same type is added twice, the counts are merged into a single entry.

### 5.4 Tree Pane

A left-hand tree pane shows the full project hierarchy:

- Project root (contains the data dictionary and the base diagram)
- Sub-diagrams nested under their parent process nodes
- An **Orphaned** directory for sub-diagrams whose parent process has been deleted

The tree pane displays error and warning badges on diagrams that contain unresolved issues. Diagrams can be renamed from the pane. The pane is resizable.

### 5.5 Undo / Redo

A single global undo/redo stack covers all changes across all diagram levels. A compound action (e.g. editing a parent process I/O that cascades into a sub-diagram boundary update) is a single undoable step.

#### Implementation decisions (phase 4)

**`Command` trait is fallible.** Both `apply` and `undo` return `Result<(), CommandError>` where `CommandError = Box<dyn Error + Send + Sync>`. A command whose `apply` fails is **not pushed** onto the stack — `UndoStack::execute` only stores a command after successful application. A failed `undo` discards the command from the `past` stack rather than pushing it to `future`, since the project state may be inconsistent.

**`UndoStack::execute` replaces the old `push` method.** The new name makes the intent explicit: execute applies and records, distinguishing it from internal stack management.

**`CompoundCommand::apply` rolls back on partial failure.** If sub-command `n` fails, sub-commands `0..n-1` are undone in reverse order before the error is returned. This keeps the project in its pre-apply state. `undo` continues reversing all sub-commands even if one fails, collecting only the first error.

**Snapshot-based concrete commands.** Every command stores everything needed for both directions up-front — no inverse computation at undo time. `DeleteNode` stores the full `Node` struct and its canvas position. `DeleteStream` stores the full `Stream`. `EditNodeSpec` and `EditStreamSpec` store before/after `TypeSpec` snapshots. `EditStreamGroup` stores before/after `StreamGroup` snapshots.

**`EditNodeSpec` uses a `NodeSpecSide` enum** (`Input` / `Output`) to avoid two separate command types for editing the two specs on a node.

**`DeleteNode` is an atomic command** that expects connected streams to already be removed. The UI layer wraps it in a `CompoundCommand` with the stream deletions. This keeps each command minimal and composable.

**`CreateStreamGroup` pre-allocates its UUID.** The `group_id` field is set before `execute` is called so that undo and redo both operate on the same UUID, keeping group identity stable across the history.

**`RenameType` patches all diagram specs.** `DataDictionary::rename` patches composite type definitions; `RenameType::apply` additionally scans every node and stream in every diagram and updates `type_ref` strings directly. `undo` reverses the same scan. This is the authoritative cascade — `ValidationCache::on_type_renamed` is called separately by the app layer after the command executes.

**`CreateSubDiagram::undo` removes by the currently-linked sub-diagram ID**, not the pre-allocated `sub_diagram_id` field. This handles the redo case where `create_sub_diagram` may assign a different UUID (because boundary node UUIDs regenerate). A full snapshot approach will be used in a production hardening pass.

**`err()` private helper resolves type-inference ambiguity.** `format!("{e}").into()` in `map_err` closures is ambiguous when `?` is used — the compiler cannot pick among multiple `Into<CommandError>` impls. The `err(msg: impl Display) -> CommandError` helper makes the target type explicit at one site.

**Concrete commands implemented**: `AddNode`, `DeleteNode`, `MoveNode`, `RenameNode`, `EditNodeSpec`, `AddStream`, `DeleteStream`, `EditStreamSpec`, `CreateStreamGroup`, `DeleteStreamGroup`, `EditStreamGroup`, `AddPrimitiveType`, `AddCompositeType`, `DeleteType`, `RenameType`, `CreateSubDiagram`, `OrphanSubDiagram`, `AttachOrphan`.

---

## 6. Diagram Decomposition

### 6.1 Overview

Any **process** node may have exactly one sub-diagram. The relationship is 1-to-1 (or 1-to-0, or 0-to-1 — a process need not have a sub-diagram, and an orphaned sub-diagram need not have a parent).

Sources and sinks cannot have sub-diagrams.

### 6.2 Boundary Nodes

When a sub-diagram is created for a process, the tool auto-generates **locked boundary nodes**:

- One locked **source** per entry in the parent process's `input_spec`, carrying that entry's `(count, type_ref)` as its `output_spec`. Data entering the process from outside appears as a source inside the sub-diagram.
- One locked **sink** per entry in the parent process's `output_spec`, carrying that entry's `(count, type_ref)` as its `input_spec`. Data leaving the process appears as a sink inside the sub-diagram.

The user cannot delete or rename boundary nodes. All other nodes and streams within the sub-diagram are freely editable.

#### Initial layout
Boundary sources are placed at x=40, y=80+120×i. Boundary sinks are placed at x=800, y=80+120×i. These positions are initial suggestions; auto-layout may reposition them.

### 6.3 Boundary Updates (top-down propagation)

The parent diagram controls the sub-diagram boundary. The sub-diagram cannot change the parent.

When the parent process I/O specification changes, `Project::update_boundary_nodes(diagram_id)` must be called:

- All existing boundary nodes are removed first (via `Diagram::remove_node`, which marks their connected streams dangling).
- New boundary nodes are generated from the current process I/O spec.
- Returns the IDs of all streams that became dangling so the caller can trigger incremental validation.

This is an atomic, undoable action (undo stack wraps the full call).

### 6.4 Parent Process Deletion

When a parent process is deleted, its sub-diagram is moved into the **Orphaned** directory via `Project::orphan_sub_diagram(diagram_id)`:

- `diagram.parent_process_id` is cleared.
- Any `node.sub_diagram_id` pointers still referencing the diagram are cleared (best-effort scan).
- The diagram ID is added to `orphaned_diagram_ids` (idempotent).
- Boundary nodes are preserved but shown in error state by validation.

### 6.5 Re-attaching an Orphaned Sub-diagram

`Project::attach_orphan(orphan_id, process_node_id)` links an orphaned diagram to a new parent process:

- Removes from `orphaned_diagram_ids`.
- Sets `diagram.parent_process_id = process_node_id`.
- Sets `process_node.sub_diagram_id = orphan_id`.
- Does **not** rebuild boundary nodes — the caller must call `update_boundary_nodes` afterwards and use the result to overlay error badges on mismatched boundary nodes.

#### Implementation decisions (phase 2b)
- `attach_orphan` deliberately separates linking from boundary rebuild to keep undo commands simple: undo of `attach_orphan` reverses only two pointer writes.
- `ProjectError` has a `NoParentProcess(Uuid)` variant (not `NodeNotFound(Uuid::nil())`) returned by `update_boundary_nodes` when the diagram has no `parent_process_id`.
- `Project::schema_version: u32` is a struct field (not only a const) so the persistence layer can read and validate it without guessing.
- `BoundaryMismatch { node_id, reason }` is defined on `Project` for future use by the UI error-badge overlay after `attach_orphan`.

---

## 7. File Persistence

Files are saved as XML. Schema version: **1** (written as `schema_version` attribute on the root `<project>` element).

The schema covers:

- Schema version (root attribute)
- Project root structure
- Data dictionary (all primitive and composite type definitions)
- Diagram tree (base diagram + sub-diagrams at n depth, including the orphaned directory)
- Nodes: position, kind (source/sink/process), name, type specification, sub-diagram reference, boundary-node flag
- Streams: source node ID, target node ID, type specification, group membership
- Stream groups: ID, name, member stream IDs
- Viewport state per diagram (zoom level, scroll position) — **resolved: yes, save per diagram**
- Stable UUIDs on all entities (required for draw.io export)

The data dictionary is saved as part of the project file. It may also be exported and imported independently.

#### Implementation decisions (phase 5a)

**Flat diagram list, not a nested tree.** All diagrams (root, sub-diagrams, orphaned) are stored as siblings inside `<diagrams>`. Each `<diagram>` carries a `parent_process_id` attribute (omitted for the root and orphaned diagrams). The reader reconstructs the logical tree from those pointers. This avoids recursive serialisation and matches the in-memory `HashMap<Uuid, Diagram>` representation exactly.

**Optional attributes are omitted when absent.** `sub_diagram_id`, `parent_process_id`, and stream `group_id` are not written when `None`. Boolean flags that default to `false` (`is_boundary`, `is_dangling`) are omitted when false; the deserialiser treats absence as `false`.

**Node positions are stored as `x` and `y` attributes on `<node>`**, not in a separate `<positions>` section. This keeps all node data co-located and avoids a second ID-keyed lookup pass during deserialisation.

**Stream group members are `<member stream_id="..."/>` children of `<group>`**, not a comma-delimited attribute. This avoids UUID list parsing and makes the format more readable and extensible.

**Orphaned diagram IDs are recorded in a dedicated `<orphaned>` element** as `<ref id="..."/>` children, separate from the `<diagrams>` list. This makes the orphaned set explicit without relying on the absence of a `parent_process_id`.

**Empty containers are written as self-closing elements** (e.g. `<streams/>`). This keeps minimal documents compact.

**`NodeKind` serialises as `"Source"`, `"Sink"`, `"Process"`** — defined as `NODE_KIND_*` constants. The deserialiser rejects any other value.

**`ATTR_SCHEMA_VERSION` replaces the old `ATTR_VERSION` constant** from the scaffold. The new name is unambiguous and matches the XML attribute string `"schema_version"` exactly. `ATTR_ROOT_DIAGRAM_ID` is new — the root diagram UUID must be stored explicitly since the flat diagram list has no implied ordering.

**`ATTR_STREAM_ID` is added** for `<member>` elements inside `<group>`, distinguishing a stream reference from the group's own `id` attribute.

**`BOOL_TRUE` and `BOOL_FALSE` constants** are defined for use by the serialiser/deserialiser. `BOOL_FALSE` is not written (absence implies false) but is defined for use in deserialiser comparisons to avoid string literals.

---

## 8. Additional Features

### 8.1 Draw.io Export

The project can be exported to a draw.io-compatible XML file. Scope and shape mapping to be defined, but node and stream IDs must be stable across saves to support round-tripping.

### 8.2 Out of Scope

The following are explicitly not part of this project:

- Simulation or runtime execution
- Code generation
- Maximum limits on nodes or streams (beyond available memory)

---

## 9. Resolved Questions

| # | Item | Resolution |
|---|---|---|
| 3 | Canvas viewport state: save per diagram? | **Yes** — `ViewportState { zoom: f32, scroll: (f32, f32) }` persisted per diagram |
| 5 | Initial project state | **Empty canvas** — no default primitive set pre-populated |

## 10. Open Questions

| # | Item | Priority |
|---|---|---|
| 1 | Draw.io export: single diagram or full project tree? Shape mapping for node types? | Critical |
| 2 | Stream endpoint model: free-form edge attachment vs named ports on nodes | Medium |
| 4 | Copy/paste and multi-select: in scope or deferred? | Medium |
| 6 | Window state persistence: tree pane width, last open diagram, window size | Low |
| 7 | Multi-select and group-move of nodes | Low |


