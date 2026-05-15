// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::types::BucketPolicy;

pub struct PolicyEvaluator;

pub enum PolicyEffect {
    Allow,
    Deny,
    NoMatch,
}

impl PolicyEvaluator {
    pub fn evaluate(
        policy: &BucketPolicy,
        action: &str,
        resource: &str,
        principal: &str,
    ) -> PolicyEffect {
        let mut matched_allow = false;

        for stmt in &policy.statements {
            let principal_match = stmt.principal.iter().any(|p| p == "*" || p == principal);
            if !principal_match {
                continue;
            }

            let action_match = stmt.action.iter().any(|a| {
                if a.ends_with('*') {
                    action.starts_with(&a[..a.len() - 1])
                } else {
                    a == action
                }
            });
            if !action_match {
                continue;
            }

            let resource_match = stmt.resource.iter().any(|r| {
                if r.ends_with('*') {
                    resource.starts_with(&r[..r.len() - 1])
                } else {
                    r == resource || r == "*"
                }
            });
            if !resource_match {
                continue;
            }

            // Matching statement
            if stmt.effect == "Deny" {
                return PolicyEffect::Deny;
            }
            if stmt.effect == "Allow" {
                matched_allow = true;
            }
        }

        if matched_allow {
            PolicyEffect::Allow
        } else {
            PolicyEffect::NoMatch
        }
    }
}
