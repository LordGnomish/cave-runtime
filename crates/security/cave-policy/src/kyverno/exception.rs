// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno PolicyException matching.
//!
//! Upstream: kyverno/kyverno v1.18.1 —
//!   - api/kyverno/v2/policy_exception_types.go (PolicyException CRD)
//!   - pkg/engine/handlers/exceptions/handler.go (match evaluation)
//!
//! An exception suppresses a (policy, rule) pairing for a resource iff **all**
//! of the following hold:
//!   1. one of `spec.exceptions[]` names the policy and lists the rule,
//!   2. the resource is in scope of the exception's `spec.match` block,
//!   3. the exception's `spec.conditions` (any/all) evaluate true — an absent
//!      `conditions` block is treated as an unconditional match.
//!
//! The engine-side matching is the security-relevant capability; the
//! PolicyException *CRD controller* (status, RBAC-scoped namespace enrollment)
//! remains scope_cut to the Phase-2 controller-runtime port.

use super::models::PolicyException;
use super::validate::eval_conditions;
use serde_json::Value;

/// Does `exc` suppress `(policy_name, rule_name)` for `resource`?
///
/// `context` is the Kyverno evaluation context (`{ "request": { ... } }`) used
/// to resolve `{{ ... }}` JMESPath references inside the exception conditions.
pub fn exception_applies(
    exc: &PolicyException,
    policy_name: &str,
    rule_name: &str,
    resource: &Value,
    namespace: Option<&str>,
    operation: &str,
    context: &Value,
    matches_match_block: impl Fn(&super::models::MatchResources, &Value, Option<&str>, &str) -> bool,
) -> bool {
    // 1. policy + rule must be named in one of the exception entries.
    let names_match = exc.spec.exceptions.iter().any(|entry| {
        entry.policy_name == policy_name && entry.rule_names.iter().any(|r| r == rule_name)
    });
    if !names_match {
        return false;
    }

    // 2. resource must be in scope of the exception's match block.
    if !matches_match_block(&exc.spec.match_resources, resource, namespace, operation) {
        return false;
    }

    // 3. conditions (if any) must evaluate true.
    if let Some(conditions) = &exc.spec.conditions {
        match eval_conditions(conditions, resource, context) {
            Ok(true) => {}
            // A condition that is false — or that errors while resolving — means
            // the exception does NOT apply (fail-closed: the policy still runs).
            _ => return false,
        }
    }

    true
}
