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

---

## 5. User Interface

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
