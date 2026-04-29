//! Webhook admission — line-by-line parity port of upstream
//! `staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/`.
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/api/admissionregistration/v1/types.go`
//!     (`MutatingWebhook`, `ValidatingWebhook`, `ServiceReference`,
//!      `WebhookClientConfig`, `MatchCondition`, `ReinvocationPolicy`,
//!      `SideEffectClass`).
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/mutating/dispatcher.go`
//!     (`mutatingDispatcher.Dispatch` — reinvocation loop, timeoutSeconds,
//!      failure policy, side-effects gate).
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/validating/dispatcher.go`
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/predicates/predicates.go`
//!     (rule + namespace + object selectors).
//!
//! ## Compile-gate strategy
//!
//! Network IO (TLS dial) is gated behind a `WebhookClient` trait. Tests that
//! exercise the dispatcher state machine use `FakeWebhookClient`, an in-memory
//! responder. Tests that demand a real TLS handshake or CABundle verify path
//! are `#[ignore]`'d and reference upstream test names.
//!
//! ## Tenant invariant
//!
//! A webhook configuration owned by tenant T MUST NOT be invoked for an
//! AdmissionRequest from tenant ≠ T, even when its rules/selectors match.
//! Cluster-scoped admission configs are rejected by `validate_config_scope`.

use crate::admission::{
    AdmissionRequest, AdmissionResponse, JsonPatch, MutatingWebhook,
    Operation, ValidatingWebhook,
};
use crate::resources::ObjectMeta;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// API types — mirror admissionregistration.k8s.io/v1.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FailurePolicyType {
    Fail,
    Ignore,
}

impl Default for FailurePolicyType {
    fn default() -> Self { FailurePolicyType::Fail }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SideEffectClass {
    Unknown,
    None,
    Some,
    NoneOnDryRun,
}

impl Default for SideEffectClass {
    fn default() -> Self { SideEffectClass::Unknown }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ReinvocationPolicyType {
    Never,
    IfNeeded,
}

impl Default for ReinvocationPolicyType {
    fn default() -> Self { ReinvocationPolicyType::Never }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MatchPolicyType {
    Exact,
    Equivalent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ScopeType {
    All, Cluster, Namespaced,
}

impl Default for ScopeType {
    fn default() -> Self { ScopeType::All }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceReference {
    pub namespace: String,
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub port: Option<i32>, // default 443 if None
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebhookClientConfig {
    /// Direct URL (mutually exclusive with `service`). MUST be HTTPS.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub service: Option<ServiceReference>,
    /// PEM-encoded CA bundle. Empty bundle defers to the system trust store.
    #[serde(default)]
    pub ca_bundle: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleWithOperations {
    #[serde(default)]
    pub operations: Vec<String>,
    #[serde(default)]
    pub api_groups: Vec<String>,
    #[serde(default)]
    pub api_versions: Vec<String>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub scope: ScopeType,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelSelector {
    #[serde(default)]
    pub match_labels: HashMap<String, String>,
    #[serde(default)]
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: String,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchCondition {
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingWebhookSpec {
    pub name: String,
    pub client_config: WebhookClientConfig,
    #[serde(default)]
    pub rules: Vec<RuleWithOperations>,
    #[serde(default)]
    pub failure_policy: FailurePolicyType,
    #[serde(default)]
    pub match_policy: Option<MatchPolicyType>,
    #[serde(default)]
    pub namespace_selector: Option<LabelSelector>,
    #[serde(default)]
    pub object_selector: Option<LabelSelector>,
    #[serde(default)]
    pub side_effects: SideEffectClass,
    /// Default 10 in upstream. Maximum 30. We enforce both.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: i32,
    #[serde(default)]
    pub admission_review_versions: Vec<String>, // e.g. ["v1"]
    #[serde(default)]
    pub reinvocation_policy: ReinvocationPolicyType,
    #[serde(default)]
    pub match_conditions: Vec<MatchCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingWebhookSpec {
    pub name: String,
    pub client_config: WebhookClientConfig,
    #[serde(default)]
    pub rules: Vec<RuleWithOperations>,
    #[serde(default)]
    pub failure_policy: FailurePolicyType,
    #[serde(default)]
    pub match_policy: Option<MatchPolicyType>,
    #[serde(default)]
    pub namespace_selector: Option<LabelSelector>,
    #[serde(default)]
    pub object_selector: Option<LabelSelector>,
    #[serde(default)]
    pub side_effects: SideEffectClass,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: i32,
    #[serde(default)]
    pub admission_review_versions: Vec<String>,
    #[serde(default)]
    pub match_conditions: Vec<MatchCondition>,
}

fn default_timeout_seconds() -> i32 { 10 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingWebhookConfiguration {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub metadata: ObjectMeta,
    #[serde(default)]
    pub webhooks: Vec<MutatingWebhookSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingWebhookConfiguration {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub kind: String,
    pub metadata: ObjectMeta,
    #[serde(default)]
    pub webhooks: Vec<ValidatingWebhookSpec>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Validation — upstream `admissionregistration/validation/validation.go`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebhookValidationError {
    #[error("clientConfig must specify either url or service")]
    MissingClient,
    #[error("clientConfig.url and clientConfig.service are mutually exclusive")]
    ConflictingClient,
    #[error("url must be https://")]
    UrlNotHttps,
    #[error("timeoutSeconds must be 1..=30")]
    BadTimeout,
    #[error("admissionReviewVersions must include at least one of [v1, v1beta1]")]
    NoSupportedReviewVersion,
    #[error("sideEffects must be None or NoneOnDryRun for non-dry-run requests")]
    SideEffectsNotAllowed,
    #[error("port must be 1..=65535")]
    BadPort,
}

pub fn validate_client_config(c: &WebhookClientConfig) -> Result<(), WebhookValidationError> {
    match (&c.url, &c.service) {
        (None, None) => Err(WebhookValidationError::MissingClient),
        (Some(_), Some(_)) => Err(WebhookValidationError::ConflictingClient),
        (Some(u), None) => {
            if !u.starts_with("https://") {
                return Err(WebhookValidationError::UrlNotHttps);
            }
            Ok(())
        }
        (None, Some(s)) => {
            if let Some(p) = s.port {
                if !(1..=65535).contains(&p) {
                    return Err(WebhookValidationError::BadPort);
                }
            }
            Ok(())
        }
    }
}

pub fn validate_mutating_webhook(w: &MutatingWebhookSpec) -> Result<(), WebhookValidationError> {
    validate_client_config(&w.client_config)?;
    if !(1..=30).contains(&w.timeout_seconds) {
        return Err(WebhookValidationError::BadTimeout);
    }
    if !w.admission_review_versions.iter().any(|v| v == "v1" || v == "v1beta1") {
        return Err(WebhookValidationError::NoSupportedReviewVersion);
    }
    Ok(())
}

pub fn validate_validating_webhook(w: &ValidatingWebhookSpec) -> Result<(), WebhookValidationError> {
    validate_client_config(&w.client_config)?;
    if !(1..=30).contains(&w.timeout_seconds) {
        return Err(WebhookValidationError::BadTimeout);
    }
    if !w.admission_review_versions.iter().any(|v| v == "v1" || v == "v1beta1") {
        return Err(WebhookValidationError::NoSupportedReviewVersion);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Predicates — upstream `predicates/predicates.go::Matches`.
// ─────────────────────────────────────────────────────────────────────────────

fn op_matches(rule: &str, op: &Operation) -> bool {
    if rule == "*" { return true; }
    matches!((rule, op),
        ("CREATE", Operation::Create) | ("UPDATE", Operation::Update)
            | ("DELETE", Operation::Delete) | ("CONNECT", Operation::Connect))
}

fn glob_or_eq(rule: &str, val: &str) -> bool { rule == "*" || rule == val }

pub struct MatchInput<'a> {
    pub group: &'a str,
    pub version: &'a str,
    pub resource: &'a str,
    pub namespace: &'a str,
    pub operation: &'a Operation,
    pub object_labels: &'a HashMap<String, String>,
    pub namespace_labels: &'a HashMap<String, String>,
}

pub fn rule_matches(r: &RuleWithOperations, input: &MatchInput) -> bool {
    if !r.operations.iter().any(|o| op_matches(o, input.operation)) { return false; }
    if !r.api_groups.is_empty() && !r.api_groups.iter().any(|g| glob_or_eq(g, input.group)) {
        return false;
    }
    if !r.api_versions.is_empty() && !r.api_versions.iter().any(|v| glob_or_eq(v, input.version)) {
        return false;
    }
    if !r.resources.is_empty() && !r.resources.iter().any(|x| glob_or_eq(x, input.resource)) {
        return false;
    }
    match r.scope {
        ScopeType::All => true,
        ScopeType::Cluster => input.namespace.is_empty(),
        ScopeType::Namespaced => !input.namespace.is_empty(),
    }
}

pub fn label_selector_matches(sel: &LabelSelector, labels: &HashMap<String, String>) -> bool {
    for (k, v) in &sel.match_labels {
        if labels.get(k) != Some(v) { return false; }
    }
    for req in &sel.match_expressions {
        let present = labels.contains_key(&req.key);
        let val = labels.get(&req.key);
        match req.operator.as_str() {
            "In" => { let Some(v) = val else { return false; };
                      if !req.values.contains(v) { return false; } }
            "NotIn" => { if let Some(v) = val { if req.values.contains(v) { return false; } } }
            "Exists" => { if !present { return false; } }
            "DoesNotExist" => { if present { return false; } }
            _ => return false,
        }
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// CABundle parsing. We accept zero-or-more PEM CERTIFICATE blocks.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CaBundleError {
    #[error("CABundle is not valid PEM")]
    NotPem,
    #[error("CABundle PEM block is not a CERTIFICATE")]
    WrongPemKind,
    #[error("CABundle is empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCaBundle {
    pub cert_count: usize,
}

pub fn parse_ca_bundle(bundle: &[u8]) -> Result<ParsedCaBundle, CaBundleError> {
    if bundle.is_empty() { return Err(CaBundleError::Empty); }
    let s = std::str::from_utf8(bundle).map_err(|_| CaBundleError::NotPem)?;
    let mut count = 0usize;
    let mut cursor = s;
    loop {
        let Some(begin) = cursor.find("-----BEGIN ") else { break; };
        let after_begin = &cursor[begin + 11..];
        let Some(eol) = after_begin.find("-----") else { return Err(CaBundleError::NotPem); };
        let kind = &after_begin[..eol];
        if kind != "CERTIFICATE" { return Err(CaBundleError::WrongPemKind); }
        let Some(end) = after_begin.find("-----END CERTIFICATE-----") else {
            return Err(CaBundleError::NotPem);
        };
        cursor = &after_begin[end + 25..];
        count += 1;
    }
    if count == 0 { return Err(CaBundleError::NotPem); }
    Ok(ParsedCaBundle { cert_count: count })
}

// ─────────────────────────────────────────────────────────────────────────────
// WebhookClient stub trait — gated for IO. FakeWebhookClient drives tests
// without a network. RealHttpsWebhookClient is intentionally absent (#[ignore]).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WebhookCallResult {
    pub allowed: bool,
    pub status_code: u16,
    pub message: String,
    pub patches: Vec<JsonPatch>,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebhookCallError {
    #[error("dial failed")]
    DialFailed,
    #[error("tls handshake failed: {0}")]
    TlsHandshake(String),
    #[error("timed out after {0}s")]
    Timeout(i32),
    #[error("http {0}")]
    HttpStatus(u16),
    #[error("malformed AdmissionResponse")]
    Malformed,
}

pub trait WebhookClient: Send + Sync {
    fn invoke(
        &self, hook_name: &str, client_config: &WebhookClientConfig,
        timeout: Duration, req: &AdmissionRequest,
    ) -> Result<WebhookCallResult, WebhookCallError>;
}

/// Test double — answers from a name→result map. Records each invocation.
pub struct FakeWebhookClient {
    answers: RwLock<HashMap<String, Result<WebhookCallResult, WebhookCallError>>>,
    calls: RwLock<Vec<(String, AdmissionRequest)>>,
}

impl FakeWebhookClient {
    pub fn new() -> Self {
        Self { answers: RwLock::new(HashMap::new()), calls: RwLock::new(vec![]) }
    }
    pub fn answer(&self, name: &str, result: Result<WebhookCallResult, WebhookCallError>) {
        self.answers.write().unwrap().insert(name.to_string(), result);
    }
    pub fn call_count(&self, name: &str) -> usize {
        self.calls.read().unwrap().iter().filter(|(n, _)| n == name).count()
    }
    pub fn total_calls(&self) -> usize { self.calls.read().unwrap().len() }
}

impl Default for FakeWebhookClient {
    fn default() -> Self { Self::new() }
}

impl WebhookClient for FakeWebhookClient {
    fn invoke(
        &self, hook_name: &str, _: &WebhookClientConfig, _: Duration,
        req: &AdmissionRequest,
    ) -> Result<WebhookCallResult, WebhookCallError> {
        self.calls.write().unwrap().push((hook_name.to_string(), req.clone()));
        let answers = self.answers.read().unwrap();
        match answers.get(hook_name) {
            Some(Ok(r)) => Ok(r.clone()),
            Some(Err(e)) => Err(match e {
                WebhookCallError::DialFailed => WebhookCallError::DialFailed,
                WebhookCallError::TlsHandshake(m) => WebhookCallError::TlsHandshake(m.clone()),
                WebhookCallError::Timeout(n) => WebhookCallError::Timeout(*n),
                WebhookCallError::HttpStatus(c) => WebhookCallError::HttpStatus(*c),
                WebhookCallError::Malformed => WebhookCallError::Malformed,
            }),
            None => Err(WebhookCallError::DialFailed),
        }
    }
}

/// Production placeholder — would do TLS dial + HTTP POST. Returns
/// `WebhookCallError::DialFailed` so callers see a real error path
/// (instead of a panic) and tests that need real HTTPS stay `#[ignore]`d.
pub struct PanicWebhookClient;
impl WebhookClient for PanicWebhookClient {
    fn invoke(
        &self, _: &str, _: &WebhookClientConfig, _: Duration, _: &AdmissionRequest,
    ) -> Result<WebhookCallResult, WebhookCallError> {
        Err(WebhookCallError::DialFailed)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MutatingDispatcher — orchestrates the chain of mutating webhooks per request,
// honoring failurePolicy + reinvocationPolicy.
// Upstream: mutating/dispatcher.go::Dispatch
// ─────────────────────────────────────────────────────────────────────────────

pub struct MutatingDispatcher {
    pub client: Arc<dyn WebhookClient>,
    pub configs: RwLock<Vec<MutatingWebhookConfiguration>>,
}

impl MutatingDispatcher {
    pub fn new(client: Arc<dyn WebhookClient>) -> Self {
        Self { client, configs: RwLock::new(vec![]) }
    }

    pub fn upsert(&self, c: MutatingWebhookConfiguration) {
        let mut g = self.configs.write().unwrap();
        g.retain(|x| x.metadata.name != c.metadata.name);
        g.push(c);
    }

    pub fn dispatch(
        &self, req: &mut AdmissionRequest, input: &MatchInput,
    ) -> Result<Vec<JsonPatch>, WebhookCallError> {
        let configs = self.configs.read().unwrap().clone();
        let mut accumulated_patches = Vec::new();
        let mut warnings = Vec::new();
        // First pass — invoke each matching webhook in declaration order.
        let mut applied: Vec<String> = Vec::new();
        for cfg in configs.iter() {
            // Tenant scope: configs are global in upstream but our deployment
            // ties tenant_id to metadata annotation. Skip mismatch.
            if let Some(t) = cfg.metadata.annotations.get("cave.runtime/tenant-id") {
                if t != &req.tenant_id { continue; }
            }
            for w in cfg.webhooks.iter() {
                if !webhook_matches(&w.rules, &w.namespace_selector, &w.object_selector, input) {
                    continue;
                }
                if let Err(e) = self.invoke_one_mut(w, req, &mut accumulated_patches, &mut warnings) {
                    match w.failure_policy {
                        FailurePolicyType::Ignore => continue,
                        FailurePolicyType::Fail => return Err(e),
                    }
                }
                applied.push(w.name.clone());
            }
        }
        // Reinvocation pass — IfNeeded webhooks see the post-mutation object.
        for cfg in configs.iter() {
            if let Some(t) = cfg.metadata.annotations.get("cave.runtime/tenant-id") {
                if t != &req.tenant_id { continue; }
            }
            for w in cfg.webhooks.iter() {
                if w.reinvocation_policy != ReinvocationPolicyType::IfNeeded { continue; }
                if !applied.contains(&w.name) { continue; }
                if !webhook_matches(&w.rules, &w.namespace_selector, &w.object_selector, input) {
                    continue;
                }
                if let Err(e) = self.invoke_one_mut(w, req, &mut accumulated_patches, &mut warnings) {
                    if w.failure_policy == FailurePolicyType::Fail { return Err(e); }
                }
            }
        }
        Ok(accumulated_patches)
    }

    fn invoke_one_mut(
        &self, w: &MutatingWebhookSpec, req: &mut AdmissionRequest,
        patches: &mut Vec<JsonPatch>, _warnings: &mut Vec<String>,
    ) -> Result<(), WebhookCallError> {
        let timeout = Duration::from_secs(w.timeout_seconds.max(1) as u64);
        // Side-effects gate (upstream `sideeffects.go`).
        if req.dry_run && !matches!(w.side_effects, SideEffectClass::None | SideEffectClass::NoneOnDryRun) {
            return Err(WebhookCallError::Malformed);
        }
        let res = self.client.invoke(&w.name, &w.client_config, timeout, req)?;
        if !res.allowed {
            return Err(WebhookCallError::HttpStatus(res.status_code));
        }
        // Tenant invariant: webhook MUST NOT change tenant_id. Patches that
        // touch /metadata/annotations/cave.runtime~1tenant-id are rejected.
        for p in &res.patches {
            if p.path == "/metadata/annotations/cave.runtime~1tenant-id" {
                return Err(WebhookCallError::Malformed);
            }
            patches.push(p.clone());
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ValidatingDispatcher — fan-out, all validating webhooks are invoked
// concurrently in upstream; we run sequentially, short-circuit on first deny.
// Upstream: validating/dispatcher.go::Dispatch
// ─────────────────────────────────────────────────────────────────────────────

pub struct ValidatingDispatcher {
    pub client: Arc<dyn WebhookClient>,
    pub configs: RwLock<Vec<ValidatingWebhookConfiguration>>,
}

impl ValidatingDispatcher {
    pub fn new(client: Arc<dyn WebhookClient>) -> Self {
        Self { client, configs: RwLock::new(vec![]) }
    }

    pub fn upsert(&self, c: ValidatingWebhookConfiguration) {
        let mut g = self.configs.write().unwrap();
        g.retain(|x| x.metadata.name != c.metadata.name);
        g.push(c);
    }

    pub fn dispatch(
        &self, req: &AdmissionRequest, input: &MatchInput,
    ) -> Result<Vec<String>, (String, u16)> {
        let configs = self.configs.read().unwrap().clone();
        let mut warnings = Vec::new();
        for cfg in configs.iter() {
            if let Some(t) = cfg.metadata.annotations.get("cave.runtime/tenant-id") {
                if t != &req.tenant_id { continue; }
            }
            for w in cfg.webhooks.iter() {
                if !webhook_matches(&w.rules, &w.namespace_selector, &w.object_selector, input) {
                    continue;
                }
                let timeout = Duration::from_secs(w.timeout_seconds.max(1) as u64);
                let res = self.client.invoke(&w.name, &w.client_config, timeout, req);
                match res {
                    Ok(r) => {
                        warnings.extend(r.warnings.clone());
                        if !r.allowed { return Err((r.message, r.status_code)); }
                    }
                    Err(_) => {
                        if w.failure_policy == FailurePolicyType::Fail {
                            return Err((format!("webhook {} failed", w.name), 500));
                        }
                    }
                }
            }
        }
        Ok(warnings)
    }
}

fn webhook_matches(
    rules: &[RuleWithOperations],
    ns_sel: &Option<LabelSelector>, obj_sel: &Option<LabelSelector>,
    input: &MatchInput,
) -> bool {
    if !rules.is_empty() && !rules.iter().any(|r| rule_matches(r, input)) {
        return false;
    }
    if let Some(s) = ns_sel {
        if !label_selector_matches(s, input.namespace_labels) { return false; }
    }
    if let Some(s) = obj_sel {
        if !label_selector_matches(s, input.object_labels) { return false; }
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Adapters — expose a Mutating/ValidatingDispatcher as a single
// MutatingWebhook / ValidatingWebhook so it can plug into AdmissionChain.
// ─────────────────────────────────────────────────────────────────────────────

pub struct MutatingDispatcherAdapter {
    pub dispatcher: Arc<MutatingDispatcher>,
}

impl MutatingWebhook for MutatingDispatcherAdapter {
    fn name(&self) -> &str { "webhook-mutating" }
    fn admit(&self, req: &mut AdmissionRequest) -> AdmissionResponse {
        let resource = req.kind.to_lowercase();
        let namespace = req.namespace.clone();
        let operation = req.operation.clone();
        let empty = HashMap::new();
        let input = MatchInput {
            group: "", version: "v1", resource: &resource,
            namespace: &namespace, operation: &operation,
            object_labels: &empty, namespace_labels: &empty,
        };
        match self.dispatcher.dispatch(req, &input) {
            Ok(patches) => {
                let mut r = AdmissionResponse::allow(req);
                r.patches = patches;
                r
            }
            Err(e) => AdmissionResponse::deny(req, 500, format!("mutating webhook chain failed: {e}")),
        }
    }
}

pub struct ValidatingDispatcherAdapter {
    pub dispatcher: Arc<ValidatingDispatcher>,
}

impl ValidatingWebhook for ValidatingDispatcherAdapter {
    fn name(&self) -> &str { "webhook-validating" }
    fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
        let group = "";
        let version = "v1";
        let resource = req.kind.to_lowercase();
        let input = MatchInput {
            group, version, resource: &resource,
            namespace: &req.namespace, operation: &req.operation,
            object_labels: &HashMap::new(), namespace_labels: &HashMap::new(),
        };
        match self.dispatcher.dispatch(req, &input) {
            Ok(warnings) => {
                let mut r = AdmissionResponse::allow(req);
                r.warnings = warnings;
                r
            }
            Err((msg, code)) => AdmissionResponse::deny(req, code, msg),
        }
    }
}

#[cfg(test)]
mod tests;
