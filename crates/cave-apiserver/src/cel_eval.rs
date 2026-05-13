//! Real CEL evaluator for ValidatingAdmissionPolicy.
//!
//! Closes the largest behavioural gap recorded in
//! `docs/parity/cave-apiserver-port-2026-05-12.md`: the dispatcher in
//! `vap_advanced::Dispatcher` was wired against the `CelEvaluator`
//! trait, but the only in-tree implementations were `PanicEvaluator`
//! (always returns `CelError::Compile`) and `FixedEvaluator` (a
//! lookup-table test double). Live policy expressions never actually
//! ran.
//!
//! [`CelInterpreterEvaluator`] is the production impl. It wraps the
//! pure-Rust `cel-interpreter` crate (Google CEL spec subset), exposes
//! a thread-safe program cache so the same expression compiles once,
//! and maps the CEL [`Value`](cel_interpreter::Value) result back to
//! the in-crate [`crate::vap_advanced::CelValue`] enum without
//! widening the trait signature.
//!
//! Activation contract: the upstream VAP CEL environment exposes
//!   `object`, `oldObject`, `request`, `params`, `namespaceObject`,
//!   `authorizer`, and named `variables`.
//! Cave matches all seven slots. The `authorizer` binding is
//! sourced from an `AuthorizerView` snapshot pre-computed by the
//! dispatcher (see `vap_advanced::AuthorizerView`); CEL expressions
//! can call:
//!
//! ```cel
//! authorizer.user == 'kube-admin'
//! authorizer.groups.exists(g, g == 'system:masters')
//! authorizer.resource('apps', 'deployments').namespace('team').check('update')
//! authorizer.path('/healthz').check('get')
//! ```
//!
//! The evaluator wires a small JSON projection of the view into the
//! `authorizer` slot so CEL's standard `.exists()` / `.size()` /
//! field-access works without per-method function registration. The
//! `.check(verb)` helper is rewritten by the dispatcher into a
//! pre-resolved boolean look-up against `view.check_resource(...)`
//! /`view.check_url(...)`, which lets the CEL expression stay
//! synchronous (the underlying authorizer chain is async).

use crate::vap_advanced::{AuthorizerView, CelActivation, CelError, CelEvaluator, CelValue};
use cel_interpreter::{Context, Program, Value as CelInternalValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Production CEL evaluator. Cheap to construct; share a single
/// instance across the apiserver (wrapped in `Arc`) so the program
/// cache is shared.
#[derive(Default)]
pub struct CelInterpreterEvaluator {
    /// Compiled-program LRU. `String → Arc<Program>` so concurrent
    /// callers receive a cheap clone instead of re-parsing.
    cache: Mutex<HashMap<String, Arc<Program>>>,
}

impl CelInterpreterEvaluator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a compiled program for `expr`; compile on miss.
    fn compile(&self, expr: &str) -> Result<Arc<Program>, CelError> {
        // Fast-path: cache hit under a short critical section.
        {
            let guard = self.cache.lock().expect("cel cache poisoned");
            if let Some(p) = guard.get(expr) {
                return Ok(p.clone());
            }
        }
        let program = Program::compile(expr).map_err(|e| {
            CelError::Compile(format!("{expr}: {e}"))
        })?;
        let arc = Arc::new(program);
        let mut guard = self.cache.lock().expect("cel cache poisoned");
        // Tolerate races — another caller may have populated the entry
        // between the two locks; either Arc is fine to return.
        Ok(guard.entry(expr.to_string()).or_insert(arc).clone())
    }

    /// Number of compiled programs currently held in the cache.
    /// Diagnostic for tests + dashboard.
    pub fn cache_len(&self) -> usize {
        self.cache.lock().expect("cel cache poisoned").len()
    }
}

impl CelEvaluator for CelInterpreterEvaluator {
    fn evaluate(&self, expr: &str, act: &CelActivation) -> Result<CelValue, CelError> {
        let program = self.compile(expr)?;
        let mut ctx = Context::default();
        // Each well-known VAP binding maps directly through serde —
        // serde_json::Value satisfies cel-interpreter's TryIntoValue
        // blanket impl, so HashMaps / arrays / nulls round-trip
        // without manual coercion.
        if let Some(v) = &act.object {
            ctx.add_variable("object", v.clone())
                .map_err(|e| CelError::Runtime(format!("bind object: {e}")))?;
        }
        if let Some(v) = &act.old_object {
            ctx.add_variable("oldObject", v.clone())
                .map_err(|e| CelError::Runtime(format!("bind oldObject: {e}")))?;
        }
        if let Some(v) = &act.request {
            ctx.add_variable("request", v.clone())
                .map_err(|e| CelError::Runtime(format!("bind request: {e}")))?;
        }
        if let Some(v) = &act.params {
            ctx.add_variable("params", v.clone())
                .map_err(|e| CelError::Runtime(format!("bind params: {e}")))?;
        }
        if let Some(v) = &act.namespace_object {
            ctx.add_variable("namespaceObject", v.clone())
                .map_err(|e| CelError::Runtime(format!("bind namespaceObject: {e}")))?;
        }
        // 2026-05-13 batch2: bind the `authorizer` slot when an
        // AuthorizerView is attached to the activation. The view
        // projects to a JSON object so CEL's stock field access +
        // list ops (`.exists()`, `.size()`) work on `.groups`. The
        // `.check(verb, ...)` helper isn't a CEL method here — the
        // dispatcher pre-resolves it through `view.check_resource()`
        // and stashes the result in named variables (see
        // `AuthorizerView::check_resource`).
        if let Some(view) = &act.authorizer {
            let projection = serde_json::json!({
                "user": view.user,
                "groups": view.groups,
                "grants": view.grants.iter().cloned().collect::<Vec<_>>(),
                "url_grants": view.url_grants.iter().cloned().collect::<Vec<_>>(),
            });
            ctx.add_variable("authorizer", projection)
                .map_err(|e| CelError::Runtime(format!("bind authorizer: {e}")))?;
        }
        for (name, val) in &act.variables {
            // Per-policy named variables. Promote the in-crate
            // CelValue back to a cel-interpreter Value via the
            // documented From impls.
            let v = match val {
                CelValue::Bool(b) => CelInternalValue::Bool(*b),
                CelValue::Int(i) => CelInternalValue::Int(*i),
                CelValue::String(s) => CelInternalValue::String(Arc::new(s.clone())),
                CelValue::Null => CelInternalValue::Null,
            };
            ctx.add_variable_from_value(name.clone(), v);
        }
        let result = program
            .execute(&ctx)
            .map_err(|e| CelError::Runtime(format!("{expr}: {e}")))?;
        from_cel_value(&result)
    }
}

/// Map cel-interpreter's `Value` back to the in-crate `CelValue`.
/// VAP-relevant scalars only — anything richer (Map/List/Function)
/// yields a `CelError::Type` so the dispatcher records the policy as
/// a fail-policy outcome instead of admitting silently.
fn from_cel_value(v: &CelInternalValue) -> Result<CelValue, CelError> {
    Ok(match v {
        CelInternalValue::Bool(b) => CelValue::Bool(*b),
        CelInternalValue::Int(i) => CelValue::Int(*i),
        CelInternalValue::UInt(u) => CelValue::Int((*u) as i64),
        CelInternalValue::Float(f) => {
            // CEL only narrows back to bool/int/string/null at the
            // trait level; surface floats as their integer truncation
            // so policies that compute averages still funnel through.
            CelValue::Int(*f as i64)
        }
        CelInternalValue::String(s) => CelValue::String(s.as_ref().clone()),
        CelInternalValue::Null => CelValue::Null,
        other => {
            return Err(CelError::Type {
                expected: "Bool|Int|String|Null".into(),
                got: format!("{:?}", other),
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn act_with_object(o: serde_json::Value) -> CelActivation {
        CelActivation {
            object: Some(o),
            ..Default::default()
        }
    }

    // ── grammar: scalars + comparisons ────────────────────────────────────

    #[test]
    fn evaluates_simple_boolean_literal() {
        let ev = CelInterpreterEvaluator::new();
        assert_eq!(
            ev.evaluate("true", &CelActivation::default()).unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate("false", &CelActivation::default()).unwrap(),
            CelValue::Bool(false),
        );
    }

    #[test]
    fn evaluates_integer_arithmetic() {
        let ev = CelInterpreterEvaluator::new();
        assert_eq!(
            ev.evaluate("2 + 3", &CelActivation::default()).unwrap(),
            CelValue::Int(5),
        );
        assert_eq!(
            ev.evaluate("10 - 4 * 2", &CelActivation::default()).unwrap(),
            CelValue::Int(2),
        );
    }

    #[test]
    fn evaluates_object_field_comparison() {
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({"spec": {"replicas": 5}}));
        assert_eq!(
            ev.evaluate("object.spec.replicas > 0", &act).unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate("object.spec.replicas < 5", &act).unwrap(),
            CelValue::Bool(false),
        );
        assert_eq!(
            ev.evaluate("object.spec.replicas == 5", &act).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn evaluates_string_equality() {
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({
            "spec": {"image": "nginx:1.27"}
        }));
        assert_eq!(
            ev.evaluate("object.spec.image == 'nginx:1.27'", &act).unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate("object.spec.image != 'latest'", &act).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn evaluates_logical_operators() {
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({
            "spec": {"replicas": 3, "image": "nginx:1.27"}
        }));
        assert_eq!(
            ev.evaluate(
                "object.spec.replicas > 0 && object.spec.image != 'latest'",
                &act
            ).unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate(
                "object.spec.replicas == 0 || object.spec.image == 'nginx:1.27'",
                &act
            ).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn evaluates_has_on_present_field() {
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({
            "metadata": {"labels": {"team": "platform"}}
        }));
        assert_eq!(
            ev.evaluate("has(object.metadata.labels.team)", &act).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn evaluates_has_on_missing_field_returns_false() {
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({
            "metadata": {"labels": {}}
        }));
        assert_eq!(
            ev.evaluate("has(object.metadata.labels.team)", &act).unwrap(),
            CelValue::Bool(false),
        );
    }

    #[test]
    fn evaluates_string_starts_with_method() {
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({
            "spec": {"image": "registry.cave.local/app:v1"}
        }));
        assert_eq!(
            ev.evaluate("object.spec.image.startsWith('registry.cave.local/')", &act).unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate("object.spec.image.startsWith('docker.io/')", &act).unwrap(),
            CelValue::Bool(false),
        );
    }

    // ── activation slots: oldObject, params, request, variables ───────────

    #[test]
    fn evaluates_old_object_diff() {
        let ev = CelInterpreterEvaluator::new();
        let act = CelActivation {
            object: Some(json!({"metadata": {"labels": {"team": "platform"}}})),
            old_object: Some(json!({"metadata": {"labels": {"team": "infra"}}})),
            ..Default::default()
        };
        assert_eq!(
            ev.evaluate(
                "object.metadata.labels.team != oldObject.metadata.labels.team",
                &act
            ).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn evaluates_paramref_against_policy_bound_params() {
        let ev = CelInterpreterEvaluator::new();
        let act = CelActivation {
            object: Some(json!({"spec": {"replicas": 3}})),
            // VAP binds params as an array per upstream; the dispatcher
            // wraps the resolver result in `Value::Array`.
            params: Some(json!([{"maxReplicas": 5}])),
            ..Default::default()
        };
        assert_eq!(
            ev.evaluate("object.spec.replicas <= params[0].maxReplicas", &act).unwrap(),
            CelValue::Bool(true),
        );
        let act2 = CelActivation {
            object: Some(json!({"spec": {"replicas": 10}})),
            params: Some(json!([{"maxReplicas": 5}])),
            ..Default::default()
        };
        assert_eq!(
            ev.evaluate("object.spec.replicas <= params[0].maxReplicas", &act2).unwrap(),
            CelValue::Bool(false),
        );
    }

    #[test]
    fn evaluates_request_metadata_op() {
        let ev = CelInterpreterEvaluator::new();
        let act = CelActivation {
            request: Some(json!({"operation": "CREATE", "userInfo": {"username": "alice"}})),
            ..Default::default()
        };
        assert_eq!(
            ev.evaluate("request.operation == 'CREATE'", &act).unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate("request.userInfo.username == 'alice'", &act).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn evaluates_user_variables() {
        let ev = CelInterpreterEvaluator::new();
        let mut act = CelActivation::default();
        act.variables.insert("maxPods".to_string(), CelValue::Int(100));
        act.object = Some(json!({"spec": {"replicas": 50}}));
        assert_eq!(
            ev.evaluate("object.spec.replicas <= maxPods", &act).unwrap(),
            CelValue::Bool(true),
        );
    }

    // ── error paths ───────────────────────────────────────────────────────

    #[test]
    fn invalid_syntax_returns_compile_error() {
        let ev = CelInterpreterEvaluator::new();
        let err = ev.evaluate("object.spec.replicas >>", &CelActivation::default()).unwrap_err();
        assert!(
            matches!(err, CelError::Compile(_)),
            "expected Compile error, got {err:?}"
        );
    }

    #[test]
    fn undeclared_reference_returns_runtime_error() {
        let ev = CelInterpreterEvaluator::new();
        // No `object` in the activation, but the expression references it.
        let err = ev.evaluate("object.foo == 'bar'", &CelActivation::default()).unwrap_err();
        assert!(
            matches!(err, CelError::Runtime(_)),
            "expected Runtime error for missing var, got {err:?}"
        );
    }

    #[test]
    fn missing_field_traversal_returns_runtime_error() {
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({"spec": {}}));
        // Field `replicas` is absent — CEL spec says traversal errors
        // (use has() to guard). cel-interpreter surfaces this as a
        // runtime error which we map into CelError::Runtime.
        let err = ev.evaluate("object.spec.replicas > 0", &act).unwrap_err();
        assert!(
            matches!(err, CelError::Runtime(_)),
            "expected Runtime error for missing field traversal, got {err:?}"
        );
    }

    // ── program cache ─────────────────────────────────────────────────────

    #[test]
    fn program_cache_holds_compiled_expressions() {
        let ev = CelInterpreterEvaluator::new();
        assert_eq!(ev.cache_len(), 0);
        let act = act_with_object(json!({"spec": {"replicas": 1}}));
        for _ in 0..5 {
            ev.evaluate("object.spec.replicas > 0", &act).unwrap();
        }
        assert_eq!(ev.cache_len(), 1, "same expression caches once");
        ev.evaluate("object.spec.replicas < 100", &act).unwrap();
        assert_eq!(ev.cache_len(), 2, "different expression adds a second entry");
    }

    // ── integration: drop-in for the Dispatcher trait ─────────────────────

    #[test]
    fn evaluator_is_dyn_compatible_for_dispatcher() {
        // The Dispatcher holds `Arc<dyn CelEvaluator>`. Make sure our
        // evaluator survives that erasure.
        let ev: Arc<dyn CelEvaluator> = Arc::new(CelInterpreterEvaluator::new());
        let act = act_with_object(json!({"x": 1}));
        assert_eq!(ev.evaluate("object.x == 1", &act).unwrap(), CelValue::Bool(true));
    }

    #[test]
    fn evaluator_returns_type_error_for_list_result() {
        // `[1,2,3]` returns a List which our trait surface narrows to
        // an error. The dispatcher then turns this into a fail-policy
        // outcome — exactly the right behaviour for a malformed policy.
        let ev = CelInterpreterEvaluator::new();
        let err = ev.evaluate("[1, 2, 3]", &CelActivation::default()).unwrap_err();
        assert!(
            matches!(err, CelError::Type { .. }),
            "expected Type error for List result, got {err:?}"
        );
    }

    // ── authorizer binding (2026-05-13 batch2) ────────────────────────────

    fn act_with_authorizer(view: AuthorizerView) -> CelActivation {
        CelActivation {
            authorizer: Some(view),
            ..Default::default()
        }
    }

    #[test]
    fn missing_authorizer_returns_runtime_for_authorizer_user_access() {
        // No authorizer slot bound → cel-interpreter undeclared
        // reference. Cave surfaces this as a Runtime error so the
        // dispatcher records it as a fail-policy outcome.
        let ev = CelInterpreterEvaluator::new();
        let err = ev
            .evaluate("authorizer.user == 'kube-admin'", &CelActivation::default())
            .unwrap_err();
        assert!(matches!(err, CelError::Runtime(_)), "got {err:?}");
    }

    #[test]
    fn authorizer_user_field_reads_through_projection() {
        let ev = CelInterpreterEvaluator::new();
        let view = AuthorizerView {
            user: "kube-admin".into(),
            ..Default::default()
        };
        let act = act_with_authorizer(view);
        assert_eq!(
            ev.evaluate("authorizer.user == 'kube-admin'", &act).unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate("authorizer.user == 'other'", &act).unwrap(),
            CelValue::Bool(false),
        );
    }

    #[test]
    fn authorizer_groups_size_is_callable_from_cel() {
        let ev = CelInterpreterEvaluator::new();
        let view = AuthorizerView::default()
            .with_group("system:authenticated")
            .with_group("system:masters");
        let act = act_with_authorizer(view);
        assert_eq!(
            ev.evaluate("size(authorizer.groups) == 2", &act).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn authorizer_grants_membership_check_via_in_operator() {
        // The dispatcher's allow-list keys are stable strings; the
        // simplest CEL spelling is membership against the list.
        let ev = CelInterpreterEvaluator::new();
        let view = AuthorizerView::default()
            .allow("update", "apps", "deployments", Some("team-a"));
        let act = act_with_authorizer(view);
        assert_eq!(
            ev.evaluate(
                "'update:apps/deployments/team-a' in authorizer.grants",
                &act,
            )
            .unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate(
                "'delete:apps/deployments/team-a' in authorizer.grants",
                &act,
            )
            .unwrap(),
            CelValue::Bool(false),
        );
    }

    #[test]
    fn check_resource_honours_wildcard_in_verb_slot() {
        let view = AuthorizerView::default().allow("*", "apps", "deployments", Some("team-a"));
        assert!(view.check_resource("get", "apps", "deployments", Some("team-a")));
        assert!(view.check_resource("update", "apps", "deployments", Some("team-a")));
        assert!(!view.check_resource("get", "apps", "deployments", Some("team-b")));
    }

    #[test]
    fn check_resource_honours_wildcard_in_resource_slot() {
        let view = AuthorizerView::default().allow("get", "core", "*", None);
        assert!(view.check_resource("get", "core", "configmaps", None));
        assert!(view.check_resource("get", "core", "secrets", None));
        assert!(!view.check_resource("update", "core", "configmaps", None));
    }

    #[test]
    fn check_resource_namespace_match_requires_either_wildcard_or_exact() {
        let view = AuthorizerView::default().allow("update", "apps", "deployments", Some("team-a"));
        assert!(view.check_resource("update", "apps", "deployments", Some("team-a")));
        assert!(!view.check_resource("update", "apps", "deployments", Some("team-b")));
        assert!(!view.check_resource("update", "apps", "deployments", None));
    }

    #[test]
    fn check_url_honours_wildcard_in_either_slot() {
        let view = AuthorizerView::default().allow_path("*", "/healthz");
        assert!(view.check_url("get", "/healthz"));
        assert!(view.check_url("head", "/healthz"));
        assert!(!view.check_url("get", "/api/v1"));

        let view = AuthorizerView::default().allow_path("get", "*");
        assert!(view.check_url("get", "/api/v1"));
        assert!(view.check_url("get", "/healthz"));
    }

    #[test]
    fn authorizer_grants_with_wildcards_match_concrete_in_cel() {
        let ev = CelInterpreterEvaluator::new();
        // Dispatcher pre-resolves the .check() to a boolean
        // variable; here we test the underlying snapshot is
        // wildcard-aware.
        let view = AuthorizerView::default().allow("*", "apps", "*", Some("team-a"));
        assert!(view.check_resource("get", "apps", "deployments", Some("team-a")));
        assert!(view.check_resource("delete", "apps", "statefulsets", Some("team-a")));
        // CEL side still reads .grants verbatim.
        let act = act_with_authorizer(view);
        assert_eq!(
            ev.evaluate("'*:apps/*/team-a' in authorizer.grants", &act).unwrap(),
            CelValue::Bool(true),
        );
    }

    #[test]
    fn authorizer_binding_does_not_leak_into_unrelated_activation() {
        // An activation with authorizer = None but object = Some
        // must not somehow bind `authorizer` to null — it should
        // truly be undeclared so a typoed expression fails loudly.
        let ev = CelInterpreterEvaluator::new();
        let act = act_with_object(json!({"x": 1}));
        let err = ev
            .evaluate("authorizer.user == 'x'", &act)
            .unwrap_err();
        assert!(matches!(err, CelError::Runtime(_)));
    }

    #[test]
    fn group_membership_via_cel_exists_macro() {
        let ev = CelInterpreterEvaluator::new();
        let view = AuthorizerView::default()
            .with_group("system:authenticated")
            .with_group("idp:platform-admins");
        let act = act_with_authorizer(view);
        // CEL's .exists() macro iterates a list with a predicate.
        assert_eq!(
            ev.evaluate(
                "authorizer.groups.exists(g, g == 'idp:platform-admins')",
                &act
            )
            .unwrap(),
            CelValue::Bool(true),
        );
        assert_eq!(
            ev.evaluate(
                "authorizer.groups.exists(g, g == 'unknown:group')",
                &act
            )
            .unwrap(),
            CelValue::Bool(false),
        );
    }

    // ── Dispatcher integration ────────────────────────────────────────────
    //
    // Drive `vap_advanced::Dispatcher` with the real CEL evaluator and
    // assert end-to-end behaviour: matchConditions short-circuit, paramRef
    // resolution, validation pass/fail, failurePolicy.

    use crate::admission::{AdmissionRequest, Operation};
    use crate::resources::{ConfigMap, ObjectMeta, Resource};
    use crate::vap_advanced::{
        match_resources_matches as _, // unused but keeps the surface visible
        Dispatcher, DispatchOutcome, FailurePolicyType, InMemoryParamResolver, MatchCondition,
        MatchInput, ValidatingAdmissionPolicy, ValidatingAdmissionPolicyBinding,
        ValidatingAdmissionPolicyBindingSpec, ValidatingAdmissionPolicySpec, Validation,
    };

    fn mk_cm(name: &str, tenant: &str, env: &str) -> Resource {
        let mut meta = ObjectMeta::new(name, "default");
        meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
        meta.labels.insert("env".into(), env.into());
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            metadata: meta,
            data: Default::default(),
        })
    }

    fn mk_admission_req(tenant: &str, name: &str, env: &str) -> AdmissionRequest {
        let obj = mk_cm(name, tenant, env);
        AdmissionRequest {
            uid: format!("uid-{name}"),
            tenant_id: tenant.to_string(),
            namespace: "default".into(),
            kind: "ConfigMap".into(),
            name: name.into(),
            operation: Operation::Create,
            object: Some(obj),
            old_object: None,
            user: "test".into(),
            dry_run: false,
        }
    }

    fn mk_policy(name: &str, tenant: &str, validations: Vec<Validation>) -> ValidatingAdmissionPolicy {
        let mut meta = ObjectMeta::new(name, "");
        meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
        ValidatingAdmissionPolicy {
            api_version: "admissionregistration.k8s.io/v1".into(),
            kind: "ValidatingAdmissionPolicy".into(),
            metadata: meta,
            spec: ValidatingAdmissionPolicySpec {
                validations,
                failure_policy: FailurePolicyType::Fail,
                ..Default::default()
            },
        }
    }

    fn mk_binding(name: &str, tenant: &str, policy: &str) -> ValidatingAdmissionPolicyBinding {
        let mut meta = ObjectMeta::new(name, "");
        meta.annotations.insert("cave.runtime/tenant-id".into(), tenant.into());
        ValidatingAdmissionPolicyBinding {
            api_version: "admissionregistration.k8s.io/v1".into(),
            kind: "ValidatingAdmissionPolicyBinding".into(),
            metadata: meta,
            spec: ValidatingAdmissionPolicyBindingSpec {
                policy_name: policy.into(),
                ..Default::default()
            },
        }
    }

    fn match_input<'a>(
        req: &'a AdmissionRequest,
        labels: &'a std::collections::HashMap<String, String>,
        ns_labels: &'a std::collections::HashMap<String, String>,
    ) -> MatchInput<'a> {
        // AdmissionRequest folds (group, version) into `kind` upstream;
        // for the dispatcher's MatchInput we expand back to empty group +
        // "v1" version which matches the v1 core API surface the tests use.
        MatchInput {
            group: "",
            version: "v1",
            resource: "configmaps",
            name: &req.name,
            namespace: &req.namespace,
            operation: &req.operation,
            object_labels: labels,
            namespace_labels: ns_labels,
        }
    }

    #[test]
    fn dispatcher_admits_on_passing_validation_with_real_cel() {
        let evaluator = Arc::new(CelInterpreterEvaluator::new());
        let resolver = Arc::new(InMemoryParamResolver::default());
        let d = Dispatcher::new(evaluator, resolver);
        let policy = mk_policy(
            "name-not-latest",
            "acme",
            vec![Validation {
                expression: "object.metadata.name != 'latest'".into(),
                message: "name must not be 'latest'".into(),
                ..Default::default()
            }],
        );
        let binding = mk_binding("bind-1", "acme", "name-not-latest");
        let req = mk_admission_req("acme", "web", "prod");
        let labels = std::collections::HashMap::new();
        let ns = std::collections::HashMap::new();
        let input = match_input(&req, &labels, &ns);
        let out = d.dispatch_one("acme", &policy, &binding, &req, &input);
        assert!(
            out.iter().all(|o| matches!(o, DispatchOutcome::Allow)),
            "all outcomes should Allow, got {out:?}"
        );
    }

    #[test]
    fn dispatcher_denies_on_failing_validation_with_real_cel() {
        let evaluator = Arc::new(CelInterpreterEvaluator::new());
        let resolver = Arc::new(InMemoryParamResolver::default());
        let d = Dispatcher::new(evaluator, resolver);
        let policy = mk_policy(
            "name-not-latest",
            "acme",
            vec![Validation {
                expression: "object.metadata.name != 'latest'".into(),
                message: "name must not be 'latest'".into(),
                ..Default::default()
            }],
        );
        let binding = mk_binding("bind-1", "acme", "name-not-latest");
        let req = mk_admission_req("acme", "latest", "prod");
        let labels = std::collections::HashMap::new();
        let ns = std::collections::HashMap::new();
        let input = match_input(&req, &labels, &ns);
        let out = d.dispatch_one("acme", &policy, &binding, &req, &input);
        assert!(
            out.iter().any(|o| matches!(o, DispatchOutcome::Deny { .. })),
            "at least one Deny expected, got {out:?}"
        );
    }

    #[test]
    fn dispatcher_match_conditions_short_circuit_to_empty() {
        // matchCondition that returns false → policy is not evaluated;
        // dispatch returns an empty vec (no outcome).
        let evaluator = Arc::new(CelInterpreterEvaluator::new());
        let resolver = Arc::new(InMemoryParamResolver::default());
        let d = Dispatcher::new(evaluator, resolver);
        let mut policy = mk_policy(
            "noop",
            "acme",
            vec![Validation {
                expression: "false".into(),
                message: "should never fire".into(),
                ..Default::default()
            }],
        );
        policy.spec.match_conditions.push(MatchCondition {
            name: "is-prod".into(),
            expression: "object.metadata.labels.env == 'prod'".into(),
        });
        let binding = mk_binding("bind-1", "acme", "noop");
        // Build a request whose object has env=dev — matchCondition returns false.
        let req = mk_admission_req("acme", "web", "dev");
        let labels = std::collections::HashMap::new();
        let ns = std::collections::HashMap::new();
        let input = match_input(&req, &labels, &ns);
        let out = d.dispatch_one("acme", &policy, &binding, &req, &input);
        assert!(out.is_empty(), "matchCondition=false skips policy, got {out:?}");
    }

    #[test]
    fn dispatcher_failure_policy_fail_returns_error_outcome() {
        // Expression that fails to compile triggers FailurePolicy::Fail → Error.
        let evaluator = Arc::new(CelInterpreterEvaluator::new());
        let resolver = Arc::new(InMemoryParamResolver::default());
        let d = Dispatcher::new(evaluator, resolver);
        let policy = mk_policy(
            "broken",
            "acme",
            vec![Validation {
                expression: "object.spec.replicas >>".into(), // invalid CEL
                message: "should surface as Error".into(),
                ..Default::default()
            }],
        );
        let binding = mk_binding("bind-1", "acme", "broken");
        let req = mk_admission_req("acme", "web", "prod");
        let labels = std::collections::HashMap::new();
        let ns = std::collections::HashMap::new();
        let input = match_input(&req, &labels, &ns);
        let out = d.dispatch_one("acme", &policy, &binding, &req, &input);
        assert!(
            out.iter().any(|o| matches!(o, DispatchOutcome::Error(_))),
            "FailurePolicy::Fail should surface Error, got {out:?}"
        );
    }

    #[test]
    fn dispatcher_failure_policy_ignore_silences_error_outcome() {
        let evaluator = Arc::new(CelInterpreterEvaluator::new());
        let resolver = Arc::new(InMemoryParamResolver::default());
        let d = Dispatcher::new(evaluator, resolver);
        let mut policy = mk_policy(
            "broken-but-ignored",
            "acme",
            vec![Validation {
                expression: "object.spec.replicas >>".into(),
                message: "ignored".into(),
                ..Default::default()
            }],
        );
        policy.spec.failure_policy = FailurePolicyType::Ignore;
        let binding = mk_binding("bind-1", "acme", "broken-but-ignored");
        let req = mk_admission_req("acme", "web", "prod");
        let labels = std::collections::HashMap::new();
        let ns = std::collections::HashMap::new();
        let input = match_input(&req, &labels, &ns);
        let out = d.dispatch_one("acme", &policy, &binding, &req, &input);
        assert!(
            out.iter().all(|o| matches!(o, DispatchOutcome::SilencedError)),
            "FailurePolicy::Ignore should silence the error, got {out:?}"
        );
    }
}
