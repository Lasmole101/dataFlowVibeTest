# Data Flow Diagramming Tool — Project Specification

## 1. Overview

A standalone desktop data flow diagramming tool compiled for Windows and Linux.

| Property | Value |
|---|---|
| Language | Rust |
| UI framework | egui |
| Persistence | XML (schema to be defined) |
| Target platforms | Windows, Linux |

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
- Names must be unique. An error is shown if the user attempts to create a type with a name already in use.
- Renaming a type propagates to all referencing nodes and streams across all diagrams.
- Deleting a type places all referencing components into an **error state**.

---

## 3. Diagram Elements

### 3.1 Nodes

Each node has a **type**, a **name**, and a **type specification** (a multiset of (count, type) pairs describing what it inputs and/or outputs).

| Node type | Inputs | Outputs |
|---|---|---|
| Source | None | One or more types |
| Sink | One or more types | None |
| Process | One or more types | One or more types |

Nodes cannot connect to themselves.

### 3.2 Streams

A stream is a point-to-point, unidirectional connection between two nodes. It carries a **type specification** — a multiset of (count, type) pairs — which is mutable via the specification panel.

- Multiple streams may leave or enter a single node (fan-out and fan-in are permitted).
- Cycles in the graph are permitted.

### 3.3 Stream Groups

Streams may be grouped for display purposes only. Grouping has no effect on validation or semantics.

**Creating a group:** Select two or more streams → right-click → "Create group" → enter a name. Grouped streams are displayed as parallel lines.

**Editing a group:** Streams may be added to or removed from an existing group. If all member streams are deleted the group dissolves automatically.

---

## 4. Validation

### 4.1 Compatibility

Type compatibility uses **structural equality**: two type expressions are compatible if they flatten to the same multiset of primitives. Names are irrelevant.

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

- One locked **source** node per item in the parent process's input specification, carrying the matching type set.
- One locked **sink** node per item in the parent process's output specification, carrying the matching type set.

The user cannot delete or rename boundary nodes. All other nodes and streams within the sub-diagram are freely editable.

### 6.3 Boundary Updates (top-down propagation)

The parent diagram controls the sub-diagram boundary. The sub-diagram cannot change the parent.

When the parent process I/O specification changes:

- Boundary nodes are updated to match as a single atomic, undoable action.
- If a boundary node is removed (because its corresponding type was removed from the parent), any streams connected to it become dangling and enter an error state.

### 6.4 Parent Process Deletion

When a parent process is deleted, its sub-diagram is moved into the **Orphaned** directory in the tree pane. The boundary nodes are preserved but shown in an error state.

### 6.5 Re-attaching an Orphaned Sub-diagram

Right-clicking any process node shows a **"Set sub-diagram"** option. This lets the user assign an orphaned sub-diagram to that process. After assignment:

- The tree and sub-diagram name are updated to reflect the new parent.
- If the orphan's boundary nodes do not match the target process's I/O, the mismatched boundary nodes enter an error state. The user resolves this by editing the process I/O or the orphan's internal layout.

---

## 7. File Persistence

Files are saved as XML. A schema must be defined covering:

- Schema version (root attribute)
- Project root structure
- Data dictionary (all primitive and composite type definitions)
- Diagram tree (base diagram + sub-diagrams at n depth, including the orphaned directory)
- Nodes: position, type (source/sink/process), name, type specification, sub-diagram reference, boundary-node flag
- Streams: source node ID, target node ID, type specification, group membership
- Stream groups: ID, name, member stream IDs
- Viewport state per diagram (zoom level, scroll position)
- Stable IDs on all entities (required for draw.io export)

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

## 9. Open Questions

The following items require decisions before implementation begins:

| # | Item | Priority |
|---|---|---|
| 1 | Draw.io export: single diagram or full project tree? Shape mapping for node types? | Critical |
| 2 | Stream endpoint model: free-form edge attachment vs named ports on nodes | Medium |
| 3 | Canvas viewport state: should zoom/scroll be saved per diagram in the XML? | Medium |
| 4 | Copy/paste and multi-select: in scope or deferred? | Medium |
| 5 | Initial project state: empty canvas, or a default primitive set pre-populated? | Medium |
| 6 | Window state persistence: tree pane width, last open diagram, window size | Low |
| 7 | Multi-select and group-move of nodes | Low |
