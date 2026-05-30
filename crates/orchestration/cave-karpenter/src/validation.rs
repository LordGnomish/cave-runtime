// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! NodeClaim / requirement / taint validation — port of
//! `pkg/apis/v1/nodeclaim_validation.go` from kubernetes-sigs/karpenter
//! v1.12.1 (sha ed490e8). Apache-2.0 upstream; see NOTICE.
//!
//! This is the pure, cloud-agnostic validation surface the admission webhook
//! and the NodePool readiness controller both lean on. The
//! controller-runtime `log.FromContext(ctx)` call in `validateWellKnownValues`
//! is replaced by a silent "proceed with valid values" path — invalid-but-
//! tolerated values are not an error, so dropping the log line changes no
//! behaviour. The webhook plumbing itself (`Validate`, CEL conversion) is
//! owned by cave-admission and is out of scope here.
//!
//! The two `k8s.io/apimachinery/pkg/util/validation` helpers Karpenter calls
//! — [`is_qualified_name`] and [`is_valid_label_value`] (plus the
//! `IsDNS1123Subdomain` prefix check they delegate to) — are ported inline so
//! the crate stays dependency-free; they reproduce the upstream regex
//! semantics by hand rather than pulling a regex engine.

use std::fmt;

use crate::labels::{is_restricted_label, well_known_values_for, WELL_KNOWN_LABELS};
use crate::scheduling::requirement::{normalized_label, NodeSelectorRequirement, Operator};
use crate::scheduling::taints::{Effect, Taint};

// ── Supported-value sets (upstream package-level `sets.NewString`) ────────────

/// `SupportedNodeSelectorOps` — every operator the scheduler understands.
pub const SUPPORTED_NODE_SELECTOR_OPS: &[&str] = &[
    "In",
    "NotIn",
    "Exists",
    "DoesNotExist",
    "Gt",
    "Lt",
    "Gte",
    "Lte",
];

/// `SupportedReservedResources` — resources a NodePool may reserve via kubelet.
pub const SUPPORTED_RESERVED_RESOURCES: &[&str] =
    &["cpu", "memory", "ephemeral-storage", "pid"];

/// `SupportedEvictionSignals` — kubelet eviction-threshold signal names.
pub const SUPPORTED_EVICTION_SIGNALS: &[&str] = &[
    "memory.available",
    "nodefs.available",
    "nodefs.inodesFree",
    "imagefs.available",
    "imagefs.inodesFree",
    "pid.available",
];

// ── Aggregated error (mirrors go.uber.org/multierr) ──────────────────────────

/// Aggregates every validation failure, mirroring upstream's `multierr.Combine`
/// / `multierr.Append`: validation does not short-circuit on the first problem,
/// it collects them all so the caller surfaces a complete diagnosis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationError {
    messages: Vec<String>,
}

impl ValidationError {
    pub(crate) fn append(&mut self, msg: impl Into<String>) {
        self.messages.push(msg.into());
    }

    /// Absorb another error's messages (mirrors `multierr.Append` over a
    /// nested combine). Used by the fan-out validators.
    pub(crate) fn absorb(&mut self, other: ValidationError) {
        self.messages.extend(other.messages);
    }

    /// The individual failure messages, in append order.
    pub fn messages(&self) -> &[String] {
        &self.messages
    }

    /// `true` when nothing failed.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub(crate) fn into_result(self) -> Result<(), ValidationError> {
        if self.messages.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, m) in self.messages.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            f.write_str(m)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

// ── k8s.io/apimachinery/pkg/util/validation helpers ──────────────────────────

const QUALIFIED_NAME_MAX_LENGTH: usize = 63;
const LABEL_VALUE_MAX_LENGTH: usize = 63;
const DNS1123_SUBDOMAIN_MAX_LENGTH: usize = 253;
const DNS1123_LABEL_MAX_LENGTH: usize = 63;

fn is_alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// `qnameExtChar` = `[-A-Za-z0-9_.]`.
fn is_qname_ext_char(c: char) -> bool {
    is_alnum(c) || c == '-' || c == '_' || c == '.'
}

/// Matches the upstream `qualifiedNameFmt` regex
/// `([A-Za-z0-9][-A-Za-z0-9_.]*)?[A-Za-z0-9]`: starts and ends with an
/// alphanumeric, interior characters from the extended set.
fn matches_qualified_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_alnum(first) {
        return false;
    }
    let bytes: Vec<char> = name.chars().collect();
    let last = *bytes.last().expect("non-empty checked above");
    if !is_alnum(last) {
        return false;
    }
    name.chars().all(is_qname_ext_char)
}

/// Matches the upstream `labelValueFmt` regex
/// `(([A-Za-z0-9][-A-Za-z0-9_.]*)?[A-Za-z0-9])?`: like a qualified name but the
/// empty string is also valid.
fn matches_label_value(value: &str) -> bool {
    value.is_empty() || matches_qualified_name(value)
}

/// One DNS-1123 label (`[a-z0-9]([-a-z0-9]*[a-z0-9])?`): lowercase alphanumeric,
/// interior dashes, starts and ends alphanumeric.
fn matches_dns1123_label(label: &str) -> bool {
    if label.is_empty() {
        return false;
    }
    let chars: Vec<char> = label.chars().collect();
    let first = chars[0];
    let last = chars[chars.len() - 1];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return false;
    }
    label
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `IsDNS1123Subdomain`: dot-joined DNS-1123 labels, ≤253 chars total.
fn is_dns1123_subdomain(value: &str) -> Vec<String> {
    let mut errs = Vec::new();
    if value.len() > DNS1123_SUBDOMAIN_MAX_LENGTH {
        errs.push(format!(
            "must be no more than {DNS1123_SUBDOMAIN_MAX_LENGTH} characters"
        ));
    }
    let valid = !value.is_empty()
        && value.split('.').all(|label| {
            label.len() <= DNS1123_LABEL_MAX_LENGTH && matches_dns1123_label(label)
        });
    if !valid {
        errs.push(format!(
            "a lowercase RFC 1123 subdomain must consist of lower case alphanumeric \
             characters, '-' or '.', and must start and end with an alphanumeric \
             character (prefix '{value}' is invalid)"
        ));
    }
    errs
}

/// `IsQualifiedName` (k8s apimachinery). Returns the list of failures — empty
/// means the name is valid. A qualified name is `[prefix/]name` where the
/// optional prefix is a DNS-1123 subdomain and the name part is ≤63 chars and
/// matches the qualified-name regex.
pub fn is_qualified_name(value: &str) -> Vec<String> {
    let mut errs = Vec::new();
    let parts: Vec<&str> = value.split('/').collect();
    let name: &str = match parts.len() {
        1 => parts[0],
        2 => {
            let prefix = parts[0];
            if prefix.is_empty() {
                errs.push("prefix part must be non-empty".to_string());
            } else {
                errs.extend(is_dns1123_subdomain(prefix));
            }
            parts[1]
        }
        _ => {
            errs.push(format!(
                "a qualified name must consist of alphanumeric characters, '-', '_' or '.', \
                 and must start and end with an alphanumeric character with an optional DNS \
                 subdomain prefix and '/' (e.g. 'MyName' or 'example.com/MyName'), got '{value}'"
            ));
            return errs;
        }
    };

    if name.is_empty() {
        errs.push("name part must be non-empty".to_string());
    } else {
        if name.len() > QUALIFIED_NAME_MAX_LENGTH {
            errs.push(format!(
                "name part must be no more than {QUALIFIED_NAME_MAX_LENGTH} characters"
            ));
        }
        if !matches_qualified_name(name) {
            errs.push(format!(
                "name part must consist of alphanumeric characters, '-', '_' or '.', and \
                 must start and end with an alphanumeric character (e.g. 'MyName'), got '{name}'"
            ));
        }
    }
    errs
}

/// `IsValidLabelValue` (k8s apimachinery). Empty is valid; otherwise ≤63 chars
/// and matching the label-value regex.
pub fn is_valid_label_value(value: &str) -> Vec<String> {
    let mut errs = Vec::new();
    if value.len() > LABEL_VALUE_MAX_LENGTH {
        errs.push(format!(
            "must be no more than {LABEL_VALUE_MAX_LENGTH} characters"
        ));
    }
    if !matches_label_value(value) {
        errs.push(format!(
            "a valid label must be an empty string or consist of alphanumeric characters, \
             '-', '_' or '.', and must start and end with an alphanumeric character, got '{value}'"
        ));
    }
    errs
}

// ── Requirement validation ───────────────────────────────────────────────────

/// `(*NodeClaimTemplateSpec).validateRequirements`: validates every
/// requirement, aggregating each failure under the "in requirements, restricted"
/// wrapper upstream uses.
pub fn validate_requirements(reqs: &[NodeSelectorRequirement]) -> Result<(), ValidationError> {
    let mut errs = ValidationError::default();
    for req in reqs {
        if let Err(e) = validate_requirement(req) {
            for msg in e.messages() {
                errs.append(format!("invalid value: {msg} in requirements, restricted"));
            }
        }
    }
    errs.into_result()
}

/// `ValidateRequirement`: validates a single node-selector requirement. The key
/// is normalized first (beta → stable). Errors are aggregated; an empty result
/// means the requirement is admissible.
pub fn validate_requirement(req: &NodeSelectorRequirement) -> Result<(), ValidationError> {
    let mut errs = ValidationError::default();
    let key = normalized_label(&req.key).to_string();

    // Operator must be one of the supported set. With cave's typed `Operator`
    // enum this can only fail if a new variant is added without registering it,
    // but the membership check is preserved for parity with upstream.
    let op = req.operator.to_string();
    if !SUPPORTED_NODE_SELECTOR_OPS.contains(&op.as_str()) {
        errs.append(format!(
            "key {key} has an unsupported operator {op} not in {SUPPORTED_NODE_SELECTOR_OPS:?}"
        ));
    }

    if let Err(e) = is_restricted_label(&key) {
        errs.append(e.to_string());
    }

    if let Some(msg) = validate_well_known_values(&key, req.operator, &req.values, req.min_values) {
        errs.append(msg);
    }

    for msg in is_qualified_name(&key) {
        errs.append(format!("key {key} is not a qualified name, {msg}"));
    }

    for value in &req.values {
        for msg in is_valid_label_value(value) {
            errs.append(format!("invalid value {value} for key {key}, {msg}"));
        }
    }

    if req.operator == Operator::In && req.values.is_empty() {
        errs.append(format!(
            "key {key} with operator {op} must have a value defined"
        ));
    }

    if req.operator == Operator::In {
        if let Some(min) = req.min_values {
            if (req.values.len() as i64) < min {
                errs.append(format!(
                    "key {key} with operator {op} must have at least minimum number of values \
                     defined in 'values' field"
                ));
            }
        }
    }

    if matches!(
        req.operator,
        Operator::Gt | Operator::Lt | Operator::Gte | Operator::Lte
    ) {
        if req.values.len() != 1 {
            errs.append(format!(
                "key {key} with operator {op} must have a single positive integer value"
            ));
        } else {
            match req.values[0].parse::<i64>() {
                Ok(v) if v >= 0 => {}
                _ => errs.append(format!(
                    "key {key} with operator {op} must have a single positive integer value"
                )),
            }
        }
    }

    errs.into_result()
}

/// `validateWellKnownValues`: for an `In` requirement on a well-known key with a
/// gated value set, returns an error only when *no* supplied value is valid, or
/// when the count of valid values falls below `min_values`. Invalid values that
/// coexist with at least one valid value are tolerated (upstream logs them;
/// here they are silently accepted). Returns `None` for keys without a gated
/// value set or non-`In` operators.
fn validate_well_known_values(
    key: &str,
    operator: Operator,
    values: &[String],
    min_values: Option<i64>,
) -> Option<String> {
    if !WELL_KNOWN_LABELS.contains(&key) || operator != Operator::In {
        return None;
    }
    let known = well_known_values_for(key)?;

    let valid: Vec<&String> = values
        .iter()
        .filter(|v| known.contains(&v.as_str()))
        .collect();
    let invalid: Vec<&String> = values
        .iter()
        .filter(|v| !known.contains(&v.as_str()))
        .collect();

    if valid.is_empty() {
        return Some(format!(
            "no valid values found in {values:?} for {key}, expected one of: {known:?}, \
             got: {invalid:?}"
        ));
    }
    if let Some(min) = min_values {
        if (valid.len() as i64) < min {
            return Some(format!(
                "not enough valid values found in {values:?} for {key}, expected at least \
                 {min} valid values from: {known:?}, got: {}",
                valid.len()
            ));
        }
    }
    None
}

// ── Taint validation ─────────────────────────────────────────────────────────

/// `(*NodeClaimTemplateSpec).validateTaints`: validates `taints` and
/// `startupTaints` against a *shared* key/effect set, so a key/effect pair that
/// appears in both fields is rejected as a duplicate.
pub fn validate_taints(taints: &[Taint], startup_taints: &[Taint]) -> Result<(), ValidationError> {
    let mut errs = ValidationError::default();
    let mut existing: Vec<(String, Effect)> = Vec::new();
    validate_taints_field(taints, &mut existing, "taints", &mut errs);
    validate_taints_field(startup_taints, &mut existing, "startupTaints", &mut errs);
    errs.into_result()
}

/// `validateTaintsField`: per-taint key/value qualified-name checks plus
/// duplicate (key, effect) detection against the running `existing` set. The
/// taint effect is validated upstream against the allowed set; cave's typed
/// [`Effect`] enum makes every constructible effect valid, so that branch is a
/// no-op here and is intentionally omitted.
fn validate_taints_field(
    taints: &[Taint],
    existing: &mut Vec<(String, Effect)>,
    field_name: &str,
    errs: &mut ValidationError,
) {
    for taint in taints {
        // Validate Key.
        if taint.key.is_empty() {
            errs.append(format!("invalid value: empty taint key in {field_name}"));
        }
        for msg in is_qualified_name(&taint.key) {
            errs.append(format!("invalid value: {msg} in {field_name}"));
        }
        // Validate Value (only when present and non-empty).
        if let Some(value) = &taint.value {
            if !value.is_empty() {
                for msg in is_qualified_name(value) {
                    errs.append(format!("invalid value: {msg} in {field_name}"));
                }
            }
        }
        // Check for duplicate Key/Effect pairs across both fields.
        let pair = (taint.key.clone(), taint.effect);
        if existing.contains(&pair) {
            errs.append(format!(
                "duplicate taint Key/Effect pair {}={}",
                taint.key, taint.effect
            ));
        }
        existing.push(pair);
    }
}
