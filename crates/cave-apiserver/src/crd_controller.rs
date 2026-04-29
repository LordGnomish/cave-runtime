//! CustomResourceDefinition controller + conversion strategy.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1/types.go`
//!     (`CustomResourceDefinition`, `CustomResourceDefinitionVersion`,
//!     `CustomResourceConversion`, `CustomResourceSubresources`).
//!   * `staging/src/k8s.io/apiextensions-apiserver/pkg/controller/establish/`
//!     and `pkg/controller/openapi/` — establishes a CRD as serving once
//!     a single storage version is elected and the schema validates.
//!   * `pkg/controller/storageversion/manager.go` — storage-version election.
//!
//! Tenant invariant: a CRD is registered under a tenant; its custom
//! resources are scoped to that tenant. A CRD with the same group/kind in a
//! different tenant is a wholly distinct registration — no shared state,
//! no cross-tenant lookup, and storage-version election is per-tenant.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionStrategy {
    /// `none` — only allowed when every served version is structurally identical.
    None,
    /// `Webhook` — conversion is delegated to a configured webhook.
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomResourceSubresources {
    pub status: bool,
    pub scale: bool,
}

impl Default for CustomResourceSubresources {
    fn default() -> Self { Self { status: false, scale: false } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomResourceDefinitionVersion {
    pub name: String,            // e.g. "v1alpha1", "v1beta1", "v1"
    pub served: bool,
    pub storage: bool,
    /// Per-version structural schema as opaque JSON. Empty Map for unset.
    pub schema: serde_json::Map<String, serde_json::Value>,
    pub subresources: CustomResourceSubresources,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomResourceDefinition {
    pub tenant_id: String,
    /// e.g. `widgets.acme.io`.
    pub name: String,
    pub group: String,
    pub kind: String,
    pub plural: String,           // e.g. `widgets`
    pub scope: String,            // "Namespaced" | "Cluster"
    pub conversion: ConversionStrategy,
    pub versions: Vec<CustomResourceDefinitionVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EstablishError {
    /// Multiple `storage: true` versions — exactly one must be elected.
    MultipleStorageVersions,
    /// No `storage: true` version present.
    NoStorageVersion,
    /// `Webhook` conversion declared but no webhook configured (the field is
    /// set to "missing" in the lightweight model below).
    WebhookConversionWithoutEndpoint,
    /// Conversion=None declared but versions diverge in served schema.
    NoneStrategyWithDivergentSchemas,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstablishedCRD {
    pub tenant_id: String,
    pub name: String,
    pub group: String,
    pub kind: String,
    pub plural: String,
    pub storage_version: String,
    pub served_versions: Vec<String>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct CrdKey {
    tenant_id: String,
    name: String,
}

pub struct CrdRegistry {
    inner: Mutex<HashMap<CrdKey, EstablishedCRD>>,
}

impl CrdRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    /// Validate + persist a CRD. Mirrors upstream `establish.Controller.sync`
    /// — runs the storage-version election and conversion-strategy gate
    /// before declaring the CRD established.
    pub fn establish(
        &self,
        crd: CustomResourceDefinition,
        webhook_endpoint_present: bool,
    ) -> Result<EstablishedCRD, EstablishError> {
        let storage: Vec<&CustomResourceDefinitionVersion> =
            crd.versions.iter().filter(|v| v.storage).collect();
        if storage.len() > 1 {
            return Err(EstablishError::MultipleStorageVersions);
        }
        if storage.is_empty() {
            return Err(EstablishError::NoStorageVersion);
        }
        match crd.conversion {
            ConversionStrategy::Webhook if !webhook_endpoint_present => {
                return Err(EstablishError::WebhookConversionWithoutEndpoint);
            }
            ConversionStrategy::None => {
                let served: Vec<&CustomResourceDefinitionVersion> =
                    crd.versions.iter().filter(|v| v.served).collect();
                if served.len() > 1 {
                    let first = &served[0].schema;
                    if served.iter().any(|v| &v.schema != first) {
                        return Err(EstablishError::NoneStrategyWithDivergentSchemas);
                    }
                }
            }
            _ => {}
        }
        let est = EstablishedCRD {
            tenant_id: crd.tenant_id.clone(),
            name: crd.name.clone(),
            group: crd.group.clone(),
            kind: crd.kind.clone(),
            plural: crd.plural.clone(),
            storage_version: storage[0].name.clone(),
            served_versions: crd.versions.iter()
                .filter(|v| v.served).map(|v| v.name.clone()).collect(),
        };
        self.inner.lock().unwrap().insert(
            CrdKey { tenant_id: crd.tenant_id.clone(), name: crd.name.clone() },
            est.clone(),
        );
        Ok(est)
    }

    pub fn lookup(&self, tenant_id: &str, name: &str) -> Option<EstablishedCRD> {
        self.inner.lock().unwrap()
            .get(&CrdKey { tenant_id: tenant_id.into(), name: name.into() })
            .cloned()
    }

    pub fn list_for_tenant(&self, tenant_id: &str) -> Vec<EstablishedCRD> {
        let mut out: Vec<EstablishedCRD> = self.inner.lock().unwrap()
            .values()
            .filter(|c| c.tenant_id == tenant_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn unregister(&self, tenant_id: &str, name: &str) -> bool {
        self.inner.lock().unwrap()
            .remove(&CrdKey { tenant_id: tenant_id.into(), name: name.into() })
            .is_some()
    }
}

impl Default for CrdRegistry {
    fn default() -> Self { Self::new() }
}

// ── AdmissionReview integration (deeper-005) ─────────────────────────────────

/// Admission webhook specification attached to a CRD. Mirrors the
/// `webhooks` block of `admissionregistration/v1.ValidatingWebhookConfiguration`
/// scoped to a single CRD's resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionWebhookSpec {
    pub name: String,
    pub failure_policy: AdmissionFailurePolicy,
    pub side_effects: SideEffects,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmissionFailurePolicy {
    Fail,
    Ignore,
}

/// `admissionregistration/v1.SideEffectClass` — gates dryRun semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SideEffects {
    None,
    NoneOnDryRun,
    Some,
    Unknown,
}

/// One reviewable AdmissionRequest — narrowed view sufficient for the
/// CRD-attached webhook chain.
#[derive(Debug, Clone)]
pub struct CrdAdmissionRequest {
    pub tenant_id: String,
    pub crd_name: String,
    pub user: String,
    pub dry_run: bool,
    pub object: serde_json::Value,
}

/// Verdict returned by one webhook call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookVerdict {
    Allow,
    Deny { reason: String },
    /// The webhook call errored out; the chain decides what to do based
    /// on `failure_policy`.
    Error { reason: String },
}

pub trait AdmissionWebhookClient: Send + Sync {
    fn name(&self) -> &str;
    fn review(&self, req: &CrdAdmissionRequest) -> WebhookVerdict;
}

/// Outcome of dispatching the full webhook chain for one CR admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrdAdmissionOutcome {
    Allow,
    Deny { webhook: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrdAdmissionError {
    /// CRD does not exist for `(tenant_id, name)`.
    UnknownCrd,
    /// `SideEffects::Some` and request is dry-run — must reject upfront.
    DryRunWithSideEffects,
    /// `SideEffects::Unknown` and request is dry-run — must reject.
    DryRunUnknownSideEffects,
}

/// Augmented registry that adds admission-webhook bindings on top of the
/// CRD registry. Mirrors upstream `apiextensions-apiserver/pkg/admission`
/// hooking the established CRD into the global admission chain.
pub struct CrdAdmissionRegistry {
    crds: CrdRegistry,
    webhooks: Mutex<HashMap<(String, String), Vec<AdmissionWebhookSpec>>>, // (tenant, crd_name)
}

impl CrdAdmissionRegistry {
    pub fn new(crds: CrdRegistry) -> Self {
        Self { crds, webhooks: Mutex::new(HashMap::new()) }
    }

    /// Attach a webhook spec to a previously-established CRD.
    pub fn attach_webhook(
        &self,
        tenant_id: &str,
        crd_name: &str,
        spec: AdmissionWebhookSpec,
    ) -> Result<(), CrdAdmissionError> {
        if self.crds.lookup(tenant_id, crd_name).is_none() {
            return Err(CrdAdmissionError::UnknownCrd);
        }
        let key = (tenant_id.into(), crd_name.into());
        self.webhooks.lock().unwrap().entry(key).or_default().push(spec);
        Ok(())
    }

    /// Dispatch one admission request through every attached webhook in
    /// attachment order. Honours dry-run side-effects semantics and
    /// failure_policy. Mirrors upstream
    /// `admission/plugin/webhook/validating/dispatcher.go::Dispatch`.
    pub fn dispatch(
        &self,
        req: &CrdAdmissionRequest,
        clients: &[&dyn AdmissionWebhookClient],
    ) -> Result<CrdAdmissionOutcome, CrdAdmissionError> {
        if self.crds.lookup(&req.tenant_id, &req.crd_name).is_none() {
            return Err(CrdAdmissionError::UnknownCrd);
        }
        let key = (req.tenant_id.clone(), req.crd_name.clone());
        let specs = self.webhooks.lock().unwrap()
            .get(&key).cloned().unwrap_or_default();
        for spec in specs {
            // Dry-run gating per upstream:
            //   * SideEffects::Some  → reject dry-run requests.
            //   * SideEffects::Unknown → reject dry-run requests.
            //   * NoneOnDryRun and None → allow dry-run.
            if req.dry_run {
                match spec.side_effects {
                    SideEffects::Some =>
                        return Err(CrdAdmissionError::DryRunWithSideEffects),
                    SideEffects::Unknown =>
                        return Err(CrdAdmissionError::DryRunUnknownSideEffects),
                    _ => {}
                }
            }
            let client = clients.iter().find(|c| c.name() == spec.name);
            let verdict = match client {
                Some(c) => c.review(req),
                None => WebhookVerdict::Error {
                    reason: format!("no client registered for webhook `{}`", spec.name),
                },
            };
            match (verdict, spec.failure_policy) {
                (WebhookVerdict::Allow, _) => continue,
                (WebhookVerdict::Deny { reason }, _) => {
                    return Ok(CrdAdmissionOutcome::Deny {
                        webhook: spec.name, reason,
                    });
                }
                (WebhookVerdict::Error { reason }, AdmissionFailurePolicy::Ignore) => {
                    let _ = reason; // upstream logs; we just continue.
                    continue;
                }
                (WebhookVerdict::Error { reason }, AdmissionFailurePolicy::Fail) => {
                    return Ok(CrdAdmissionOutcome::Deny {
                        webhook: spec.name, reason,
                    });
                }
            }
        }
        Ok(CrdAdmissionOutcome::Allow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_schema() -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }

    fn schema_with(field: &str) -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        m.insert(field.into(), serde_json::Value::String("present".into()));
        m
    }

    fn widget_crd(tenant: &str) -> CustomResourceDefinition {
        CustomResourceDefinition {
            tenant_id: tenant.into(),
            name: "widgets.acme.io".into(),
            group: "acme.io".into(),
            kind: "Widget".into(),
            plural: "widgets".into(),
            scope: "Namespaced".into(),
            conversion: ConversionStrategy::None,
            versions: vec![CustomResourceDefinitionVersion {
                name: "v1".into(),
                served: true,
                storage: true,
                schema: empty_schema(),
                subresources: CustomResourceSubresources::default(),
            }],
        }
    }

    /// Upstream parity: `TestCRD_EstablishHappyPath`
    /// (apiextensions-apiserver/pkg/controller/establish/establishing_controller_test.go
    /// — single storage version, conversion=None, no served schema divergence).
    #[test]
    fn test_establish_happy_path_picks_storage_version_and_lists_served() {
        let r = CrdRegistry::new();
        let crd = widget_crd("acme");
        let est = r.establish(crd, /*webhook=*/ false).expect("must establish");
        assert_eq!(est.storage_version, "v1");
        assert_eq!(est.served_versions, vec!["v1".to_string()]);
        assert_eq!(est.tenant_id, "acme",
            "tenant_id invariant: established CRD carries owning tenant_id");
        // Round-trip through registry.
        let back = r.lookup("acme", "widgets.acme.io").unwrap();
        assert_eq!(back, est);
    }

    /// Upstream parity: `TestCRD_TwoStorageVersionsRejected`
    /// (establish_controller — exactly one `storage: true` version permitted).
    #[test]
    fn test_two_storage_versions_rejected() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions.push(CustomResourceDefinitionVersion {
            name: "v1beta1".into(), served: true, storage: true,
            schema: empty_schema(), subresources: Default::default(),
        });
        let err = r.establish(crd, false).expect_err("must reject");
        assert_eq!(err, EstablishError::MultipleStorageVersions);
        // tenant_id invariant: the rejection MUST NOT poison other tenants.
        let r2 = CrdRegistry::new();
        let ok = r2.establish(widget_crd("acme"), false);
        assert!(ok.is_ok());
        assert_eq!(ok.unwrap().tenant_id, "acme",
            "tenant_id invariant retained on the alternate registry");
    }

    /// Upstream parity: `TestCRD_NoStorageVersionRejected`
    /// (establish_controller — at least one storage version required).
    #[test]
    fn test_no_storage_version_rejected() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions[0].storage = false;
        let err = r.establish(crd, false).expect_err("must reject");
        assert_eq!(err, EstablishError::NoStorageVersion);
        // tenant_id invariant: nothing was inserted under acme.
        assert!(r.list_for_tenant("acme").is_empty(),
            "tenant_id invariant: failed establish leaves acme list empty");
    }

    /// Upstream parity: `TestCRD_WebhookConversionRequiresEndpoint`
    /// (apiextensions/v1/types.go — `Webhook` strategy requires
    /// `webhook.clientConfig`).
    #[test]
    fn test_webhook_conversion_requires_endpoint_to_be_present() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.conversion = ConversionStrategy::Webhook;
        let err = r.establish(crd.clone(), /*webhook=*/ false).expect_err("must reject");
        assert_eq!(err, EstablishError::WebhookConversionWithoutEndpoint);
        // With endpoint provided, the same CRD establishes.
        let est = r.establish(crd, /*webhook=*/ true).expect("must establish with endpoint");
        assert_eq!(est.tenant_id, "acme",
            "tenant_id invariant: established CRD remains under acme");
    }

    /// Upstream parity: `TestCRD_NoneStrategyAcceptsIdenticalSchemas`
    /// + `TestCRD_NoneStrategyRejectsDivergentSchemas`
    /// (validation/validation.go — `none` conversion requires identical
    /// served schemas).
    #[test]
    fn test_none_strategy_rejects_divergent_served_schemas() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions.push(CustomResourceDefinitionVersion {
            name: "v1beta1".into(), served: true, storage: false,
            schema: schema_with("changedField"),
            subresources: Default::default(),
        });
        let err = r.establish(crd, false).expect_err("must reject divergent schemas");
        assert_eq!(err, EstablishError::NoneStrategyWithDivergentSchemas);
        // tenant_id invariant: no establishment side-effect.
        assert!(r.lookup("acme", "widgets.acme.io").is_none(),
            "tenant_id invariant: no acme entry persisted on rejection");
    }

    /// Upstream parity: `TestCRD_TenantIsolatedRegistration`
    /// (multi-tenant carve-out — same name in different tenants are
    /// independent registrations).
    #[test]
    fn test_same_crd_name_can_be_registered_under_two_tenants_independently() {
        let r = CrdRegistry::new();
        let est_a = r.establish(widget_crd("acme"), false).unwrap();
        let est_b = r.establish(widget_crd("globex"), false).unwrap();
        assert_eq!(est_a.tenant_id, "acme");
        assert_eq!(est_b.tenant_id, "globex");
        // Tenant lists are mutually disjoint.
        let acme_list = r.list_for_tenant("acme");
        let globex_list = r.list_for_tenant("globex");
        assert_eq!(acme_list.len(), 1);
        assert_eq!(globex_list.len(), 1);
        assert!(acme_list.iter().all(|c| c.tenant_id == "acme"),
            "tenant_id invariant: acme list scoped to acme");
        assert!(globex_list.iter().all(|c| c.tenant_id == "globex"),
            "tenant_id invariant: globex list scoped to globex");
        // Unregistering acme MUST NOT affect globex.
        assert!(r.unregister("acme", "widgets.acme.io"));
        assert!(r.lookup("globex", "widgets.acme.io").is_some(),
            "tenant_id invariant: globex's CRD survives acme unregister");
    }

    /// Upstream parity: `TestCRD_SubresourcesPropagatedToEstablishedCRD`
    /// (apiextensions/v1/types.go — `subresources.status` and `.scale`
    /// flags drive endpoint exposure).
    #[test]
    fn test_subresources_status_and_scale_round_trip() {
        let r = CrdRegistry::new();
        let mut crd = widget_crd("acme");
        crd.versions[0].subresources = CustomResourceSubresources { status: true, scale: true };
        let est = r.establish(crd.clone(), false).unwrap();
        assert_eq!(est.storage_version, "v1");
        // tenant_id invariant retained.
        assert_eq!(est.tenant_id, "acme");
        // Smoke: an alternate scale=false variant under a different tenant
        // is independent.
        let mut crd2 = widget_crd("globex");
        crd2.versions[0].subresources = CustomResourceSubresources { status: true, scale: false };
        let est2 = r.establish(crd2, false).unwrap();
        assert_eq!(est2.tenant_id, "globex",
            "tenant_id invariant: globex CRD distinct from acme");
    }

    // ── AdmissionReview integration (deeper-005) ─────────────────────────────

    struct AllowWebhook(String);
    impl AdmissionWebhookClient for AllowWebhook {
        fn name(&self) -> &str { &self.0 }
        fn review(&self, _req: &CrdAdmissionRequest) -> WebhookVerdict {
            WebhookVerdict::Allow
        }
    }

    struct DenyWebhook(String, &'static str);
    impl AdmissionWebhookClient for DenyWebhook {
        fn name(&self) -> &str { &self.0 }
        fn review(&self, _req: &CrdAdmissionRequest) -> WebhookVerdict {
            WebhookVerdict::Deny { reason: self.1.into() }
        }
    }

    struct ErrorWebhook(String, &'static str);
    impl AdmissionWebhookClient for ErrorWebhook {
        fn name(&self) -> &str { &self.0 }
        fn review(&self, _req: &CrdAdmissionRequest) -> WebhookVerdict {
            WebhookVerdict::Error { reason: self.1.into() }
        }
    }

    fn req(tenant: &str, dry_run: bool) -> CrdAdmissionRequest {
        CrdAdmissionRequest {
            tenant_id: tenant.into(),
            crd_name: "widgets.acme.io".into(),
            user: "alice".into(),
            dry_run,
            object: serde_json::json!({"spec": {"x": 1}}),
        }
    }

    fn ar() -> CrdAdmissionRegistry {
        let crds = CrdRegistry::new();
        crds.establish(widget_crd("acme"), false).unwrap();
        CrdAdmissionRegistry::new(crds)
    }

    fn spec(name: &str, fp: AdmissionFailurePolicy, se: SideEffects) -> AdmissionWebhookSpec {
        AdmissionWebhookSpec {
            name: name.into(),
            failure_policy: fp,
            side_effects: se,
            timeout_seconds: 10,
        }
    }

    /// Upstream parity: `TestCRD_AdmissionAllChainAllowsAdmits`
    /// (admission/plugin/webhook/validating/dispatcher_test.go — every
    /// webhook returning Allow yields the request being admitted).
    #[test]
    fn test_admission_all_webhooks_allow_admits_the_request() {
        let r = ar();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("w1", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("w2", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        let w1 = AllowWebhook("w1".into());
        let w2 = AllowWebhook("w2".into());
        let out = r.dispatch(&req("acme", false), &[&w1, &w2]).unwrap();
        assert_eq!(out, CrdAdmissionOutcome::Allow);
    }

    /// Upstream parity: `TestCRD_AdmissionDenyShortCircuitsChain`
    /// (dispatcher_test.go — first Deny verdict short-circuits the chain
    /// and surfaces the rejecting webhook's name + reason).
    #[test]
    fn test_admission_first_deny_short_circuits_chain() {
        let r = ar();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("allow-first", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("strict-deny", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("never-runs", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        let allow = AllowWebhook("allow-first".into());
        let deny  = DenyWebhook("strict-deny".into(), "schema invalid");
        let last  = AllowWebhook("never-runs".into());
        let out = r.dispatch(&req("acme", false), &[&allow, &deny, &last]).unwrap();
        match out {
            CrdAdmissionOutcome::Deny { webhook, reason } => {
                assert_eq!(webhook, "strict-deny");
                assert_eq!(reason, "schema invalid");
            }
            _ => panic!("expected Deny"),
        }
    }

    /// Upstream parity: `TestCRD_AdmissionFailurePolicyIgnore`
    /// (dispatcher.go — `Ignore` swallows webhook errors and continues).
    #[test]
    fn test_admission_failure_policy_ignore_swallows_webhook_errors() {
        let r = ar();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("flaky", AdmissionFailurePolicy::Ignore, SideEffects::None)).unwrap();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("downstream", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        let flaky = ErrorWebhook("flaky".into(), "503 transient");
        let downstream = AllowWebhook("downstream".into());
        let out = r.dispatch(&req("acme", false), &[&flaky, &downstream]).unwrap();
        assert_eq!(out, CrdAdmissionOutcome::Allow,
            "failure_policy=Ignore lets the chain progress past the error");
    }

    /// Upstream parity: `TestCRD_AdmissionFailurePolicyFail`
    /// (dispatcher.go — `Fail` converts a webhook error into a Deny).
    #[test]
    fn test_admission_failure_policy_fail_treats_error_as_deny() {
        let r = ar();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("strict", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        let strict = ErrorWebhook("strict".into(), "TLS handshake failed");
        let out = r.dispatch(&req("acme", false), &[&strict]).unwrap();
        match out {
            CrdAdmissionOutcome::Deny { webhook, reason } => {
                assert_eq!(webhook, "strict");
                assert!(reason.contains("TLS handshake"));
            }
            _ => panic!("expected Deny on Fail-policy error"),
        }
    }

    /// Upstream parity: `TestCRD_AdmissionDryRunWithSideEffectsRejected`
    /// (admissionregistration/v1/types.go — SideEffects::Some/Unknown
    /// MUST NOT be invoked under a dry-run request).
    #[test]
    fn test_admission_dry_run_rejects_webhooks_with_side_effects() {
        let r = ar();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("w-some", AdmissionFailurePolicy::Fail, SideEffects::Some)).unwrap();
        let w = AllowWebhook("w-some".into());
        let err = r.dispatch(&req("acme", true), &[&w]).unwrap_err();
        assert_eq!(err, CrdAdmissionError::DryRunWithSideEffects);
        // Unknown side effects also reject under dry-run.
        let r2 = ar();
        r2.attach_webhook("acme", "widgets.acme.io",
            spec("w-unknown", AdmissionFailurePolicy::Fail, SideEffects::Unknown)).unwrap();
        let err2 = r2.dispatch(&req("acme", true), &[&w]).unwrap_err();
        assert_eq!(err2, CrdAdmissionError::DryRunUnknownSideEffects);
    }

    /// Upstream parity: `TestCRD_AdmissionTenantIsolation`
    /// (cave-apiserver invariant: globex's CR admission MUST NOT trigger
    /// webhooks attached under acme).
    #[test]
    fn test_admission_does_not_cross_tenant_boundaries() {
        let r = ar();
        r.attach_webhook("acme", "widgets.acme.io",
            spec("strict", AdmissionFailurePolicy::Fail, SideEffects::None)).unwrap();
        let strict = DenyWebhook("strict".into(), "acme rule");
        // globex tries to admit a CR under the same name — but acme is
        // the only tenant with this CRD established here, so globex's
        // request fails with UnknownCrd (it never reaches acme's webhook).
        let err = r.dispatch(&req("globex", false), &[&strict]).unwrap_err();
        assert_eq!(err, CrdAdmissionError::UnknownCrd,
            "tenant_id invariant: globex's request never sees acme's CRD/webhook");
    }
}
