//! Balance validation for streams and nodes (spec §4.2).
//!
//! A component is *balanced* when the sum of its incoming/outgoing stream
//! types matches its declared specification exactly.  Imbalance is reported
//! as a warning; missing types or dangling connections are errors.

use uuid::Uuid;

use crate::model::{diagram::Diagram, dictionary::DataDictionary};

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Imbalance — node/stream will not function correctly but the structure
    /// is not broken.
    Warning,
    /// Structural break — missing type, dangling stream, or boundary mismatch.
    Error,
}

/// A single validation finding attached to a component.
#[derive(Debug, Clone)]
pub struct Finding {
    /// The component (node or stream ID) this finding is attached to.
    pub component_id: Uuid,
    /// Severity of the finding.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
}

/// Check stream balance for a single stream.
///
/// A stream is unbalanced if its source does not output all types the stream
/// carries, or if its target does not accept all types the stream carries
/// (spec §4.2, stream balance).
pub fn check_stream_balance(
    stream_id: Uuid,
    diagram: &Diagram,
    dict: &DataDictionary,
) -> Vec<Finding> {
    todo!("Phase 3 — balance::check_stream_balance")
}

/// Check input balance for a single node.
///
/// The sum of incoming stream types must exactly match the node's input
/// specification (spec §4.2, node input balance).
pub fn check_node_input_balance(
    node_id: Uuid,
    diagram: &Diagram,
    dict: &DataDictionary,
) -> Vec<Finding> {
    todo!("Phase 3 — balance::check_node_input_balance")
}

/// Check output balance for a single node.
///
/// The sum of outgoing stream types must not exceed the node's output
/// specification; over- or under-consumption both produce warnings
/// (spec §4.2, node output balance).
pub fn check_node_output_balance(
    node_id: Uuid,
    diagram: &Diagram,
    dict: &DataDictionary,
) -> Vec<Finding> {
    todo!("Phase 3 — balance::check_node_output_balance")
}
