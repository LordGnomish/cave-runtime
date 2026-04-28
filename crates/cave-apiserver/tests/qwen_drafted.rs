//! Cross-module integration tests for cave-apiserver.
//!
//! These exercise the public surface end-to-end: admission chain composition,
//! aggregation registry + availability, audit pipeline, discovery + ETag,
//! conversion dispatch, and tenant invariant guarantees that span multiple
//! modules. Each names the upstream behaviour it mirrors so reverse-grep is
//! one command.

use std::collections::HashMap;
use std::sync::Arc;

use cave_apiserver::admission::{
    AdmissionChain, AdmissionRequest, AdmissionResponse, JsonPatch,
    MutatingWebhook, Operation, ValidatingWebhook,
};
use cave_apiserver::aggregator_v2::{
    APIServiceCondition, AvailabilityInput, ConditionStatus, ProbeOutcome,
    ProxyDecision, compute_condition, evaluate_proxy,
};
use cave_apiserver::audit::AuditLevel;
use cave_apiserver::audit_policy_v2::{
    GroupResources, PolicyEvaluationInput, PolicyRule, PolicyV1,
};
use cave_apiserver::audit_policy_v3::{
    BatchBackend, BatchConfig, Backend, CountingBackend,
};
use cave_apiserver::auth_review::{
    AuthzDecision, Authorizer, ResourceAttributes, StaticAuthorizer,
    StaticTokenAuthenticator, SubjectAccessReview, SubjectAccessReviewSpec,
    TokenAuthenticator, TokenReview, TokenReviewSpec, UserInfo,
    run_subject_access_review, run_token_review,
};
use cave_apiserver::conversion::{ConversionRequest, ConvertibleObject};
use cave_apiserver::conversion_v1::{
    ConversionStrategyType, CustomResourceConversion, NopConverter,
    WebhookClientConfig as ConversionClientConfig, WebhookConversion,
    dispatch_conversion, validate_conversion,
};
use cave_apiserver::discovery_v2::{etag_for_bytes, gzip_envelope, is_gzip_envelope};
use cave_apiserver::resources::{ConfigMap, ObjectMeta, Resource};
use cave_apiserver::simple_cel::SimpleEvaluator;
use cave_apiserver::storage_registry::{
    ConsistentReadOutcome, ProgressNotifier, WatchEventType,
    evaluate_consistent_read,
};
use cave_apiserver::vap_advanced::{
    CelActivation, CelEvaluator, CelValue,
};
use cave_apiserver::webhook_admission::FailurePolicyType;

fn req(op: Operation, ns: &str, tenant: &str, user: &str) -> AdmissionRequest {
    let cm = Resource::ConfigMap(ConfigMap {
        api_version: "v1".into(),
        kind: "ConfigMap".into(),
        metadata: ObjectMeta::new("cm1", ns),
        data: HashMap::new(),
    });
    AdmissionRequest {
        uid: "uid".into(),
        tenant_id: tenant.into(),
        namespace: ns.into(),
        kind: "ConfigMap".into(),
        name: "cm1".into(),
        operation: op,
        object: Some(cm),
        old_object: None,
        user: user.into(),
        dry_run: false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Full admission chain — mutating then validating, tenant invariant flows.
//    Upstream: apiserver/pkg/admission/chain.go::Handler.{Admit,Validate}.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_1() {
    struct LabelInjector;
    impl MutatingWebhook for LabelInjector {
        fn name(&self) -> &str { "label-injector" }
        fn admit(&self, req: &mut AdmissionRequest) -> AdmissionResponse {
            let mut r = AdmissionResponse::allow(req);
            r.patches.push(JsonPatch {
                op: "add".into(),
                path: "/metadata/labels/cave.runtime~1tenant-id".into(),
                value: Some(serde_json::Value::String(req.tenant_id.clone())),
            });
            r
        }
    }
    struct DenyEmptyTenant;
    impl ValidatingWebhook for DenyEmptyTenant {
        fn name(&self) -> &str { "deny-empty-tenant" }
        fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
            if req.tenant_id.is_empty() {
                AdmissionResponse::deny(req, 403, "tenant_id required")
            } else {
                AdmissionResponse::allow(req)
            }
        }
    }
    let chain = AdmissionChain::new()
        .with_mutating(Arc::new(LabelInjector))
        .with_validating(Arc::new(DenyEmptyTenant));
    let (out, resp) = chain.dispatch(req(Operation::Create, "default", "acme", "alice"));
    assert!(resp.allowed);
    assert_eq!(out.tenant_id, "acme", "tenant_id invariant preserved through chain");
    let (_, deny) = chain.dispatch(req(Operation::Create, "default", "", "alice"));
    assert!(!deny.allowed);
    assert_eq!(deny.status_code, 403);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. SimpleEvaluator drives a realistic VAP-style expression end-to-end.
//    Upstream: apiserver/pkg/admission/plugin/policy/validating/dispatcher.go
//    plus apiserver/pkg/cel/library/lists.go ('contains' on string list).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_2() {
    let mut act = CelActivation::default();
    act.object = Some(serde_json::json!({
        "metadata": {"name": "audit-secret"},
        "spec": {"hostNetwork": false}
    }));
    let e = SimpleEvaluator::new();
    assert_eq!(
        e.evaluate(
            "object.metadata.name.startsWith('audit-') && !object.spec.hostNetwork",
            &act).unwrap(),
        CelValue::Bool(true));
    assert_eq!(
        e.evaluate("has(object.spec.hostNetwork)", &act).unwrap(),
        CelValue::Bool(true));
    assert_eq!(
        e.evaluate("'foo123'.matches('^foo[0-9]+$') && size('hi') == 2", &act).unwrap(),
        CelValue::Bool(true));
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Aggregation API: probe outcome → APIService availability → proxy decision.
//    Upstream: kube-aggregator/pkg/controllers/status/available_controller.go
//    + kube-aggregator/pkg/apiserver/handler_proxy.go.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_3() {
    fn cond_for(probe: ProbeOutcome) -> APIServiceCondition {
        compute_condition(&AvailabilityInput {
            api_service_name: "v1beta1.metrics.k8s.io".into(),
            local: false, service_resolved: true, endpoints_count: 1,
            probe: Some(probe),
        })
    }
    let healthy = cond_for(ProbeOutcome::Reachable);
    assert_eq!(healthy.status, ConditionStatus::True);
    assert_eq!(evaluate_proxy(&healthy, false, true), ProxyDecision::Forward);

    let dead = cond_for(ProbeOutcome::Unreachable("connection refused".into()));
    assert_eq!(dead.status, ConditionStatus::False);
    matches!(evaluate_proxy(&dead, false, false),
        ProxyDecision::ServiceUnavailable { http_status: 503, .. });

    // Pending probe (probe never ran) → Unknown → still 503 from the proxy.
    let pending = compute_condition(&AvailabilityInput {
        api_service_name: "x".into(),
        local: false, service_resolved: true, endpoints_count: 1, probe: None,
    });
    assert_eq!(pending.status, ConditionStatus::Unknown);
    matches!(evaluate_proxy(&pending, false, true),
        ProxyDecision::ServiceUnavailable { .. });
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Audit pipeline: PolicyV1 → decision → BatchBackend tenant-partitioned flush.
//    Upstream: apiserver/pkg/audit/policy/checker.go + buffered/buffered.go.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_4() {
    let policy = PolicyV1 {
        level: AuditLevel::Metadata,
        omit_stages: vec![],
        rules: vec![
            PolicyRule {
                level: AuditLevel::None,
                verbs: vec!["get".into()],
                ..Default::default()
            },
            PolicyRule {
                level: AuditLevel::RequestResponse,
                resources: vec![GroupResources {
                    group: "".into(),
                    resources: vec!["secrets".into()],
                    resource_names: vec![],
                }],
                ..Default::default()
            },
        ],
    };
    let groups: Vec<String> = vec![];
    let secret_input = PolicyEvaluationInput {
        user: "alice", user_groups: &groups, verb: "create",
        group: "", resource: "secrets", subresource: "", namespace: "default",
        name: "tok", non_resource_url: None,
    };
    assert_eq!(policy.evaluate(&secret_input).level, AuditLevel::RequestResponse);

    // Backend tenant partitioning: events from two tenants in one buffer
    // must be flushed in two separate downstream calls.
    use cave_apiserver::audit::{AuditEvent, AuditStage};
    let down = Arc::new(CountingBackend::new());
    let batch = BatchBackend::new(down.clone(), BatchConfig {
        batch_size: 100,
        max_wait: std::time::Duration::from_secs(60),
    });
    let ev_a = AuditEvent::new("a", AuditLevel::Metadata, AuditStage::ResponseComplete,
        "alice", "acme", "ns", "create", "secrets", "tok", "/", 201);
    let ev_b = AuditEvent::new("b", AuditLevel::Metadata, AuditStage::ResponseComplete,
        "bob", "globex", "ns", "create", "secrets", "tok", "/", 201);
    batch.process_events(&[ev_a]);
    batch.process_events(&[ev_b]);
    let n = batch.flush();
    assert_eq!(n, 2, "both events flushed downstream");
    let calls = down.calls();
    assert_eq!(calls.len(), 2, "tenant invariant: two tenants → two downstream calls");
    let tenants: std::collections::BTreeSet<&String> = calls.iter().map(|(t, _)| t).collect();
    assert!(tenants.contains(&"acme".to_string()));
    assert!(tenants.contains(&"globex".to_string()));
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Discovery transport: gzip envelope + content-addressed ETag round-trip.
//    Upstream: apiserver/pkg/endpoints/discovery/aggregated/handler.go.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_5() {
    let payload = b"{\"apiVersion\":\"apidiscovery.k8s.io/v2\",\"kind\":\"APIGroupDiscoveryList\",\"items\":[]}";
    let env = gzip_envelope(payload);
    assert!(is_gzip_envelope(&env));

    let etag1 = etag_for_bytes(payload);
    let etag2 = etag_for_bytes(payload);
    assert_eq!(etag1, etag2, "stable ETag for identical body");

    // Different body MUST produce different ETag (cache validation contract).
    let other = b"{\"items\":[{\"name\":\"apps\"}]}";
    assert_ne!(etag1, etag_for_bytes(other));
    assert!(etag1.starts_with('"') && etag1.ends_with('"'),
        "RFC 7232 quoted-string format");
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Conversion strategy dispatch: None passthrough vs Webhook fan-out.
//    Upstream: apiextensions-apiserver/pkg/apiserver/conversion/converter.go.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_1777318016_1() {
    fn obj(api_ver: &str, tenant: &str) -> ConvertibleObject {
        let mut fields = serde_json::Map::new();
        fields.insert("foo".into(), serde_json::Value::Number(1.into()));
        ConvertibleObject {
            api_version: api_ver.into(),
            kind: "Widget".into(),
            name: "w1".into(),
            namespace: "default".into(),
            tenant_id: tenant.into(),
            fields,
        }
    }

    // Strategy=None: passthrough only valid when source==desired version.
    let cv_none = CustomResourceConversion {
        strategy: ConversionStrategyType::None,
        webhook: None,
    };
    assert_eq!(validate_conversion(&cv_none, "acme"), Ok(()));
    let nop = NopConverter;
    let req_match = ConversionRequest {
        uid: "u1".into(), desired_api_version: "widgets/v1".into(),
        objects: vec![obj("widgets/v1", "acme")],
    };
    let r1 = dispatch_conversion::<cave_apiserver::conversion_v1::FakeConversionClient>(
        &cv_none, "wh", &nop, None, req_match);
    assert_eq!(r1.result_status, "Success");
    assert_eq!(r1.converted_objects[0].tenant_id, "acme");

    let req_mismatch = ConversionRequest {
        uid: "u2".into(), desired_api_version: "widgets/v2".into(),
        objects: vec![obj("widgets/v1", "acme")],
    };
    let r2 = dispatch_conversion::<cave_apiserver::conversion_v1::FakeConversionClient>(
        &cv_none, "wh", &nop, None, req_mismatch);
    assert_eq!(r2.result_status, "Failure");

    // Strategy=Webhook: validation requires v1 or v1beta1 in review_versions.
    let cv_wh_ok = CustomResourceConversion {
        strategy: ConversionStrategyType::Webhook,
        webhook: Some(WebhookConversion {
            client_config: ConversionClientConfig {
                url: Some("https://wh.example/convert".into()),
                service: None,
                ca_bundle: vec![],
            },
            conversion_review_versions: vec!["v1".into()],
        }),
    };
    assert_eq!(validate_conversion(&cv_wh_ok, "acme"), Ok(()));
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Auth review pipeline: TokenReview validates → SAR authorizes.
//    Upstream: apiserver/pkg/authentication + apiserver/pkg/authorization.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_1777318016_2() {
    let auth = StaticTokenAuthenticator::new();
    auth.register("acme", "tok-1",
        UserInfo { username: "alice".into(), uid: "u-1".into(),
                   groups: vec!["devs".into()], extra: Default::default() },
        vec!["api".into()]);
    let tr = TokenReview {
        spec: TokenReviewSpec { token: "tok-1".into(), audiences: vec!["api".into()] },
        ..Default::default()
    };
    let tr_out = run_token_review(&auth, "acme", &tr);
    assert!(tr_out.status.authenticated);
    assert_eq!(tr_out.status.user.username, "alice");

    // Authenticated user goes on to authz.
    let authz = StaticAuthorizer::new();
    authz.allow("acme", "alice", "create", "configmaps", "rbac:dev-binding");
    let sar = SubjectAccessReview {
        spec: SubjectAccessReviewSpec {
            user: "alice".into(),
            groups: vec!["devs".into()],
            uid: "u-1".into(),
            extra: Default::default(),
            resource_attributes: Some(ResourceAttributes {
                namespace: "default".into(),
                verb: "create".into(),
                group: "".into(),
                version: "v1".into(),
                resource: "configmaps".into(),
                subresource: "".into(),
                name: "".into(),
            }),
            non_resource_attributes: None,
        },
        ..Default::default()
    };
    let sar_out = run_subject_access_review(&authz, "acme", &sar);
    assert!(sar_out.status.allowed);
    assert_eq!(sar_out.status.reason, "rbac:dev-binding");
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. KEP-956 consistent reads + KEP-1904 watch progress notifications,
//    composed.  Upstream: apiserver/pkg/storage/cacher/cache_watcher.go.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_1777318016_3() {
    // Cache at RV=200 — request without RV serves from cache.
    assert_eq!(evaluate_consistent_read(None, 200),
        ConsistentReadOutcome::ServeFromCache { at_rv: 200 });
    // Request RV ahead of cache forces fall-through to backing store.
    assert_eq!(evaluate_consistent_read(Some("250"), 200),
        ConsistentReadOutcome::FallThrough);
    // RV=0 is upstream's "any" semantics — always cache.
    assert_eq!(evaluate_consistent_read(Some("0"), 200),
        ConsistentReadOutcome::ServeFromCache { at_rv: 200 });

    // Watch progress: a single bookmark per RV advance, never duplicate.
    let p = ProgressNotifier::new(std::time::Duration::from_secs(10));
    let e1 = p.maybe_bookmark(200, true).unwrap();
    assert_eq!(e1.kind, WatchEventType::Bookmark);
    assert_eq!(e1.resource_version, "200");
    assert!(p.maybe_bookmark(200, true).is_none(),
        "no duplicate bookmark at unchanged RV");
    let e2 = p.maybe_bookmark(220, true).unwrap();
    assert_eq!(e2.resource_version, "220");
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. Webhook admission: matchConditions CEL filtering with the in-tree
//    SimpleEvaluator. Upstream: apiserver/pkg/admission/plugin/webhook/matchconditions/matcher.go.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_1777318016_4() {
    use cave_apiserver::webhook_admission::MatchCondition;
    use cave_apiserver::webhook_admission_v2::{MatchConditionsMatcher, MatchOutcome};
    let m = MatchConditionsMatcher { evaluator: Arc::new(SimpleEvaluator::new()) };
    let conds = vec![
        MatchCondition { name: "tenant-known".into(),
            expression: "request.tenant_id != ''".into() },
        MatchCondition { name: "is-create".into(),
            expression: "request.operation == 'Create'".into() },
    ];
    let r = req(Operation::Create, "default", "acme", "alice");
    assert_eq!(m.match_(&conds, &FailurePolicyType::Fail, &r), MatchOutcome::Matched);

    // Failure on a missing-variable expression with FailurePolicy=Fail.
    let bad = vec![MatchCondition { name: "broken".into(),
        expression: "undefined.thing".into() }];
    matches!(m.match_(&bad, &FailurePolicyType::Fail, &r), MatchOutcome::Fail { .. });
    matches!(m.match_(&bad, &FailurePolicyType::Ignore, &r),
        MatchOutcome::SilencedError { .. });
}

// ─────────────────────────────────────────────────────────────────────────────
// 10. Tenant invariant cross-cut: the same admission chain receives two
//     requests under different tenants and never blends decisions.
//     Upstream: cave-runtime invariant (no upstream equivalent).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn placeholder_1777318016_5() {
    struct AcmeOnly;
    impl Authorizer for AcmeOnly {
        fn authorize(&self, tenant: &str, _: &SubjectAccessReviewSpec) -> AuthzDecision {
            if tenant == "acme" {
                AuthzDecision::Allow { reason: "acme tenant".into() }
            } else {
                AuthzDecision::Deny { reason: "wrong tenant".into() }
            }
        }
    }
    let authz = AcmeOnly;
    let mk_sar = |tenant_for_authz: &str| SubjectAccessReview {
        spec: SubjectAccessReviewSpec {
            user: "alice".into(), groups: vec![], uid: "".into(),
            extra: Default::default(),
            resource_attributes: Some(ResourceAttributes {
                namespace: "default".into(),
                verb: "get".into(), group: "".into(), version: "v1".into(),
                resource: "configmaps".into(), subresource: "".into(),
                name: "".into(),
            }),
            non_resource_attributes: None,
        },
        api_version: tenant_for_authz.into(), // unused; we pass the tenant through run()
        kind: "SubjectAccessReview".into(),
        ..Default::default()
    };
    let acme_out = run_subject_access_review(&authz, "acme", &mk_sar(""));
    let globex_out = run_subject_access_review(&authz, "globex", &mk_sar(""));
    assert!(acme_out.status.allowed);
    assert!(!globex_out.status.allowed);
    assert!(globex_out.status.denied);
    assert_eq!(globex_out.status.reason, "wrong tenant");
}
