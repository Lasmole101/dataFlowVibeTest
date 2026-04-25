//! Global undo / redo stack (spec §5.5).
//!
//! Every mutation to the project goes through a [`Command`].  Commands are
//! pushed onto [`UndoStack`], which keeps a bounded history.  Calling
//! [`UndoStack::undo`] reverses the most-recent command; [`UndoStack::redo`]
//! re-applies it.
//!
//! Compound actions are represented by [`CompoundCommand`], which bundles
//! multiple sub-commands into a single undoable step.

use crate::model::project::Project;

/// Maximum number of commands retained in the undo history.
/// Oldest commands are evicted first (FIFO) once this limit is reached.
pub const MAX_UNDO_DEPTH: usize = 200;

// ---------------------------------------------------------------------------
// Command trait
// ---------------------------------------------------------------------------

/// A reversible mutation of [`Project`].
///
/// Implementors must be able to both apply and reverse their change
/// deterministically.
pub trait Command: std::fmt::Debug + Send + Sync {
    /// Apply this command to `project`.
    fn apply(&self, project: &mut Project);

    /// Reverse this command, restoring `project` to its state before
    /// [`apply`](Command::apply) was called.
    fn undo(&self, project: &mut Project);

    /// Short human-readable label shown in a hypothetical history panel.
    fn label(&self) -> &str;
}

// ---------------------------------------------------------------------------
// CompoundCommand
// ---------------------------------------------------------------------------

/// A compound action: multiple sub-commands applied and reversed atomically.
///
/// Sub-commands are applied in order and reversed in reverse order.
#[derive(Debug)]
pub struct CompoundCommand {
    /// The human-readable label for the whole compound action.
    pub label: String,
    /// Sub-commands, stored in application order.
    pub commands: Vec<Box<dyn Command>>,
}

impl Command for CompoundCommand {
    fn apply(&self, project: &mut Project) {
        for cmd in &self.commands {
            cmd.apply(project);
        }
    }

    fn undo(&self, project: &mut Project) {
        for cmd in self.commands.iter().rev() {
            cmd.undo(project);
        }
    }

    fn label(&self) -> &str {
        &self.label
    }
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// The global undo / redo stack.
#[derive(Debug, Default)]
pub struct UndoStack {
    /// Commands that can be undone, ordered oldest-first.
    past: Vec<Box<dyn Command>>,
    /// Commands that can be redone, ordered most-recently-undone first.
    future: Vec<Box<dyn Command>>,
}

impl UndoStack {
    /// Push a command onto the stack, clearing the redo history.
    ///
    /// If the stack has reached [`MAX_UNDO_DEPTH`], the oldest entry is
    /// evicted to bound memory use.
    pub fn push(&mut self, cmd: Box<dyn Command>, project: &mut Project) {
        cmd.apply(project);
        self.future.clear();
        self.past.push(cmd);
        if self.past.len() > MAX_UNDO_DEPTH {
            self.past.remove(0);
        }
    }

    /// Undo the most-recent command.  Does nothing if the stack is empty.
    pub fn undo(&mut self, project: &mut Project) {
        if let Some(cmd) = self.past.pop() {
            cmd.undo(project);
            self.future.push(cmd);
        }
    }

    /// Redo the most-recently-undone command.  Does nothing if there is
    /// nothing to redo.
    pub fn redo(&mut self, project: &mut Project) {
        if let Some(cmd) = self.future.pop() {
            cmd.apply(project);
            self.past.push(cmd);
        }
    }

    /// Returns `true` if there is at least one command to undo.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Returns `true` if there is at least one command to redo.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Clear both stacks (used when loading a new project).
    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }

    /// Label of the command that would be undone next, if any.
    pub fn undo_label(&self) -> Option<&str> {
        self.past.last().map(|c| c.label())
    }

    /// Label of the command that would be redone next, if any.
    pub fn redo_label(&self) -> Option<&str> {
        self.future.last().map(|c| c.label())
    }
}
