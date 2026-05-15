// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! audit.k8s.io/v1 Policy + PolicyRule — line-by-line port of upstream
//! `staging/src/k8s.io/apiserver/pkg/apis/audit/v1/types.go` and
//! `staging/src/k8s.io/apiserver/pkg/audit/policy/checker.go`.
//!
//! The existing `audit.rs` already covers Stage / Level / Event with a
//! lightweight default-level policy. This module layers KEP-1601-shaped
//! `Policy` with a list of `PolicyRule`s, each selecting on user, verb,
//! resource, namespace, nonResourceURL — first-match wins.
//!
//! ## Tenant invariant
//!
//! Tenant_id is part of the event identity, NOT a selection axis. Rules MUST
//! NOT match (or skip) on tenant_id, and the chosen Level applies regardless
//! of which tenant the request belongs to.

use crate::audit::{AuditEvent, AuditLevel, AuditStage};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupResources {
    /// API group; empty string == core.
    #[serde(default)]
    pub group: String,
    /// Empty list == "all resources in this group" (`*`).
    #[serde(default)]
    pub resources: Vec<String>,
    /// `<resource>/<subresource>` patterns when subresource matters.
    #[serde(default)]
    pub resource_names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRule {
    pub level: AuditLevel,
    /// Users selected by name; empty == any.
    #[serde(default)]
    pub users: Vec<String>,
    #[serde(default)]
    pub user_groups: Vec<String>,
    /// Verbs (`get`, `list`, `watch`, `create`, `update`, `patch`, `delete`,
    /// `deletecollection`); empty == any.
    #[serde(default)]
    pub verbs: Vec<String>,
    #[serde(default)]
    pub resources: Vec<GroupResources>,
    /// Namespaces; empty == any (cluster-scoped + namespaced).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Non-resource URLs (`/healthz`, `/livez`, `/version`, `/api/*`).
    #[serde(default)]
    pub non_resource_urls: Vec<String>,
    /// Per-rule omitStages — empty inherits Policy.omit_stages.
    #[serde(default)]
    pub omit_stages: Vec<AuditStage>,
    /// KEP-2155: drops `metadata.managedFields` from the audited object.
    #[serde(default)]
    pub omit_managed_fields: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PolicyV1 {
    /// Default level when no rule matches. Upstream defaults to `None` (drop)
    /// when no rule is matched, but operators commonly set Metadata.
    pub level: AuditLevel,
    pub omit_stages: Vec<AuditStage>,
    pub rules: Vec<PolicyRule>,
}

impl PolicyV1 {
    /// Pick the first rule matching the request descriptor. Returns the rule's
    /// level and effective omit_stages (rule's, falling back to policy's).
    pub fn evaluate(&self, req: &PolicyEvaluationInput) -> PolicyDecision {
        for rule in &self.rules {
            if rule_matches(rule, req) {
                return PolicyDecision {
                    level: rule.level,
                    omit_stages: if rule.omit_stages.is_empty() {
                        self.omit_stages.clone()
                    } else {
                        rule.omit_stages.clone()
                    },
                    omit_managed_fields: rule.omit_managed_fields,
                };
            }
        }
        PolicyDecision {
            level: self.level,
            omit_stages: self.omit_stages.clone(),
            omit_managed_fields: false,
        }
    }
}

/// Inputs to one policy evaluation. Mirrors `audit.Event` minus the body.
#[derive(Debug, Clone)]
pub struct PolicyEvaluationInput<'a> {
    pub user: &'a str,
    pub user_groups: &'a [String],
    pub verb: &'a str,
    pub group: &'a str,
    pub resource: &'a str,
    pub subresource: &'a str,
    pub namespace: &'a str,
    pub name: &'a str,
    /// `Some(path)` for non-resource URLs (e.g. `/healthz`); `None` for resource
    /// requests.
    pub non_resource_url: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    pub level: AuditLevel,
    pub omit_stages: Vec<AuditStage>,
    pub omit_managed_fields: bool,
}

fn any_or_contains(list: &[String], val: &str) -> bool {
    list.is_empty() || list.iter().any(|x| x == val)
}

fn group_resources_match(res: &[GroupResources], group: &str, resource: &str, subresource: &str) -> bool {
    if res.is_empty() { return true; }
    for r in res {
        if r.group != group { continue; }
        // resources empty == any
        if !r.resources.is_empty() && !r.resources.iter().any(|x| x == resource || x == "*") {
            continue;
        }
        // resource_names empty == any subresource
        if !r.resource_names.is_empty() {
            let qualified = if subresource.is_empty() {
                resource.to_string()
            } else {
                format!("{resource}/{subresource}")
            };
            if !r.resource_names.iter().any(|n| n == &qualified) {
                continue;
            }
        }
        return true;
    }
    false
}

pub fn rule_matches(rule: &PolicyRule, input: &PolicyEvaluationInput) -> bool {
    // Non-resource URL rules: only match non-resource requests; URL list selects.
    let is_nrurl = input.non_resource_url.is_some();
    let rule_is_nrurl = !rule.non_resource_urls.is_empty();
    if is_nrurl != rule_is_nrurl {
        // a non-resource URL request must hit a rule with non_resource_urls,
        // and a resource request must not.
        return false;
    }
    if rule_is_nrurl {
        let url = input.non_resource_url.unwrap();
        if !rule.non_resource_urls.iter().any(|p| nrurl_matches(p, url)) {
            return false;
        }
    } else {
        if !group_resources_match(&rule.resources, input.group, input.resource, input.subresource) {
            return false;
        }
        if !rule.namespaces.is_empty()
            && !rule.namespaces.iter().any(|n| n == input.namespace)
        {
            return false;
        }
    }
    if !any_or_contains(&rule.users, input.user) { return false; }
    if !rule.user_groups.is_empty() {
        let want: HashSet<&String> = rule.user_groups.iter().collect();
        let have: HashSet<&String> = input.user_groups.iter().collect();
        if want.is_disjoint(&have) { return false; }
    }
    if !any_or_contains(&rule.verbs, input.verb) { return false; }
    true
}

/// `*` is a single-level wildcard (matches any path segment but not `/`).
fn nrurl_matches(pattern: &str, url: &str) -> bool {
    if pattern == url { return true; }
    if let Some(prefix) = pattern.strip_suffix("/*") {
        if url.starts_with(prefix) {
            let rest = &url[prefix.len()..];
            return rest.starts_with('/') && !rest[1..].contains('/');
        }
    }
    false
}

// ─────────────────────────────────────────────────────────────────────────────
// Stage progression — for a single request the same audit_id walks through
// {RequestReceived → ResponseStarted → ResponseComplete} (or Panic). The
// emitter MUST produce the same audit_id across all stages, and the policy
// decision MUST be evaluated once at RequestReceived and reused. Mirrors
// upstream `audit/event.go::Event.AuditID` flow.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StageEmitter {
    pub audit_id: String,
    pub decision: PolicyDecision,
    pub tenant_id: String,
    /// Stages we've emitted so far — guards against duplicates.
    emitted: HashSet<u8>,
}

impl StageEmitter {
    pub fn new(audit_id: String, decision: PolicyDecision, tenant_id: String) -> Self {
        Self { audit_id, decision, tenant_id, emitted: HashSet::new() }
    }

    fn stage_id(s: AuditStage) -> u8 {
        match s {
            AuditStage::RequestReceived => 0,
            AuditStage::ResponseStarted => 1,
            AuditStage::ResponseComplete => 2,
            AuditStage::Panic => 3,
        }
    }

    /// Build an event for this stage, applying level redaction + omit_stages.
    /// Returns `None` if the stage is omitted by policy or already emitted.
    pub fn build(
        &mut self,
        stage: AuditStage,
        user: &str,
        verb: &str,
        resource: &str,
        name: &str,
        namespace: &str,
        request_uri: &str,
        response_code: u16,
        request_object: Option<serde_json::Value>,
        response_object: Option<serde_json::Value>,
    ) -> Option<AuditEvent> {
        // Level=None drops everything.
        if self.decision.level == AuditLevel::None { return None; }
        // Drop omit_stages.
        if self.decision.omit_stages.contains(&stage) { return None; }
        // Suppress duplicates.
        let id = Self::stage_id(stage);
        if !self.emitted.insert(id) { return None; }

        let mut ev = AuditEvent::new(
            self.audit_id.clone(),
            self.decision.level,
            stage,
            user, self.tenant_id.clone(), namespace,
            verb, resource, name, request_uri, response_code,
        );
        ev.request_object = request_object;
        ev.response_object = response_object;
        ev.redact_for_level();
        if self.decision.omit_managed_fields {
            redact_managed_fields(&mut ev.request_object);
            redact_managed_fields(&mut ev.response_object);
        }
        Some(ev)
    }
}

/// Strip `metadata.managedFields` from the JSON object payload, if present.
/// Mirrors upstream `audit.OmitManagedFields()`.
pub fn redact_managed_fields(o: &mut Option<serde_json::Value>) {
    if let Some(serde_json::Value::Object(m)) = o {
        if let Some(serde_json::Value::Object(meta)) = m.get_mut("metadata") {
            meta.remove("managedFields");
        }
    }
}

#[cfg(test)]
mod tests;
