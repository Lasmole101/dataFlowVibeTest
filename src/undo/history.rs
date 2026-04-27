//! Global undo / redo stack and the `Command` trait (spec §5.5).
//!
//! # Command contract
//!
//! Every mutation of [`Project`] is encapsulated in a [`Command`].
//! Commands are **value types**: they store all data needed to both apply
//! *and* reverse the change deterministically.  Snapshot-based storage
//! (before/after values) is preferred over computing inverses.
//!
//! `apply` is fallible.  If `apply` returns `Err`, [`UndoStack::execute`]
//! does **not** push the command — a failed command leaves no history entry.
//!
//! `undo` is also fallible.  Failure during `undo` indicates a programming
//! error (the project state is inconsistent with the stored command); in
//! debug builds this panics, in release it is logged and the entry is
//! discarded.
//!
//! # Depth limit
//!
//! [`MAX_UNDO_DEPTH`] caps memory use.  When the limit is reached the oldest
//! entry is evicted from the front of `past`.
//!
//! # Compound commands
//!
//! [`CompoundCommand`] bundles several sub-commands into one undoable step.
//! Sub-commands are applied in order and undone in reverse order.  If any
//! sub-command's `apply` fails the compound stops immediately; already-applied
//! sub-commands are rolled back in reverse order before the error is returned.

use crate::model::project::Project;

/// Boxed, thread-safe error returned by [`Command`] operations.
pub type CommandError = Box<dyn std::error::Error + Send + Sync>;

/// Maximum number of commands retained in the undo history.
pub const MAX_UNDO_DEPTH: usize = 200;

// ---------------------------------------------------------------------------
// Command trait
// ---------------------------------------------------------------------------

/// A reversible, self-contained mutation of [`Project`].
pub trait Command: std::fmt::Debug + Send + Sync {
    /// Apply this command to `project`.
    ///
    /// On success the command may be pushed onto the undo stack.
    /// On failure the command must **not** have partially mutated `project`;
    /// callers treat `Err` as a no-op.
    fn apply(&self, project: &mut Project) -> Result<(), CommandError>;

    /// Reverse this command, restoring `project` to its state before
    /// [`apply`] was called.
    ///
    /// A well-implemented undo should be infallible in practice.  `Err` here
    /// signals a programming error.
    fn undo(&self, project: &mut Project) -> Result<(), CommandError>;

    /// Short human-readable description for menus and history panels.
    fn description(&self) -> &str;
}

// ---------------------------------------------------------------------------
// CompoundCommand
// ---------------------------------------------------------------------------

/// A bundle of sub-commands applied and reversed as a single undo step
/// (spec §5.5 — "compound action is a single undoable step").
///
/// On apply: sub-commands run in order.  If one fails, already-applied
/// sub-commands are rolled back in reverse order before the error propagates.
///
/// On undo: sub-commands are reversed in reverse order.
#[derive(Debug)]
pub struct CompoundCommand {
    /// Human-readable label for the whole compound operation.
    pub description: String,
    /// Sub-commands in application order.
    pub commands: Vec<Box<dyn Command>>,
}

impl CompoundCommand {
    /// Create a new compound command.
    pub fn new(description: impl Into<String>, commands: Vec<Box<dyn Command>>) -> Self {
        Self {
            description: description.into(),
            commands,
        }
    }

    /// Add a sub-command.
    pub fn push(&mut self, cmd: Box<dyn Command>) {
        self.commands.push(cmd);
    }
}

impl Command for CompoundCommand {
    fn apply(&self, project: &mut Project) -> Result<(), CommandError> {
        let mut applied = 0usize;
        for cmd in &self.commands {
            if let Err(e) = cmd.apply(project) {
                // Roll back already-applied sub-commands.
                for rollback in self.commands[..applied].iter().rev() {
                    let _ = rollback.undo(project); // best-effort; ignore errors
                }
                return Err(e);
            }
            applied += 1;
        }
        Ok(())
    }

    fn undo(&self, project: &mut Project) -> Result<(), CommandError> {
        let mut first_err: Option<CommandError> = None;
        for cmd in self.commands.iter().rev() {
            if let Err(e) = cmd.undo(project) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
                // Continue undoing remaining sub-commands even on error.
            }
        }
        match first_err {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// The global undo / redo stack.
///
/// Owned by [`crate::app::AppState`].  Every UI action that modifies the
/// project calls [`execute`](UndoStack::execute) rather than mutating the
/// project directly.
#[derive(Debug, Default)]
pub struct UndoStack {
    /// Applied commands, oldest first.  The last entry is the next to undo.
    past: Vec<Box<dyn Command>>,
    /// Undone commands, most-recently-undone first (top of redo stack).
    future: Vec<Box<dyn Command>>,
}

impl UndoStack {
    // -----------------------------------------------------------------------
    // Mutation
    // -----------------------------------------------------------------------

    /// Apply `cmd` to `project` and push it onto the undo stack.
    ///
    /// Clears the redo history (any branch from this point is now the
    /// only branch).
    ///
    /// If `apply` returns `Err` the command is **not** pushed and the error
    /// is returned to the caller; the redo history is also left intact.
    ///
    /// Evicts the oldest entry when the stack exceeds [`MAX_UNDO_DEPTH`].
    pub fn execute(
        &mut self,
        cmd: Box<dyn Command>,
        project: &mut Project,
    ) -> Result<(), CommandError> {
        cmd.apply(project)?;
        self.future.clear();
        self.past.push(cmd);
        if self.past.len() > MAX_UNDO_DEPTH {
            self.past.remove(0);
        }
        Ok(())
    }

    /// Undo the most-recent command.
    ///
    /// Does nothing (returns `Ok`) if the stack is empty.
    /// On error the command is discarded from `past` rather than pushed
    /// to `future` — the history is truncated at the point of failure.
    pub fn undo(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let Some(cmd) = self.past.pop() else {
            return Ok(());
        };
        match cmd.undo(project) {
            Ok(()) => {
                self.future.push(cmd);
                Ok(())
            }
            Err(e) => {
                // Discard the command — state may be inconsistent.
                Err(e)
            }
        }
    }

    /// Redo the most-recently-undone command.
    ///
    /// Does nothing (returns `Ok`) if there is nothing to redo.
    pub fn redo(&mut self, project: &mut Project) -> Result<(), CommandError> {
        let Some(cmd) = self.future.pop() else {
            return Ok(());
        };
        match cmd.apply(project) {
            Ok(()) => {
                self.past.push(cmd);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Returns `true` if there is at least one command to undo.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Returns `true` if there is at least one command to redo.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Description of the command that would be undone next, if any.
    pub fn undo_description(&self) -> Option<&str> {
        self.past.last().map(|c| c.description())
    }

    /// Description of the command that would be redone next, if any.
    pub fn redo_description(&self) -> Option<&str> {
        self.future.last().map(|c| c.description())
    }

    /// Total number of undoable steps.
    pub fn undo_depth(&self) -> usize {
        self.past.len()
    }

    /// Total number of redoable steps.
    pub fn redo_depth(&self) -> usize {
        self.future.len()
    }

    /// Clear both stacks (called when loading a new project).
    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }
}
