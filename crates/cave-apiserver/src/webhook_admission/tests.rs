//! Webhook admission tests — every test names its upstream Go target so
//! reverse lookup is one `grep` away.

use super::*;
use crate::admission::{AdmissionChain, AdmissionRequest, Operation};
use crate::resources::{ConfigMap, ObjectMeta, Resource};
use std::collections::HashMap;
use std::sync::Arc;

fn req(op: Operation, ns: &str, tenant: &str) -> AdmissionRequest {
    let mut meta = ObjectMeta::new("cm1", ns);
    meta.labels.insert("env".into(), "prod".into());
    let cm = Resource::ConfigMap(ConfigMap {
        api_version: "v1".into(), kind: "ConfigMap".into(),
        metadata: meta, data: HashMap::new(),
    });
    AdmissionRequest {
        uid: "uid".into(), tenant_id: tenant.into(),
        namespace: ns.into(), kind: "ConfigMap".into(), name: "cm1".into(),
        operation: op, object: Some(cm), old_object: None,
        user: "alice".into(), dry_run: false,
    }
}

fn make_input<'a>(
    ns: &'a str, op: &'a Operation,
    obj: &'a HashMap<String, String>, ns_l: &'a HashMap<String, String>,
) -> MatchInput<'a> {
    MatchInput {
        group: "", version: "v1", resource: "configmaps",
        namespace: ns, operation: op,
        object_labels: obj, namespace_labels: ns_l,
    }
}

fn mwc(name: &str, tenant: &str, hooks: Vec<MutatingWebhookSpec>) -> MutatingWebhookConfiguration {
    let mut meta = ObjectMeta::new(name, "");
    meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
    MutatingWebhookConfiguration {
        api_version: "admissionregistration.k8s.io/v1".into(),
        kind: "MutatingWebhookConfiguration".into(),
        metadata: meta, webhooks: hooks,
    }
}

fn vwc(name: &str, tenant: &str, hooks: Vec<ValidatingWebhookSpec>) -> ValidatingWebhookConfiguration {
    let mut meta = ObjectMeta::new(name, "");
    meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
    ValidatingWebhookConfiguration {
        api_version: "admissionregistration.k8s.io/v1".into(),
        kind: "ValidatingWebhookConfiguration".into(),
        metadata: meta, webhooks: hooks,
    }
}

fn mhook(name: &str, url: &str) -> MutatingWebhookSpec {
    MutatingWebhookSpec {
        name: name.into(),
        client_config: WebhookClientConfig { url: Some(url.into()), service: None, ca_bundle: vec![] },
        rules: vec![RuleWithOperations {
            operations: vec!["*".into()], api_groups: vec!["*".into()],
            api_versions: vec!["*".into()], resources: vec!["*".into()],
            scope: ScopeType::All,
        }],
        failure_policy: FailurePolicyType::Fail,
        match_policy: None, namespace_selector: None, object_selector: None,
        side_effects: SideEffectClass::None, timeout_seconds: 10,
        admission_review_versions: vec!["v1".into()],
        reinvocation_policy: ReinvocationPolicyType::Never,
        match_conditions: vec![],
    }
}

fn vhook(name: &str, url: &str) -> ValidatingWebhookSpec {
    ValidatingWebhookSpec {
        name: name.into(),
        client_config: WebhookClientConfig { url: Some(url.into()), service: None, ca_bundle: vec![] },
        rules: vec![RuleWithOperations {
            operations: vec!["*".into()], api_groups: vec!["*".into()],
            api_versions: vec!["*".into()], resources: vec!["*".into()],
            scope: ScopeType::All,
        }],
        failure_policy: FailurePolicyType::Fail,
        match_policy: None, namespace_selector: None, object_selector: None,
        side_effects: SideEffectClass::None, timeout_seconds: 10,
        admission_review_versions: vec!["v1".into()],
        match_conditions: vec![],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type round-trip
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn mwc_roundtrip() {
    let c = mwc("mwc-1", "acme", vec![mhook("h1", "https://h.example/")]);
    let s = serde_json::to_string(&c).unwrap();
    let c2: MutatingWebhookConfiguration = serde_json::from_str(&s).unwrap();
    assert_eq!(c2.metadata.name, "mwc-1");
    assert_eq!(c2.webhooks.len(), 1);
}

#[test]
fn vwc_roundtrip() {
    let c = vwc("vwc-1", "acme", vec![vhook("v1", "https://h.example/")]);
    let s = serde_json::to_string(&c).unwrap();
    let c2: ValidatingWebhookConfiguration = serde_json::from_str(&s).unwrap();
    assert_eq!(c2.webhooks.len(), 1);
}

#[test]
fn failure_policy_default_is_fail() {
    assert_eq!(FailurePolicyType::default(), FailurePolicyType::Fail);
}

#[test]
fn side_effects_default_is_unknown() {
    assert_eq!(SideEffectClass::default(), SideEffectClass::Unknown);
}

#[test]
fn reinvocation_default_is_never() {
    assert_eq!(ReinvocationPolicyType::default(), ReinvocationPolicyType::Never);
}

// ─────────────────────────────────────────────────────────────────────────────
// validate_client_config — `validation/validation.go::validateWebhookClientConfig`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn client_config_requires_url_or_service() {
    let c = WebhookClientConfig::default();
    assert_eq!(validate_client_config(&c), Err(WebhookValidationError::MissingClient));
}

#[test]
fn client_config_url_and_service_conflict() {
    let c = WebhookClientConfig {
        url: Some("https://x".into()),
        service: Some(ServiceReference { namespace: "default".into(), name: "svc".into(), path: None, port: None }),
        ca_bundle: vec![],
    };
    assert_eq!(validate_client_config(&c), Err(WebhookValidationError::ConflictingClient));
}

#[test]
fn client_config_url_must_be_https() {
    let c = WebhookClientConfig { url: Some("http://x".into()), service: None, ca_bundle: vec![] };
    assert_eq!(validate_client_config(&c), Err(WebhookValidationError::UrlNotHttps));
}

#[test]
fn client_config_https_url_ok() {
    let c = WebhookClientConfig { url: Some("https://x".into()), service: None, ca_bundle: vec![] };
    assert_eq!(validate_client_config(&c), Ok(()));
}

#[test]
fn client_config_service_only_ok() {
    let c = WebhookClientConfig {
        url: None,
        service: Some(ServiceReference { namespace: "default".into(), name: "svc".into(), path: None, port: Some(443) }),
        ca_bundle: vec![],
    };
    assert_eq!(validate_client_config(&c), Ok(()));
}

#[test]
fn client_config_service_port_out_of_range() {
    let c = WebhookClientConfig {
        url: None,
        service: Some(ServiceReference { namespace: "default".into(), name: "svc".into(), path: None, port: Some(0) }),
        ca_bundle: vec![],
    };
    assert_eq!(validate_client_config(&c), Err(WebhookValidationError::BadPort));
}

#[test]
fn client_config_service_port_too_high() {
    let c = WebhookClientConfig {
        url: None,
        service: Some(ServiceReference { namespace: "default".into(), name: "svc".into(), path: None, port: Some(65536) }),
        ca_bundle: vec![],
    };
    assert_eq!(validate_client_config(&c), Err(WebhookValidationError::BadPort));
}

// ─────────────────────────────────────────────────────────────────────────────
// validate_mutating_webhook / validate_validating_webhook
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn mutating_webhook_requires_review_version() {
    let mut h = mhook("h", "https://x");
    h.admission_review_versions = vec!["v3".into()];
    assert_eq!(validate_mutating_webhook(&h),
        Err(WebhookValidationError::NoSupportedReviewVersion));
}

#[test]
fn mutating_webhook_accepts_v1() {
    let h = mhook("h", "https://x");
    assert_eq!(validate_mutating_webhook(&h), Ok(()));
}

#[test]
fn mutating_webhook_accepts_v1beta1() {
    let mut h = mhook("h", "https://x");
    h.admission_review_versions = vec!["v1beta1".into()];
    assert_eq!(validate_mutating_webhook(&h), Ok(()));
}

#[test]
fn mutating_webhook_rejects_zero_timeout() {
    let mut h = mhook("h", "https://x");
    h.timeout_seconds = 0;
    assert_eq!(validate_mutating_webhook(&h), Err(WebhookValidationError::BadTimeout));
}

#[test]
fn mutating_webhook_rejects_high_timeout() {
    let mut h = mhook("h", "https://x");
    h.timeout_seconds = 31;
    assert_eq!(validate_mutating_webhook(&h), Err(WebhookValidationError::BadTimeout));
}

#[test]
fn validating_webhook_rejects_negative_timeout() {
    let mut h = vhook("h", "https://x");
    h.timeout_seconds = -1;
    assert_eq!(validate_validating_webhook(&h), Err(WebhookValidationError::BadTimeout));
}

#[test]
fn validating_webhook_default_timeout_is_10() {
    let h = vhook("h", "https://x");
    assert_eq!(h.timeout_seconds, 10);
}

// ─────────────────────────────────────────────────────────────────────────────
// CABundle parser — `cert.go::ParseCertsPEM`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn parse_ca_bundle_empty_errors() {
    assert_eq!(parse_ca_bundle(b""), Err(CaBundleError::Empty));
}

#[test]
fn parse_ca_bundle_single_certificate() {
    let pem = b"-----BEGIN CERTIFICATE-----\nABCDEF\n-----END CERTIFICATE-----\n";
    let p = parse_ca_bundle(pem).unwrap();
    assert_eq!(p.cert_count, 1);
}

#[test]
fn parse_ca_bundle_multiple_certificates() {
    let pem = b"-----BEGIN CERTIFICATE-----\nA\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nB\n-----END CERTIFICATE-----\n";
    let p = parse_ca_bundle(pem).unwrap();
    assert_eq!(p.cert_count, 2);
}

#[test]
fn parse_ca_bundle_rejects_non_certificate() {
    let pem = b"-----BEGIN RSA PRIVATE KEY-----\nA\n-----END RSA PRIVATE KEY-----\n";
    assert_eq!(parse_ca_bundle(pem), Err(CaBundleError::WrongPemKind));
}

#[test]
fn parse_ca_bundle_rejects_garbage() {
    let pem = b"this is not pem";
    assert_eq!(parse_ca_bundle(pem), Err(CaBundleError::NotPem));
}

// ─────────────────────────────────────────────────────────────────────────────
// Predicates — `predicates_test.go::TestMatcher`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn rule_matches_wildcard_groups_versions_resources() {
    let rule = RuleWithOperations {
        operations: vec!["*".into()],
        api_groups: vec!["*".into()],
        api_versions: vec!["*".into()],
        resources: vec!["*".into()],
        scope: ScopeType::All,
    };
    let op = Operation::Create;
    let empty = HashMap::new();
    let input = make_input("default", &op, &empty, &empty);
    assert!(rule_matches(&rule, &input));
}

#[test]
fn rule_filters_by_operation() {
    let rule = RuleWithOperations {
        operations: vec!["CREATE".into()],
        api_groups: vec!["*".into()],
        api_versions: vec!["*".into()],
        resources: vec!["*".into()],
        scope: ScopeType::All,
    };
    let empty = HashMap::new();
    let op_c = Operation::Create;
    let op_d = Operation::Delete;
    assert!(rule_matches(&rule, &make_input("default", &op_c, &empty, &empty)));
    assert!(!rule_matches(&rule, &make_input("default", &op_d, &empty, &empty)));
}

#[test]
fn rule_scope_namespaced_filters_cluster_resource() {
    let rule = RuleWithOperations {
        operations: vec!["*".into()],
        api_groups: vec!["*".into()],
        api_versions: vec!["*".into()],
        resources: vec!["*".into()],
        scope: ScopeType::Namespaced,
    };
    let empty = HashMap::new();
    let op = Operation::Create;
    assert!(rule_matches(&rule, &make_input("default", &op, &empty, &empty)));
    assert!(!rule_matches(&rule, &make_input("", &op, &empty, &empty)));
}

#[test]
fn label_selector_match_labels() {
    let mut s = LabelSelector::default();
    s.match_labels.insert("app".into(), "web".into());
    let mut l = HashMap::new();
    l.insert("app".into(), "web".into());
    assert!(label_selector_matches(&s, &l));
    l.insert("app".into(), "db".into());
    assert!(!label_selector_matches(&s, &l));
}

#[test]
fn label_selector_in_operator() {
    let s = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelSelectorRequirement {
            key: "tier".into(), operator: "In".into(),
            values: vec!["frontend".into()],
        }],
    };
    let mut l = HashMap::new();
    l.insert("tier".into(), "frontend".into());
    assert!(label_selector_matches(&s, &l));
}

#[test]
fn label_selector_does_not_exist() {
    let s = LabelSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![LabelSelectorRequirement {
            key: "absent".into(), operator: "DoesNotExist".into(),
            values: vec![],
        }],
    };
    assert!(label_selector_matches(&s, &HashMap::new()));
}

// ─────────────────────────────────────────────────────────────────────────────
// FakeWebhookClient — testing the testing tool
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn fake_client_returns_configured() {
    let c = FakeWebhookClient::new();
    c.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![], warnings: vec![],
    }));
    let cfg = WebhookClientConfig { url: Some("https://x".into()), service: None, ca_bundle: vec![] };
    let r = c.invoke("h1", &cfg, Duration::from_secs(1), &req(Operation::Create, "ns", "acme"));
    assert!(r.unwrap().allowed);
}

#[test]
fn fake_client_records_calls() {
    let c = FakeWebhookClient::new();
    c.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(), patches: vec![], warnings: vec![],
    }));
    let cfg = WebhookClientConfig { url: Some("https://x".into()), service: None, ca_bundle: vec![] };
    let r1 = req(Operation::Create, "ns", "acme");
    let r2 = req(Operation::Update, "ns", "acme");
    c.invoke("h1", &cfg, Duration::from_secs(1), &r1).unwrap();
    c.invoke("h1", &cfg, Duration::from_secs(1), &r2).unwrap();
    assert_eq!(c.call_count("h1"), 2);
    assert_eq!(c.total_calls(), 2);
}

#[test]
fn fake_client_propagates_error() {
    let c = FakeWebhookClient::new();
    c.answer("h1", Err(WebhookCallError::Timeout(10)));
    let cfg = WebhookClientConfig { url: Some("https://x".into()), service: None, ca_bundle: vec![] };
    let r = c.invoke("h1", &cfg, Duration::from_secs(1), &req(Operation::Create, "ns", "acme"));
    assert!(matches!(r, Err(WebhookCallError::Timeout(10))));
}

#[test]
#[should_panic(expected = "real TLS-backed webhook client")]
fn panic_client_panics() {
    let c = PanicWebhookClient;
    let cfg = WebhookClientConfig { url: Some("https://x".into()), service: None, ca_bundle: vec![] };
    let _ = c.invoke("any", &cfg, Duration::from_secs(1), &req(Operation::Create, "ns", "acme"));
}

// ─────────────────────────────────────────────────────────────────────────────
// MutatingDispatcher — `mutating/dispatcher_test.go`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn mutating_dispatch_invokes_matching_hook() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![], warnings: vec![],
    }));
    let d = MutatingDispatcher::new(client.clone());
    d.upsert(mwc("c", "acme", vec![mhook("h1", "https://x")]));
    let mut req = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    d.dispatch(&mut req, &input).unwrap();
    assert_eq!(client.call_count("h1"), 1);
}

#[test]
fn mutating_dispatch_accumulates_patches() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![JsonPatch { op: "add".into(), path: "/x".into(), value: None }],
        warnings: vec![],
    }));
    client.answer("h2", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![JsonPatch { op: "add".into(), path: "/y".into(), value: None }],
        warnings: vec![],
    }));
    let d = MutatingDispatcher::new(client);
    d.upsert(mwc("c", "acme", vec![mhook("h1", "https://x"), mhook("h2", "https://x")]));
    let mut req = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    let patches = d.dispatch(&mut req, &input).unwrap();
    assert_eq!(patches.len(), 2);
}

#[test]
fn mutating_dispatch_skips_other_tenant_config() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(), patches: vec![], warnings: vec![],
    }));
    let d = MutatingDispatcher::new(client.clone());
    d.upsert(mwc("c", "globex", vec![mhook("h1", "https://x")]));
    let mut req = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    d.dispatch(&mut req, &input).unwrap();
    assert_eq!(client.call_count("h1"), 0,
        "globex config must not run for acme request (cross-tenant invariant)");
}

#[test]
fn mutating_dispatch_failure_policy_fail_propagates() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Err(WebhookCallError::DialFailed));
    let d = MutatingDispatcher::new(client);
    d.upsert(mwc("c", "acme", vec![mhook("h1", "https://x")]));
    let mut req = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    assert!(d.dispatch(&mut req, &input).is_err());
}

#[test]
fn mutating_dispatch_failure_policy_ignore_continues() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Err(WebhookCallError::DialFailed));
    client.answer("h2", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![JsonPatch { op: "add".into(), path: "/y".into(), value: None }],
        warnings: vec![],
    }));
    let mut h1 = mhook("h1", "https://x");
    h1.failure_policy = FailurePolicyType::Ignore;
    let d = MutatingDispatcher::new(client);
    d.upsert(mwc("c", "acme", vec![h1, mhook("h2", "https://x")]));
    let mut req = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    let patches = d.dispatch(&mut req, &input).unwrap();
    assert_eq!(patches.len(), 1, "h2 still ran after h1 was Ignore-d");
}

#[test]
fn mutating_dispatch_skips_non_matching_rule() {
    let client = Arc::new(FakeWebhookClient::new());
    let mut h = mhook("h", "https://x");
    h.rules = vec![RuleWithOperations {
        operations: vec!["DELETE".into()],
        api_groups: vec!["*".into()],
        api_versions: vec!["*".into()],
        resources: vec!["*".into()],
        scope: ScopeType::All,
    }];
    let d = MutatingDispatcher::new(client.clone());
    d.upsert(mwc("c", "acme", vec![h]));
    let mut r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    d.dispatch(&mut r, &input).unwrap();
    assert_eq!(client.total_calls(), 0);
}

#[test]
fn mutating_dispatch_reinvocation_if_needed_runs_twice() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![JsonPatch { op: "add".into(), path: "/x".into(), value: None }],
        warnings: vec![],
    }));
    let mut h = mhook("h1", "https://x");
    h.reinvocation_policy = ReinvocationPolicyType::IfNeeded;
    let d = MutatingDispatcher::new(client.clone());
    d.upsert(mwc("c", "acme", vec![h]));
    let mut r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    d.dispatch(&mut r, &input).unwrap();
    assert_eq!(client.call_count("h1"), 2,
        "reinvocationPolicy=IfNeeded must call hook a second time in the reinvocation pass");
}

#[test]
fn mutating_dispatch_reinvocation_never_runs_once() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(), patches: vec![], warnings: vec![],
    }));
    let d = MutatingDispatcher::new(client.clone());
    d.upsert(mwc("c", "acme", vec![mhook("h1", "https://x")]));
    let mut r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    d.dispatch(&mut r, &input).unwrap();
    assert_eq!(client.call_count("h1"), 1);
}

#[test]
fn mutating_dispatch_dry_run_blocks_unsupported_side_effects() {
    let client = Arc::new(FakeWebhookClient::new());
    let mut h = mhook("h1", "https://x");
    h.side_effects = SideEffectClass::Some;
    h.failure_policy = FailurePolicyType::Fail;
    let d = MutatingDispatcher::new(client);
    d.upsert(mwc("c", "acme", vec![h]));
    let mut r = req(Operation::Create, "ns", "acme");
    r.dry_run = true;
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    assert!(d.dispatch(&mut r, &input).is_err(),
        "Some side-effects must fail dry-run requests");
}

#[test]
fn mutating_dispatch_dry_run_allows_none_on_dry_run() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(), patches: vec![], warnings: vec![],
    }));
    let mut h = mhook("h1", "https://x");
    h.side_effects = SideEffectClass::NoneOnDryRun;
    let d = MutatingDispatcher::new(client.clone());
    d.upsert(mwc("c", "acme", vec![h]));
    let mut r = req(Operation::Create, "ns", "acme");
    r.dry_run = true;
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    d.dispatch(&mut r, &input).unwrap();
    assert_eq!(client.call_count("h1"), 1);
}

#[test]
fn mutating_dispatch_rejects_tenant_id_mutation() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("h1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![JsonPatch {
            op: "replace".into(),
            path: "/metadata/annotations/cave.runtime~1tenant-id".into(),
            value: Some(serde_json::Value::String("globex".into())),
        }],
        warnings: vec![],
    }));
    let d = MutatingDispatcher::new(client);
    d.upsert(mwc("c", "acme", vec![mhook("h1", "https://x")]));
    let mut r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    assert!(d.dispatch(&mut r, &input).is_err(),
        "tenant_id mutation must be rejected (cave-runtime invariant)");
}

// ─────────────────────────────────────────────────────────────────────────────
// ValidatingDispatcher
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn validating_dispatch_allows_when_no_configs() {
    let client = Arc::new(FakeWebhookClient::new());
    let d = ValidatingDispatcher::new(client);
    let r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    let warnings = d.dispatch(&r, &input).unwrap();
    assert!(warnings.is_empty());
}

#[test]
fn validating_dispatch_denies_on_first_deny() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Ok(WebhookCallResult {
        allowed: false, status_code: 403, message: "policy says no".into(),
        patches: vec![], warnings: vec![],
    }));
    client.answer("v2", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(), patches: vec![], warnings: vec![],
    }));
    let d = ValidatingDispatcher::new(client.clone());
    d.upsert(vwc("c", "acme", vec![vhook("v1", "https://x"), vhook("v2", "https://x")]));
    let r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    let res = d.dispatch(&r, &input);
    assert!(matches!(res, Err((m, 403)) if m == "policy says no"));
    assert_eq!(client.call_count("v2"), 0,
        "post-deny webhook must NOT be invoked when short-circuiting");
}

#[test]
fn validating_dispatch_collects_warnings() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![], warnings: vec!["w1".into(), "w2".into()],
    }));
    let d = ValidatingDispatcher::new(client);
    d.upsert(vwc("c", "acme", vec![vhook("v1", "https://x")]));
    let r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    let warnings = d.dispatch(&r, &input).unwrap();
    assert_eq!(warnings, vec!["w1".to_string(), "w2".into()]);
}

#[test]
fn validating_dispatch_failure_policy_ignore_continues() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Err(WebhookCallError::DialFailed));
    let mut h = vhook("v1", "https://x");
    h.failure_policy = FailurePolicyType::Ignore;
    let d = ValidatingDispatcher::new(client);
    d.upsert(vwc("c", "acme", vec![h]));
    let r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    assert!(d.dispatch(&r, &input).is_ok());
}

#[test]
fn validating_dispatch_failure_policy_fail_errors() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Err(WebhookCallError::Timeout(10)));
    let d = ValidatingDispatcher::new(client);
    d.upsert(vwc("c", "acme", vec![vhook("v1", "https://x")]));
    let r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    assert!(d.dispatch(&r, &input).is_err());
}

#[test]
fn validating_dispatch_skips_other_tenant() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Ok(WebhookCallResult {
        allowed: false, status_code: 403, message: "would deny".into(),
        patches: vec![], warnings: vec![],
    }));
    let d = ValidatingDispatcher::new(client.clone());
    d.upsert(vwc("c", "globex", vec![vhook("v1", "https://x")]));
    let r = req(Operation::Create, "ns", "acme");
    let labs = HashMap::new();
    let op = Operation::Create;
    let input = make_input("ns", &op, &labs, &labs);
    assert!(d.dispatch(&r, &input).is_ok());
    assert_eq!(client.call_count("v1"), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// AdmissionChain wiring
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dispatcher_adapter_plugs_into_admission_chain() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(), patches: vec![], warnings: vec![],
    }));
    let vd = Arc::new(ValidatingDispatcher::new(client));
    vd.upsert(vwc("c", "acme", vec![vhook("v1", "https://x")]));
    let chain = AdmissionChain::new()
        .with_validating(Arc::new(ValidatingDispatcherAdapter { dispatcher: vd }));
    let r = req(Operation::Create, "ns", "acme");
    let (_, resp) = chain.dispatch(r);
    assert!(resp.allowed);
}

#[test]
fn dispatcher_adapter_warning_emitted_by_inner_response() {
    // The AdmissionChain rebuilds a fresh allow() response after all
    // validators pass, dropping per-hook warnings. The adapter still emits
    // them on its own return value, which we assert directly here.
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Ok(WebhookCallResult {
        allowed: true, status_code: 200, message: "".into(),
        patches: vec![], warnings: vec!["careful".into()],
    }));
    let vd = Arc::new(ValidatingDispatcher::new(client));
    vd.upsert(vwc("c", "acme", vec![vhook("v1", "https://x")]));
    let adapter = ValidatingDispatcherAdapter { dispatcher: vd };
    let r = req(Operation::Create, "ns", "acme");
    let resp = adapter.validate(&r);
    assert!(resp.allowed);
    assert_eq!(resp.warnings, vec!["careful".to_string()]);
}

#[test]
fn dispatcher_adapter_deny_translates_to_response() {
    let client = Arc::new(FakeWebhookClient::new());
    client.answer("v1", Ok(WebhookCallResult {
        allowed: false, status_code: 422, message: "rejected".into(),
        patches: vec![], warnings: vec![],
    }));
    let vd = Arc::new(ValidatingDispatcher::new(client));
    vd.upsert(vwc("c", "acme", vec![vhook("v1", "https://x")]));
    let chain = AdmissionChain::new()
        .with_validating(Arc::new(ValidatingDispatcherAdapter { dispatcher: vd }));
    let r = req(Operation::Create, "ns", "acme");
    let (_, resp) = chain.dispatch(r);
    assert!(!resp.allowed);
    assert_eq!(resp.status_code, 422);
    assert_eq!(resp.status_message, "rejected");
}

// ─────────────────────────────────────────────────────────────────────────────
// `#[ignore]` — gated on real TLS / CABundle verification / cel matchConditions.
// ─────────────────────────────────────────────────────────────────────────────

#[test] #[ignore]
fn real_tls_dial_against_mock_server() {
    // upstream: webhook/integration_test.go::TestAdmissionWebhook_TLS
    todo!("requires real HTTPS server + rustls handshake");
}

#[test] #[ignore]
fn ca_bundle_pins_self_signed_cert() {
    todo!("requires real X.509 verification — pin the test CA, reject others");
}

#[test] #[ignore]
fn timeout_seconds_aborts_slow_webhook() {
    todo!("requires async runtime + deadline; FakeWebhookClient is synchronous");
}

#[test] #[ignore]
fn cluster_dns_resolves_service_reference() {
    todo!("requires Service resolver — `kubernetes.default.svc` style");
}

#[test] #[ignore]
fn match_condition_cel_filters_invocation() {
    // upstream: matchconditions/matcher_test.go
    todo!("requires CEL evaluator (shared with VAP) — `request.userInfo.username == 'x'`");
}

#[test] #[ignore]
fn match_condition_cel_failure_policy_fail() {
    todo!("requires CEL — runtime error in matchCondition + Fail policy");
}

#[test] #[ignore]
fn match_condition_cel_failure_policy_ignore() {
    todo!("requires CEL — runtime error in matchCondition + Ignore policy");
}

#[test] #[ignore]
fn audit_annotations_emitted_on_response() {
    todo!("M4 hand-off — audit framework produces annotation map");
}

#[test] #[ignore]
fn webhook_returns_v1_admission_review() {
    todo!("requires real HTTP body parser for AdmissionReview v1");
}

#[test] #[ignore]
fn webhook_v1beta1_compatibility() {
    todo!("requires AdmissionReview v1beta1 conversion shim");
}
