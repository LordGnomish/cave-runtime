//! webhook_admission_v2 tests — TLS dial FSM, service resolver, CABundle
//! chain, deadline, matchConditions matcher, reinvocation tracker.

use super::*;
use crate::admission::{AdmissionRequest, Operation};
use crate::resources::{ConfigMap, ObjectMeta, Resource};
use crate::vap_advanced::{CelValue, CelError, FixedEvaluator};
use crate::webhook_admission::{FailurePolicyType, ServiceReference, WebhookClientConfig};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn req(tenant: &str) -> AdmissionRequest {
    AdmissionRequest {
        uid: "u".into(), tenant_id: tenant.into(),
        namespace: "ns".into(), kind: "ConfigMap".into(), name: "cm".into(),
        operation: Operation::Create,
        object: Some(Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new("cm", "ns"), data: HashMap::new(),
        })),
        old_object: None, user: "alice".into(), dry_run: false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DialFsm
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dial_fsm_happy_path() {
    let mut f = DialFsm::new();
    f.step(DialEvent::Resolve).unwrap();
    f.step(DialEvent::Connect).unwrap();
    f.step(DialEvent::Handshake).unwrap();
    f.step(DialEvent::Complete).unwrap();
    assert_eq!(f.state, DialState::Established);
    assert!(f.is_terminal());
}

#[test]
fn dial_fsm_rejects_skip_steps() {
    let mut f = DialFsm::new();
    assert!(f.step(DialEvent::Connect).is_err(),
        "cannot Connect before Resolve");
}

#[test]
fn dial_fsm_fail_from_any_state() {
    let mut f = DialFsm::new();
    f.step(DialEvent::Resolve).unwrap();
    f.step(DialEvent::Fail("rst".into())).unwrap();
    assert_eq!(f.state, DialState::Failed);
}

#[test]
fn dial_fsm_records_transitions() {
    let mut f = DialFsm::new();
    f.step(DialEvent::Resolve).unwrap();
    f.step(DialEvent::Connect).unwrap();
    assert_eq!(f.transitions.len(), 2);
    assert_eq!(f.transitions[0].2, DialState::Resolved);
    assert_eq!(f.transitions[1].2, DialState::Connected);
}

#[test]
fn dial_fsm_terminal_states() {
    let mut f = DialFsm::new();
    assert!(!f.is_terminal());
    f.step(DialEvent::Fail("x".into())).unwrap();
    assert!(f.is_terminal());
}

// ─────────────────────────────────────────────────────────────────────────────
// ServiceResolver / resolve_endpoint
// ─────────────────────────────────────────────────────────────────────────────

fn cluster_resolver() -> ClusterServiceResolver {
    ClusterServiceResolver { cluster_domain: "cluster.local".into() }
}

#[test]
fn resolver_default_port_443() {
    let r = cluster_resolver();
    let url = r.resolve(&ServiceReference {
        namespace: "default".into(), name: "svc".into(),
        path: None, port: None,
    }).unwrap();
    assert_eq!(url, "https://svc.default.svc.cluster.local:443");
}

#[test]
fn resolver_explicit_port() {
    let r = cluster_resolver();
    let url = r.resolve(&ServiceReference {
        namespace: "default".into(), name: "svc".into(),
        path: None, port: Some(8443),
    }).unwrap();
    assert!(url.ends_with(":8443"));
}

#[test]
fn resolver_path_prepended_with_slash() {
    let r = cluster_resolver();
    let url = r.resolve(&ServiceReference {
        namespace: "default".into(), name: "svc".into(),
        path: Some("hook".into()), port: None,
    }).unwrap();
    assert!(url.ends_with("/hook"));
}

#[test]
fn resolver_path_already_slashed() {
    let r = cluster_resolver();
    let url = r.resolve(&ServiceReference {
        namespace: "default".into(), name: "svc".into(),
        path: Some("/hook".into()), port: None,
    }).unwrap();
    assert!(url.ends_with("/hook"));
    assert!(!url.ends_with("//hook"));
}

#[test]
fn resolver_rejects_empty_name() {
    let r = cluster_resolver();
    assert!(r.resolve(&ServiceReference {
        namespace: "default".into(), name: "".into(),
        path: None, port: None,
    }).is_err());
}

#[test]
fn resolver_rejects_empty_namespace() {
    let r = cluster_resolver();
    assert!(r.resolve(&ServiceReference {
        namespace: "".into(), name: "svc".into(),
        path: None, port: None,
    }).is_err());
}

#[test]
fn resolver_rejects_invalid_port() {
    let r = cluster_resolver();
    assert!(r.resolve(&ServiceReference {
        namespace: "default".into(), name: "svc".into(),
        path: None, port: Some(0),
    }).is_err());
    assert!(r.resolve(&ServiceReference {
        namespace: "default".into(), name: "svc".into(),
        path: None, port: Some(65536),
    }).is_err());
}

#[test]
fn resolve_endpoint_url_passthrough() {
    let cfg = WebhookClientConfig {
        url: Some("https://x.example/h".into()), service: None, ca_bundle: vec![],
    };
    let r = cluster_resolver();
    assert_eq!(resolve_endpoint(&r, &cfg).unwrap(), "https://x.example/h");
}

#[test]
fn resolve_endpoint_service_uses_resolver() {
    let cfg = WebhookClientConfig {
        url: None,
        service: Some(ServiceReference {
            namespace: "default".into(), name: "svc".into(),
            path: None, port: Some(443),
        }),
        ca_bundle: vec![],
    };
    let r = cluster_resolver();
    assert!(resolve_endpoint(&r, &cfg).unwrap().contains("svc.default.svc.cluster.local"));
}

#[test]
fn resolve_endpoint_neither_url_nor_service_errors() {
    let cfg = WebhookClientConfig::default();
    let r = cluster_resolver();
    assert!(resolve_endpoint(&r, &cfg).is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// verify_chain
// ─────────────────────────────────────────────────────────────────────────────

fn cn(s: &str, i: &str, root: bool) -> CertNode {
    CertNode { subject: s.into(), issuer: i.into(), is_root: root }
}

#[test]
fn chain_leaf_directly_trusted() {
    let certs = vec![cn("leaf", "leaf", true)];
    let out = verify_chain("leaf", &certs, &["leaf".to_string()]);
    assert_eq!(out, ChainOutcome::Trusted { depth: 0 });
}

#[test]
fn chain_leaf_intermediate_root() {
    let certs = vec![
        cn("leaf", "intermediate", false),
        cn("intermediate", "root", false),
        cn("root", "root", true),
    ];
    let out = verify_chain("leaf", &certs, &["root".to_string()]);
    matches!(out, ChainOutcome::Trusted { .. });
}

#[test]
fn chain_missing_intermediate_untrusted() {
    let certs = vec![cn("leaf", "missing", false)];
    let out = verify_chain("leaf", &certs, &["root".to_string()]);
    assert!(matches!(out, ChainOutcome::Untrusted(_)));
}

#[test]
fn chain_untrusted_root() {
    let certs = vec![
        cn("leaf", "evil", false),
        cn("evil", "evil", true),
    ];
    let out = verify_chain("leaf", &certs, &["good-root".to_string()]);
    assert!(matches!(out, ChainOutcome::Untrusted(_)));
}

#[test]
fn chain_cycle_detected() {
    let certs = vec![
        cn("a", "b", false),
        cn("b", "a", false),
    ];
    let out = verify_chain("a", &certs, &["root".to_string()]);
    assert_eq!(out, ChainOutcome::Cycle);
}

#[test]
fn chain_too_long_untrusted() {
    let mut certs = vec![];
    for i in 0..20 {
        certs.push(CertNode {
            subject: format!("cn{i}"), issuer: format!("cn{}", i + 1), is_root: false,
        });
    }
    certs.push(cn("cn20", "cn20", true));
    let out = verify_chain("cn0", &certs, &["root".to_string()]);
    assert!(matches!(out, ChainOutcome::Untrusted(_)));
}

// ─────────────────────────────────────────────────────────────────────────────
// Deadline
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn deadline_remaining_in_future() {
    let d = Deadline::from_seconds(10);
    assert!(d.remaining().is_some());
    assert!(!d.expired());
}

#[test]
fn deadline_expired_after_sleep() {
    let d = Deadline::from_millis(10);
    thread::sleep(Duration::from_millis(20));
    assert!(d.expired());
    assert!(d.remaining().is_none());
}

#[test]
fn deadline_minimum_one_second() {
    let d = Deadline::from_seconds(0);
    let r = d.remaining().unwrap();
    assert!(r >= Duration::from_millis(900));
    assert!(r <= Duration::from_secs(1));
}

#[test]
fn deadline_clamps_negative_seconds_to_one() {
    let d = Deadline::from_seconds(-5);
    assert!(d.remaining().is_some());
}

// ─────────────────────────────────────────────────────────────────────────────
// MatchConditionsMatcher
// ─────────────────────────────────────────────────────────────────────────────

fn matcher(ev: FixedEvaluator) -> MatchConditionsMatcher {
    MatchConditionsMatcher { evaluator: Arc::new(ev) }
}

#[test]
fn matcher_no_conditions_matched() {
    let m = matcher(FixedEvaluator::new());
    let r = m.match_(&[], &FailurePolicyType::Fail, &req("acme"));
    assert_eq!(r, MatchOutcome::Matched);
}

#[test]
fn matcher_all_true_matched() {
    let ev = FixedEvaluator::new()
        .with("a", CelValue::Bool(true))
        .with("b", CelValue::Bool(true));
    let m = matcher(ev);
    let conds = vec![
        MatchCondition { name: "a".into(), expression: "a".into() },
        MatchCondition { name: "b".into(), expression: "b".into() },
    ];
    let r = m.match_(&conds, &FailurePolicyType::Fail, &req("acme"));
    assert_eq!(r, MatchOutcome::Matched);
}

#[test]
fn matcher_any_false_no_match() {
    let ev = FixedEvaluator::new()
        .with("a", CelValue::Bool(true))
        .with("b", CelValue::Bool(false));
    let conds = vec![
        MatchCondition { name: "a".into(), expression: "a".into() },
        MatchCondition { name: "b".into(), expression: "b".into() },
    ];
    let m = matcher(ev);
    let r = m.match_(&conds, &FailurePolicyType::Fail, &req("acme"));
    assert_eq!(r, MatchOutcome::NoMatch);
}

#[test]
fn matcher_non_bool_fails() {
    let ev = FixedEvaluator::new().with("a", CelValue::Int(7));
    let conds = vec![MatchCondition { name: "a".into(), expression: "a".into() }];
    let m = matcher(ev);
    let r = m.match_(&conds, &FailurePolicyType::Fail, &req("acme"));
    matches!(r, MatchOutcome::Fail { .. });
}

#[test]
fn matcher_error_with_fail_policy_fails() {
    let ev = FixedEvaluator::new().with_err("a", CelError::Compile("x".into()));
    let conds = vec![MatchCondition { name: "a".into(), expression: "a".into() }];
    let m = matcher(ev);
    let r = m.match_(&conds, &FailurePolicyType::Fail, &req("acme"));
    matches!(r, MatchOutcome::Fail { .. });
}

#[test]
fn matcher_error_with_ignore_policy_silenced() {
    let ev = FixedEvaluator::new().with_err("a", CelError::Compile("x".into()));
    let conds = vec![MatchCondition { name: "a".into(), expression: "a".into() }];
    let m = matcher(ev);
    let r = m.match_(&conds, &FailurePolicyType::Ignore, &req("acme"));
    matches!(r, MatchOutcome::SilencedError { .. });
}

#[test]
fn matcher_short_circuits_on_first_false() {
    let ev = FixedEvaluator::new()
        .with("a", CelValue::Bool(false));
    // 'b' missing — would cause Compile error if reached.
    let conds = vec![
        MatchCondition { name: "a".into(), expression: "a".into() },
        MatchCondition { name: "b".into(), expression: "b".into() },
    ];
    let m = matcher(ev);
    let r = m.match_(&conds, &FailurePolicyType::Fail, &req("acme"));
    assert_eq!(r, MatchOutcome::NoMatch,
        "matcher must short-circuit on first false (without evaluating later ones)");
}

// ─────────────────────────────────────────────────────────────────────────────
// ReinvocationContext
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn reinvoke_first_invocation_is_not_reinvocation() {
    let r = ReinvocationContext::new();
    assert!(!r.mark_invoked("h1"));
}

#[test]
fn reinvoke_second_invocation_flagged() {
    let r = ReinvocationContext::new();
    r.mark_invoked("h1");
    assert!(r.mark_invoked("h1"));
}

#[test]
fn reinvoke_should_reinvoke_after_modify() {
    let r = ReinvocationContext::new();
    r.mark_invoked("h1");
    r.mark_invoked("h2");
    r.mark_modified_after("h2"); // h2 modified the object
    assert!(r.should_reinvoke("h1"),
        "h1 ran before h2; h2 modified → h1 must be reinvoked");
}

#[test]
fn reinvoke_does_not_flag_self_or_later_for_modify_event() {
    let r = ReinvocationContext::new();
    r.mark_invoked("h1");
    r.mark_invoked("h2");
    r.mark_invoked("h3");
    r.mark_modified_after("h2");
    assert!(!r.should_reinvoke("h2"));
    assert!(!r.should_reinvoke("h3"));
}

#[test]
fn reinvoke_invoked_count_grows() {
    let r = ReinvocationContext::new();
    r.mark_invoked("h1");
    r.mark_invoked("h2");
    r.mark_invoked("h1"); // dup
    assert_eq!(r.invoked_count(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real rustls / async runtime
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn dial_fsm_drives_real_tls_connection() {
    todo!("requires rustls + tokio TcpStream — full handshake against fixture cert");
}

#[test] #[ignore]
fn deadline_aborts_in_flight_request() {
    todo!("requires async cancel — drop/abort the future at deadline");
}

#[test] #[ignore]
fn cabundle_real_x509_chain_validation() {
    todo!("requires rustls/webpki — sigalg + key-usage + EKU checks");
}

#[test] #[ignore]
fn matcher_uses_real_cel_evaluator() {
    todo!("requires CEL — `request.userInfo.username == 'alice'`");
}

#[test] #[ignore]
fn reinvoke_tracks_object_diff_via_json_patch() {
    todo!("requires diff between successive object views to mark modified_after");
}
