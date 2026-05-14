// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Routing rule evaluation. Given a request path, decide whether one of the
//! configured rules permits or denies it. `Allow` rules act as an
//! allowlist (matched paths pass, others fail); `Block` rules act as a
//! denylist (matched paths fail, others pass).
//!
//! When evaluating multiple rules, the order is: any `Block` match denies,
//! else any `Allow` match permits, else if no `Allow` rules exist the path
//! is permitted by default. This mirrors Nexus' precedence model.

use super::error::NexusError;
use super::models::{RoutingDecision, RoutingMode, RoutingRule};
use regex::Regex;

/// Test a single rule against `path`.
pub fn evaluate(rule: &RoutingRule, path: &str) -> Result<RoutingDecision, NexusError> {
    let any_match = rule
        .matchers
        .iter()
        .map(|m| Regex::new(m).map_err(|e| NexusError::InvalidRegex(e.to_string())))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|re| re.is_match(path));

    Ok(match (rule.mode, any_match) {
        (RoutingMode::Allow, true) => RoutingDecision::Allowed,
        (RoutingMode::Allow, false) => RoutingDecision::Blocked,
        (RoutingMode::Block, true) => RoutingDecision::Blocked,
        (RoutingMode::Block, false) => RoutingDecision::Allowed,
    })
}

/// Apply the full ruleset to `path`. See module-level docs for precedence.
pub fn evaluate_all(rules: &[RoutingRule], path: &str) -> Result<RoutingDecision, NexusError> {
    let mut has_allow = false;
    let mut allow_match = false;
    for r in rules {
        match r.mode {
            RoutingMode::Block => {
                if matches!(evaluate(r, path)?, RoutingDecision::Blocked) {
                    return Ok(RoutingDecision::Blocked);
                }
            }
            RoutingMode::Allow => {
                has_allow = true;
                if matches!(evaluate(r, path)?, RoutingDecision::Allowed) {
                    allow_match = true;
                }
            }
        }
    }
    Ok(if !has_allow || allow_match {
        RoutingDecision::Allowed
    } else {
        RoutingDecision::Blocked
    })
}
