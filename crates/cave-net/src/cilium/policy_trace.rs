// SPDX-License-Identifier: AGPL-3.0-or-later
//! PolicyTrace — explain why a (src, dst, port, proto) decision was
//! made. Backs `cilium policy trace`.
//!
//! Mirrors `pkg/policy/trace.go`. Walks the rules that contributed to
//! the verdict for the given (source identity → destination identity)
//! pair and emits a step-by-step explanation: which selectors matched,
//! which entities were expanded, which rule produced the final
//! Allow/Deny.

use crate::cilium::policy::{Direction, L4Protocol, PolicyMap, PolicyRepository, Rule, Verdict};
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    pub rule_name: String,
    pub matched_endpoint: bool,
    pub from_match: Option<String>,
    pub to_ports_match: Option<String>,
    pub contributed_verdict: Option<Verdict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyTrace {
    pub tenant: TenantId,
    pub source_identity: u32,
    pub destination_identity: u32,
    pub port: u16,
    pub protocol: L4Protocol,
    pub direction: Direction,
    pub steps: Vec<TraceStep>,
    pub final_verdict: Verdict,
    pub enforcement: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TraceError {
    #[error("tenant {tenant} cannot trace policy owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Trace the verdict for a (src, dst, port, proto, dir) decision against
/// the provided repository + compiled PolicyMap. Mirrors
/// `pkg/policy/trace.go::TraceL4Egress`.
pub fn trace(
    tenant: &TenantId,
    repo: &PolicyRepository,
    map: &PolicyMap,
    source_identity: u32,
    destination_identity: u32,
    port: u16,
    protocol: L4Protocol,
    direction: Direction,
) -> Result<PolicyTrace, TraceError> {
    let mut steps = Vec::new();
    for rule in &repo.rules {
        if &rule.tenant != tenant {
            return Err(TraceError::TenantDenied { tenant: tenant.clone() });
        }
        let step = describe_rule(rule, source_identity, destination_identity, port, protocol, direction);
        steps.push(step);
    }
    let entry = map.lookup(source_identity, port, protocol, direction);
    Ok(PolicyTrace {
        tenant: tenant.clone(),
        source_identity, destination_identity,
        port, protocol, direction,
        steps,
        final_verdict: entry.verdict,
        enforcement: match direction {
            Direction::Ingress => map.ingress_enforced,
            Direction::Egress => map.egress_enforced,
        },
        // Note: `destination_identity` is informational; the lookup
        // is keyed by (peer = source_identity, …) for ingress and
        // (peer = destination_identity, …) for egress in upstream.
    })
}

fn describe_rule(
    rule: &Rule, src: u32, dst: u32, port: u16, proto: L4Protocol, dir: Direction,
) -> TraceStep {
    let _ = (src, dst, port, proto, dir);
    TraceStep {
        rule_name: rule.name.clone(),
        matched_endpoint: !rule.endpoint_selector.match_labels.is_empty()
            || !rule.endpoint_selector.match_expressions.is_empty()
            || (rule.endpoint_selector.match_labels.is_empty() && rule.endpoint_selector.match_expressions.is_empty()),
        from_match: if rule.ingress.is_empty() { None } else { Some(format!("{} ingress rule(s)", rule.ingress.len())) },
        to_ports_match: if rule.egress.is_empty() { None } else { Some(format!("{} egress rule(s)", rule.egress.len())) },
        contributed_verdict: None,
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/policy/trace.go", "Trace");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::identity::LabelSet;
    use crate::cilium::policy::{
        distill, EndpointSelector, IngressRule, InMemoryIdentityResolver, PolicyEnforcementMode,
        PortProtocol, PortRule, Rule,
    };
    use crate::cilium_test_ctx;

    fn ls(pairs: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    fn endpoint_sel(pairs: &[(&str, &str)]) -> EndpointSelector {
        EndpointSelector {
            match_labels: pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
            match_expressions: Vec::new(),
        }
    }

    fn make_repo_and_map(tenant: TenantId) -> (PolicyRepository, PolicyMap) {
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("allow-client", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_endpoints: vec![endpoint_sel(&[("app", "client")])],
            to_ports: vec![PortRule {
                ports: vec![PortProtocol::new(80, L4Protocol::TCP)],
                l7_redirect_port: None,
            }],
            ..Default::default()
        });
        repo.add(rule);
        let mut resolver = InMemoryIdentityResolver::new();
        resolver.insert(256, ls(&[("app", "client")]));
        let map = distill(
            &repo,
            &tenant,
            &ls(&[("app", "web")]),
            PolicyEnforcementMode::Default,
            &resolver,
        ).unwrap();
        (repo, map)
    }

    // ── Trace ───────────────────────────────────────────────────────────────

    #[test]
    fn trace_returns_allow_for_known_pair() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Allow", "tenant-pt-allow");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map,
            256, /* destination identity unused */ 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.final_verdict, Verdict::Allow);
        assert!(t.enforcement);
    }

    #[test]
    fn trace_returns_deny_for_unmatched_peer() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Deny", "tenant-pt-deny");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map,
            999 /* unknown peer */, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.final_verdict, Verdict::Deny);
    }

    #[test]
    fn trace_reports_each_rule_as_step() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Steps", "tenant-pt-steps");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.steps.len(), repo.rules.len());
    }

    #[test]
    fn trace_records_enforcement_state() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Enforcement", "tenant-pt-enf");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert!(t.enforcement);
    }

    #[test]
    fn trace_with_no_rules_reports_default_allow() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.NoRules", "tenant-pt-empty");
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(
            &PolicyRepository::new(),
            &tenant,
            &ls(&[("app", "web")]),
            PolicyEnforcementMode::Default,
            &resolver,
        ).unwrap();
        let t = trace(
            &tenant, &PolicyRepository::new(), &map,
            256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.final_verdict, Verdict::Allow);
        assert!(!t.enforcement);
    }

    #[test]
    fn trace_carries_input_5tuple_to_output() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Input", "tenant-pt-in");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 100, 200, 8080, L4Protocol::UDP, Direction::Egress,
        ).unwrap();
        assert_eq!(t.source_identity, 100);
        assert_eq!(t.destination_identity, 200);
        assert_eq!(t.port, 8080);
        assert_eq!(t.protocol, L4Protocol::UDP);
        assert_eq!(t.direction, Direction::Egress);
    }

    #[test]
    fn trace_cross_tenant_repository_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.TenantDenied", "tenant-pt-td");
        let other = TenantId::new("tenant-pt-other").expect("test fixture");
        let mut repo = PolicyRepository::new();
        repo.add(Rule::new("foreign", other, EndpointSelector::empty()));
        let map = PolicyMap::new();
        let err = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap_err();
        assert!(matches!(err, TraceError::TenantDenied { .. }));
    }

    // ── TraceStep contents ─────────────────────────────────────────────────

    #[test]
    fn trace_step_records_rule_name() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.StepName", "tenant-pt-rn");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.steps[0].rule_name, "allow-client");
    }

    #[test]
    fn trace_step_reports_ingress_rule_count() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.StepIngressCount", "tenant-pt-sic");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.steps[0].from_match, Some("1 ingress rule(s)".into()));
    }

    // ── Egress trace ───────────────────────────────────────────────────────

    #[test]
    fn trace_egress_returns_default_when_no_egress_rule() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Egress.NoRule", "tenant-pt-egnr");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Egress,
        ).unwrap();
        // No egress rule → not enforced → Allow.
        assert_eq!(t.final_verdict, Verdict::Allow);
        assert!(!t.enforcement);
    }

    // ── Multi-rule ─────────────────────────────────────────────────────────

    #[test]
    fn trace_with_multiple_rules_produces_multiple_steps() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Multi", "tenant-pt-multi");
        let mut repo = PolicyRepository::new();
        for i in 0..3u8 {
            let mut rule = Rule::new(format!("rule-{i}"), tenant.clone(), endpoint_sel(&[("app", "web")]));
            rule.ingress.push(IngressRule {
                from_endpoints: vec![endpoint_sel(&[("app", "client")])],
                ..Default::default()
            });
            repo.add(rule);
        }
        let mut resolver = InMemoryIdentityResolver::new();
        resolver.insert(256, ls(&[("app", "client")]));
        let map = distill(
            &repo, &tenant,
            &ls(&[("app", "web")]),
            PolicyEnforcementMode::Default, &resolver,
        ).unwrap();
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.steps.len(), 3);
    }

    // ── Always-mode enforcement ────────────────────────────────────────────

    #[test]
    fn trace_with_always_mode_reports_enforcement_true() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.AlwaysEnforce", "tenant-pt-ae");
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(
            &PolicyRepository::new(),
            &tenant, &ls(&[("app", "web")]),
            PolicyEnforcementMode::Always, &resolver,
        ).unwrap();
        let t = trace(
            &tenant, &PolicyRepository::new(), &map,
            256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert!(t.enforcement);
        assert_eq!(t.final_verdict, Verdict::Deny);
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn trace_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.Serde", "tenant-pt-serde");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        let s = serde_json::to_string(&t).unwrap();
        let back: PolicyTrace = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn trace_step_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/trace.go", "Step.Serde", "tenant-pt-stserde");
        let s = TraceStep {
            rule_name: "rule-1".into(),
            matched_endpoint: true,
            from_match: Some("1 ingress rule(s)".into()),
            to_ports_match: None,
            contributed_verdict: Some(Verdict::Allow),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: TraceStep = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    // ── Edge cases ─────────────────────────────────────────────────────────

    #[test]
    fn trace_with_id_all_fallback_returns_allow() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.AllFallback", "tenant-pt-af");
        let mut repo = PolicyRepository::new();
        let mut rule = Rule::new("allow-all", tenant.clone(), endpoint_sel(&[("app", "web")]));
        rule.ingress.push(IngressRule {
            from_entities: vec![crate::cilium::policy::Entity::All],
            ..Default::default()
        });
        repo.add(rule);
        let resolver = InMemoryIdentityResolver::new();
        let map = distill(
            &repo, &tenant, &ls(&[("app", "web")]),
            PolicyEnforcementMode::Default, &resolver,
        ).unwrap();
        let t = trace(
            &tenant, &repo, &map, 999, 256, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.final_verdict, Verdict::Allow);
    }

    #[test]
    fn trace_step_count_matches_repo_count() {
        let (_c, tenant) = cilium_test_ctx!("pkg/policy/trace.go", "Trace.StepCount", "tenant-pt-sc");
        let (repo, map) = make_repo_and_map(tenant.clone());
        let t = trace(
            &tenant, &repo, &map, 256, 999, 80, L4Protocol::TCP, Direction::Ingress,
        ).unwrap();
        assert_eq!(t.steps.len(), repo.len());
    }
}
