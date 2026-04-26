//! Balance validation for streams and nodes (spec §4.2).
//!
//! # Balance model
//!
//! The spec defines balance at two levels:
//!
//! **Stream balance** — a stream is valid when:
//! * The source node's `output_spec` contains at least as many of each
//!   primitive as the stream carries.
//! * The target node's `input_spec` contains at least as many of each
//!   primitive as the stream carries.
//!
//! This is a *containment* check, not an exact-match check.  A source may
//! output more than a single stream carries (it may fan out); the node-level
//! check handles over-consumption.
//!
//! **Node balance** — a node is balanced when the *sum* of all its incoming
//! (or outgoing) stream specs exactly matches its declared `input_spec`
//! (or `output_spec`) after flattening.  Both under- and over-supply are
//! warnings.
//!
//! # Types
//!
//! [`BalanceResult`] is the return value of every check function.  It is
//! self-contained and testable without a live diagram.
//!
//! [`Finding`] and [`Severity`] are the storage types used by
//! [`crate::validation::validator::ValidationCache`].  The cache converts
//! `BalanceResult` → `Finding`s, attaching the component UUID.
//!
//! # Dangling streams
//!
//! Dangling streams (where a node was deleted) are detected and reported as
//! errors before any balance computation is attempted.  The balance functions
//! below assume their inputs are non-dangling; the orchestration layer
//! ([`crate::validation::validator`]) skips balance checks for dangling
//! components and emits an error finding directly.

use std::collections::HashMap;

use uuid::Uuid;

use crate::{
    model::{dictionary::DataDictionary, node::Node, stream::Stream, types::TypeError},
    validation::flatten::{flatten_spec, is_balanced, subtract_specs, sum_specs},
};

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Imbalance — node/stream will not function correctly but the structure
    /// is not broken.  The user should fix this but the diagram is loadable.
    Warning,
    /// Structural break — missing type, dangling stream, or boundary mismatch.
    /// The diagram cannot be considered complete.
    Error,
}

// ---------------------------------------------------------------------------
// BalanceResult
// ---------------------------------------------------------------------------

/// The result of a single balance check.
///
/// Returned by every `check_*` function in this module.  The caller (usually
/// [`crate::validation::validator::ValidationCache`]) converts this into
/// [`Finding`]s by attaching a component UUID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BalanceResult {
    /// The component is perfectly balanced — no issues found.
    Ok,
    /// One or more balance warnings.  The component is usable but the data
    /// flow is not fully balanced.
    Warning(Vec<String>),
    /// One or more structural errors.  The component is broken.
    Error(Vec<String>),
}

impl BalanceResult {
    /// Returns `true` if this result is [`BalanceResult::Ok`].
    pub fn is_ok(&self) -> bool {
        matches!(self, BalanceResult::Ok)
    }

    /// The worst [`Severity`] present in this result, or `None` for `Ok`.
    pub fn severity(&self) -> Option<Severity> {
        match self {
            BalanceResult::Ok => None,
            BalanceResult::Warning(_) => Some(Severity::Warning),
            BalanceResult::Error(_) => Some(Severity::Error),
        }
    }

    /// Merge `other` into `self`, combining messages and taking the worst
    /// severity.
    pub fn merge(self, other: BalanceResult) -> BalanceResult {
        match (self, other) {
            (BalanceResult::Ok, r) | (r, BalanceResult::Ok) => r,
            (BalanceResult::Error(mut a), BalanceResult::Error(b)) => {
                a.extend(b);
                BalanceResult::Error(a)
            }
            (BalanceResult::Error(mut e), BalanceResult::Warning(w))
            | (BalanceResult::Warning(w), BalanceResult::Error(mut e)) => {
                e.extend(w);
                BalanceResult::Error(e)
            }
            (BalanceResult::Warning(mut a), BalanceResult::Warning(b)) => {
                a.extend(b);
                BalanceResult::Warning(a)
            }
        }
    }

    /// Collect messages from this result.  Returns an empty slice for `Ok`.
    pub fn messages(&self) -> &[String] {
        match self {
            BalanceResult::Ok => &[],
            BalanceResult::Warning(v) => v,
            BalanceResult::Error(v) => v,
        }
    }
}

// ---------------------------------------------------------------------------
// Finding  (used by ValidationCache)
// ---------------------------------------------------------------------------

/// A single validation finding attached to a specific component by UUID.
///
/// Produced by the orchestration layer when it converts [`BalanceResult`]s
/// into cacheable findings.
#[derive(Debug, Clone)]
pub struct Finding {
    /// The component (node or stream ID) this finding is attached to.
    pub component_id: Uuid,
    /// Severity of this finding.
    pub severity: Severity,
    /// Human-readable description shown in the UI tooltip.
    pub message: String,
}

impl Finding {
    /// Convenience constructor.
    pub fn new(component_id: Uuid, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            component_id,
            severity,
            message: message.into(),
        }
    }

    /// Convert a [`BalanceResult`] into a `Vec<Finding>`, attaching
    /// `component_id` to each message.
    pub fn from_balance_result(component_id: Uuid, result: BalanceResult) -> Vec<Finding> {
        match result {
            BalanceResult::Ok => vec![],
            BalanceResult::Warning(msgs) => msgs
                .into_iter()
                .map(|m| Finding::new(component_id, Severity::Warning, m))
                .collect(),
            BalanceResult::Error(msgs) => msgs
                .into_iter()
                .map(|m| Finding::new(component_id, Severity::Error, m))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Format a flat primitive map as a human-readable string for messages.
/// e.g. `{uint32_t: 2, char: 1}`
fn fmt_flat(map: &HashMap<String, u32>) -> String {
    let mut pairs: Vec<(&String, &u32)> = map.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str()); // deterministic order in messages
    let inner: Vec<String> = pairs
        .into_iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect();
    format!("{{{}}}", inner.join(", "))
}

#[allow(dead_code)]
/// Format a signed delta map for over/under-supply messages.
/// Positive entries are surplus; negative are deficit.
fn fmt_delta(delta: &HashMap<String, i64>) -> String {
    let mut pairs: Vec<(&String, &i64)> = delta.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str());
    let inner: Vec<String> = pairs
        .into_iter()
        .map(|(k, v)| {
            if *v > 0 {
                format!("{k}: +{v} (surplus)")
            } else {
                format!("{k}: {v} (deficit)")
            }
        })
        .collect();
    inner.join(", ")
}

#[allow(dead_code)]
/// Build warning messages from a delta map.
/// Positive → oversupply; negative → undersupply.
fn warnings_from_delta(direction: &str, delta: &HashMap<String, i64>) -> Vec<String> {
    let mut pairs: Vec<(&String, &i64)> = delta.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str());
    pairs
        .into_iter()
        .map(|(prim, &d)| {
            if d > 0 {
                format!(
                    "{direction}: oversupply of '{prim}' by {d} \
                     (stream carries more than node spec)"
                )
            } else {
                format!(
                    "{direction}: undersupply of '{prim}' by {} \
                     (stream carries less than node spec)",
                    d.unsigned_abs()
                )
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// stream_balance
// ---------------------------------------------------------------------------

/// Check whether `stream` is balanced relative to its source and target nodes.
///
/// Two independent checks are performed (spec §4.2, stream balance):
///
/// **Source side** — the source node's `output_spec` must contain at least
/// as many of each primitive as the stream carries.  If the source outputs
/// less than the stream expects, the stream is over-drawing from the source.
///
/// **Target side** — the target node's `input_spec` must contain at least
/// as many of each primitive as the stream carries.  If the target accepts
/// less than the stream carries, the stream is delivering more than the
/// target can accept.
///
/// A mismatch on either side produces a **warning** (not an error — the
/// topology is intact; only the counts are wrong).
///
/// # Errors
/// Returns `BalanceResult::Error` if any type name in any spec cannot be
/// resolved through `dict`.
pub fn stream_balance(
    stream: &Stream,
    source_node: &Node,
    target_node: &Node,
    dict: &DataDictionary,
) -> BalanceResult {
    // Flatten all three specs.  An UnknownType in any of them is an error.
    let stream_flat = match flatten_spec(&stream.type_spec, dict) {
        Ok(f) => f,
        Err(TypeError::UnknownType(t)) => {
            return BalanceResult::Error(vec![format!(
                "stream type spec references unknown type '{t}'"
            )])
        }
        Err(e) => return BalanceResult::Error(vec![e.to_string()]),
    };
    let src_out_flat = match flatten_spec(source_node.effective_output_spec(), dict) {
        Ok(f) => f,
        Err(TypeError::UnknownType(t)) => {
            return BalanceResult::Error(vec![format!(
                "source node '{}' output spec references unknown type '{t}'",
                source_node.name
            )])
        }
        Err(e) => return BalanceResult::Error(vec![e.to_string()]),
    };
    let tgt_in_flat = match flatten_spec(target_node.effective_input_spec(), dict) {
        Ok(f) => f,
        Err(TypeError::UnknownType(t)) => {
            return BalanceResult::Error(vec![format!(
                "target node '{}' input spec references unknown type '{t}'",
                target_node.name
            )])
        }
        Err(e) => return BalanceResult::Error(vec![e.to_string()]),
    };

    let mut warnings: Vec<String> = Vec::new();

    // Source-side check: stream must not carry more of any primitive than the
    // source node provides.  Delta = stream_flat − src_out_flat; positive
    // entries mean the stream over-draws from the source.
    let src_delta = subtract_specs(&stream_flat, &src_out_flat);
    for (prim, &d) in &src_delta {
        if d > 0 {
            warnings.push(format!(
                "stream over-draws from source '{}': \
                 stream needs {need} '{prim}' but source only provides {avail}",
                source_node.name,
                need = stream_flat.get(prim).copied().unwrap_or(0),
                avail = src_out_flat.get(prim).copied().unwrap_or(0),
            ));
        }
        // Negative entries mean the source provides more than the stream uses
        // on this stream — fine at the stream level (other streams may consume
        // the remainder; node output balance handles over-consumption globally).
    }

    // Target-side check: stream must not carry more of any primitive than the
    // target node accepts.
    let tgt_delta = subtract_specs(&stream_flat, &tgt_in_flat);
    for (prim, &d) in &tgt_delta {
        if d > 0 {
            warnings.push(format!(
                "stream over-delivers to target '{}': \
                 stream carries {carries} '{prim}' but target only accepts {accepts}",
                target_node.name,
                carries = stream_flat.get(prim).copied().unwrap_or(0),
                accepts = tgt_in_flat.get(prim).copied().unwrap_or(0),
            ));
        }
    }

    if warnings.is_empty() {
        BalanceResult::Ok
    } else {
        BalanceResult::Warning(warnings)
    }
}

// ---------------------------------------------------------------------------
// node_input_balance
// ---------------------------------------------------------------------------

/// Check whether the sum of `incoming_streams` exactly matches `node`'s
/// `input_spec` (spec §4.2, node input balance).
///
/// Both undersupply and oversupply produce **warnings**.
///
/// Source nodes have an empty `input_spec`; if `incoming_streams` is also
/// empty the result is always `Ok`.
///
/// # Errors
/// Returns `BalanceResult::Error` if any type name cannot be resolved.
pub fn node_input_balance(
    node: &Node,
    incoming_streams: &[&Stream],
    dict: &DataDictionary,
) -> BalanceResult {
    // Flatten node input spec.
    let required = match flatten_spec(node.effective_input_spec(), dict) {
        Ok(f) => f,
        Err(TypeError::UnknownType(t)) => {
            return BalanceResult::Error(vec![format!(
                "node '{}' input spec references unknown type '{t}'",
                node.name
            )])
        }
        Err(e) => return BalanceResult::Error(vec![e.to_string()]),
    };

    // Flatten and sum all incoming stream specs.
    let mut stream_flats: Vec<HashMap<String, u32>> = Vec::with_capacity(incoming_streams.len());
    for stream in incoming_streams {
        match flatten_spec(&stream.type_spec, dict) {
            Ok(f) => stream_flats.push(f),
            Err(TypeError::UnknownType(t)) => {
                return BalanceResult::Error(vec![format!(
                    "incoming stream type spec references unknown type '{t}'"
                )])
            }
            Err(e) => return BalanceResult::Error(vec![e.to_string()]),
        }
    }
    let supplied = sum_specs(stream_flats.iter());

    if is_balanced(&supplied, &required) {
        return BalanceResult::Ok;
    }

    // supplied − required: positive = oversupply, negative = undersupply.
    let delta = subtract_specs(&supplied, &required);
    let mut warnings: Vec<String> = Vec::new();

    let mut keys: Vec<&String> = delta.keys().collect();
    keys.sort();
    for prim in keys {
        let d = delta[prim];
        if d > 0 {
            warnings.push(format!(
                "input oversupply: node '{}' requires {req} '{prim}' \
                 but incoming streams supply {got} (excess: {d})",
                node.name,
                req = required.get(prim).copied().unwrap_or(0),
                got = supplied.get(prim).copied().unwrap_or(0),
            ));
        } else {
            warnings.push(format!(
                "input undersupply: node '{}' requires {req} '{prim}' \
                 but incoming streams only supply {got} (deficit: {deficit})",
                node.name,
                req = required.get(prim).copied().unwrap_or(0),
                got = supplied.get(prim).copied().unwrap_or(0),
                deficit = d.unsigned_abs(),
            ));
        }
    }

    BalanceResult::Warning(warnings)
}

// ---------------------------------------------------------------------------
// node_output_balance
// ---------------------------------------------------------------------------

/// Check whether the sum of `outgoing_streams` exactly matches `node`'s
/// `output_spec` (spec §4.2, node output balance).
///
/// Both under-consumption and over-consumption produce **warnings**.
///
/// Sink nodes have an empty `output_spec`; if `outgoing_streams` is also
/// empty the result is always `Ok`.
///
/// # Errors
/// Returns `BalanceResult::Error` if any type name cannot be resolved.
pub fn node_output_balance(
    node: &Node,
    outgoing_streams: &[&Stream],
    dict: &DataDictionary,
) -> BalanceResult {
    // Flatten node output spec.
    let provided = match flatten_spec(node.effective_output_spec(), dict) {
        Ok(f) => f,
        Err(TypeError::UnknownType(t)) => {
            return BalanceResult::Error(vec![format!(
                "node '{}' output spec references unknown type '{t}'",
                node.name
            )])
        }
        Err(e) => return BalanceResult::Error(vec![e.to_string()]),
    };

    // Flatten and sum all outgoing stream specs.
    let mut stream_flats: Vec<HashMap<String, u32>> = Vec::with_capacity(outgoing_streams.len());
    for stream in outgoing_streams {
        match flatten_spec(&stream.type_spec, dict) {
            Ok(f) => stream_flats.push(f),
            Err(TypeError::UnknownType(t)) => {
                return BalanceResult::Error(vec![format!(
                    "outgoing stream type spec references unknown type '{t}'"
                )])
            }
            Err(e) => return BalanceResult::Error(vec![e.to_string()]),
        }
    }
    let consumed = sum_specs(stream_flats.iter());

    if is_balanced(&consumed, &provided) {
        return BalanceResult::Ok;
    }

    // consumed − provided: positive = over-consumption, negative = under-consumption.
    let delta = subtract_specs(&consumed, &provided);
    let mut warnings: Vec<String> = Vec::new();

    let mut keys: Vec<&String> = delta.keys().collect();
    keys.sort();
    for prim in keys {
        let d = delta[prim];
        if d > 0 {
            warnings.push(format!(
                "output over-consumed: node '{}' provides {prov} '{prim}' \
                 but outgoing streams consume {con} (excess: {d})",
                node.name,
                prov = provided.get(prim).copied().unwrap_or(0),
                con = consumed.get(prim).copied().unwrap_or(0),
            ));
        } else {
            warnings.push(format!(
                "output under-consumed: node '{}' provides {prov} '{prim}' \
                 but outgoing streams only consume {con} (deficit: {deficit})",
                node.name,
                prov = provided.get(prim).copied().unwrap_or(0),
                con = consumed.get(prim).copied().unwrap_or(0),
                deficit = d.unsigned_abs(),
            ));
        }
    }

    BalanceResult::Warning(warnings)
}
