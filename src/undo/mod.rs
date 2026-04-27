//! Undo / redo subsystem (spec §5.5).
//!
//! A single global [`history::UndoStack`] covers all changes across all
//! diagram levels.  Compound actions (e.g. updating a parent process I/O and
//! cascading the sub-diagram boundary update) are committed as a single
//! undoable step.

pub mod commands;
pub mod history;
