//! Multi-tenant routing — tenant_id label-based virtual host with
//! default-deny cross-tenant access and per-tenant rate-limit.
//!
//! Sovereign-profile feature: each route / service / cluster is labelled with
//! a `tenant_id`. The gateway resolves the inbound request's tenant from
//! header / JWT claim / path prefix / SNI, then enforces:
//!
//! 1. **Default-deny cross-tenant**: a tenant cannot access a resource
//!    labelled with a different tenant unless an explicit allowlist rule
//!    permits it.
//! 2. **Per-tenant rate-limit**: each tenant has its own token-bucket budget.
//! 3. **Per-tenant virtual host**: routes can be scoped to a tenant; the
//!    matcher only considers routes for the resolved tenant.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ── TenantId ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TenantId(pub String);

impl TenantId {
    pub fn new(s: impl Into<String>) -> Result<Self, TenantError> {
        let s = s.into();
        if s.is_empty() {
            return Err(TenantError::EmptyTenantId);
        }
        if s.len() > 128 {
            return Err(TenantError::TenantIdTooLong);
        }
        for c in s.chars() {
            if !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
                return Err(TenantError::InvalidTenantIdChar(c));
            }
        }
        Ok(TenantId(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum TenantError {
    #[error("tenant id is empty")]
    EmptyTenantId,
    #[error("tenant id longer than 128 characters")]
    TenantIdTooLong,
    #[error("invalid char in tenant id: {0:?}")]
    InvalidTenantIdChar(char),
    #[error("tenant {0} unknown")]
    UnknownTenant(String),
    #[error("tenant {0} disabled")]
    TenantDisabled(String),
    #[error("tenant could not be resolved from request")]
    Unresolved,
    #[error("cross-tenant access denied: {actor} ↛ {resource}")]
    CrossTenantDenied { actor: String, resource: String },
    #[error("tenant {0} rate limit exceeded")]
    RateLimited(String),
}

// ── Tenant record + registry ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
    pub enabled: bool,
    pub rate_limit_rpm: u32,
    pub burst: u32,
    /// Other tenants this tenant may access (default-deny w/ explicit allow).
    pub allowed_peers: Vec<TenantId>,
    /// Domain names served by this tenant (used for SNI-based resolution).
    pub domains: Vec<String>,
}

impl Tenant {
    pub fn new(id: TenantId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            enabled: true,
            rate_limit_rpm: 600,
            burst: 60,
            allowed_peers: vec![],
            domains: vec![],
        }
    }

    pub fn validate(&self) -> Result<(), TenantError> {
        if self.rate_limit_rpm == 0 {
            // 0 means disabled rate-limit, which is allowed.
        }
        // Domain validation reuses simple ASCII-host rules.
        for d in &self.domains {
            if d.is_empty() || d.contains('/') {
                return Err(TenantError::InvalidTenantIdChar('/'));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct TenantRegistry {
    tenants: RwLock<BTreeMap<TenantId, Arc<Tenant>>>,
    /// domain → tenant_id index for SNI-based resolution.
    by_domain: RwLock<HashMap<String, TenantId>>,
}

impl TenantRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tenants: RwLock::new(BTreeMap::new()),
            by_domain: RwLock::new(HashMap::new()),
        })
    }

    pub fn upsert(&self, tenant: Tenant) -> Result<(), TenantError> {
        tenant.validate()?;
        let mut g = self.tenants.write().unwrap();
        let mut idx = self.by_domain.write().unwrap();
        // Drop old domain bindings.
        if let Some(prev) = g.get(&tenant.id) {
            for d in &prev.domains {
                idx.remove(d);
            }
        }
        for d in &tenant.domains {
            idx.insert(d.to_ascii_lowercase(), tenant.id.clone());
        }
        g.insert(tenant.id.clone(), Arc::new(tenant));
        Ok(())
    }

    pub fn get(&self, id: &TenantId) -> Option<Arc<Tenant>> {
        self.tenants.read().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<Tenant>> {
        self.tenants.read().unwrap().values().cloned().collect()
    }

    pub fn remove(&self, id: &TenantId) -> bool {
        let mut g = self.tenants.write().unwrap();
        if let Some(t) = g.remove(id) {
            let mut idx = self.by_domain.write().unwrap();
            for d in &t.domains {
                idx.remove(d);
            }
            return true;
        }
        false
    }

    pub fn by_domain(&self, domain: &str) -> Option<TenantId> {
        self.by_domain
            .read()
            .unwrap()
            .get(&domain.to_ascii_lowercase())
            .cloned()
    }
}

// ── Resolver ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantResolverConfig {
    /// Header name carrying the tenant id (e.g. `X-Tenant-Id`).
    pub header_name: Option<String>,
    /// JWT claim name to extract.
    pub jwt_claim: Option<String>,
    /// Path prefix that introduces a tenant, e.g. `/t/` (consumed during
    /// routing).
    pub path_prefix: Option<String>,
    /// If true, also use SNI/host as a resolution source.
    pub use_sni: bool,
    /// Order to try resolution sources. Defaults to: header, jwt, path, sni.
    pub order: Vec<ResolveSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveSource {
    Header,
    Jwt,
    Path,
    Sni,
}

impl Default for TenantResolverConfig {
    fn default() -> Self {
        Self {
            header_name: Some("x-tenant-id".into()),
            jwt_claim: Some("tenant_id".into()),
            path_prefix: Some("/t/".into()),
            use_sni: true,
            order: vec![
                ResolveSource::Header,
                ResolveSource::Jwt,
                ResolveSource::Path,
                ResolveSource::Sni,
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolveInputs<'a> {
    pub headers: &'a HashMap<String, String>,
    pub path: &'a str,
    pub host: &'a str,
    pub jwt_claims: &'a HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTenant {
    pub id: TenantId,
    pub source: ResolveSource,
    /// If resolved from the path, the tenant prefix is consumed.
    pub stripped_path: Option<String>,
}

pub fn resolve<'a>(
    cfg: &TenantResolverConfig,
    registry: &TenantRegistry,
    inputs: &ResolveInputs<'a>,
) -> Result<ResolvedTenant, TenantError> {
    for src in &cfg.order {
        match src {
            ResolveSource::Header => {
                if let Some(name) = &cfg.header_name {
                    if let Some(v) = inputs.headers.get(&name.to_ascii_lowercase()) {
                        let id = TenantId::new(v.clone())?;
                        ensure_enabled(registry, &id)?;
                        return Ok(ResolvedTenant {
                            id,
                            source: ResolveSource::Header,
                            stripped_path: None,
                        });
                    }
                }
            }
            ResolveSource::Jwt => {
                if let Some(claim) = &cfg.jwt_claim {
                    if let Some(v) = inputs.jwt_claims.get(claim) {
                        let id = TenantId::new(v.clone())?;
                        ensure_enabled(registry, &id)?;
                        return Ok(ResolvedTenant {
                            id,
                            source: ResolveSource::Jwt,
                            stripped_path: None,
                        });
                    }
                }
            }
            ResolveSource::Path => {
                if let Some(prefix) = &cfg.path_prefix {
                    if let Some(after) = inputs.path.strip_prefix(prefix.as_str()) {
                        let (raw_id, rest) = match after.find('/') {
                            Some(i) => (&after[..i], &after[i..]),
                            None => (after, ""),
                        };
                        if !raw_id.is_empty() {
                            let id = TenantId::new(raw_id.to_string())?;
                            ensure_enabled(registry, &id)?;
                            let stripped = if rest.is_empty() { "/".to_string() } else { rest.to_string() };
                            return Ok(ResolvedTenant {
                                id,
                                source: ResolveSource::Path,
                                stripped_path: Some(stripped),
                            });
                        }
                    }
                }
            }
            ResolveSource::Sni => {
                if cfg.use_sni && !inputs.host.is_empty() {
                    if let Some(id) = registry.by_domain(inputs.host) {
                        ensure_enabled(registry, &id)?;
                        return Ok(ResolvedTenant {
                            id,
                            source: ResolveSource::Sni,
                            stripped_path: None,
                        });
                    }
                }
            }
        }
    }
    Err(TenantError::Unresolved)
}

fn ensure_enabled(registry: &TenantRegistry, id: &TenantId) -> Result<(), TenantError> {
    let t = registry
        .get(id)
        .ok_or_else(|| TenantError::UnknownTenant(id.0.clone()))?;
    if !t.enabled {
        return Err(TenantError::TenantDisabled(id.0.clone()));
    }
    Ok(())
}

// ── Cross-tenant access policy ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossTenantPolicy {
    pub default_action: PolicyAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    Allow,
    Deny,
}

impl Default for CrossTenantPolicy {
    fn default() -> Self {
        // Sovereign default: cross-tenant requests denied unless an explicit
        // allow rule (peer allow-list on the actor) permits it.
        Self { default_action: PolicyAction::Deny }
    }
}

pub fn check_cross_tenant(
    policy: &CrossTenantPolicy,
    registry: &TenantRegistry,
    actor: &TenantId,
    resource_owner: &TenantId,
) -> Result<(), TenantError> {
    if actor == resource_owner {
        return Ok(());
    }
    if let Some(actor_record) = registry.get(actor) {
        if actor_record.allowed_peers.contains(resource_owner) {
            return Ok(());
        }
    }
    match policy.default_action {
        PolicyAction::Allow => Ok(()),
        PolicyAction::Deny => Err(TenantError::CrossTenantDenied {
            actor: actor.0.clone(),
            resource: resource_owner.0.clone(),
        }),
    }
}

// ── Per-tenant rate limiter (token bucket) ──────────────────────────────────

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
    capacity: f64,
    refill_per_sec: f64,
}

impl Bucket {
    fn new(capacity: u32, refill_per_sec: f64) -> Self {
        Self {
            tokens: capacity as f64,
            last_refill: Instant::now(),
            capacity: capacity as f64,
            refill_per_sec,
        }
    }

    fn try_take(&mut self, n: u32, now: Instant) -> bool {
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= n as f64 {
            self.tokens -= n as f64;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Default)]
pub struct TenantRateLimiter {
    buckets: Mutex<HashMap<TenantId, Bucket>>,
}

impl TenantRateLimiter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Charge one request. Tenants must already exist in the registry.
    pub fn allow(
        &self,
        registry: &TenantRegistry,
        tenant_id: &TenantId,
        now: Instant,
    ) -> Result<(), TenantError> {
        let tenant = registry
            .get(tenant_id)
            .ok_or_else(|| TenantError::UnknownTenant(tenant_id.0.clone()))?;
        if tenant.rate_limit_rpm == 0 {
            return Ok(());
        }
        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets.entry(tenant_id.clone()).or_insert_with(|| {
            Bucket::new(tenant.burst.max(1), tenant.rate_limit_rpm as f64 / 60.0)
        });
        if bucket.try_take(1, now) {
            Ok(())
        } else {
            Err(TenantError::RateLimited(tenant_id.0.clone()))
        }
    }

    pub fn snapshot(&self) -> HashMap<TenantId, f64> {
        let buckets = self.buckets.lock().unwrap();
        buckets.iter().map(|(k, b)| (k.clone(), b.tokens)).collect()
    }
}

// ── Tenant-scoped routes ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantScope {
    pub tenant_id: TenantId,
}

#[derive(Debug, Default)]
pub struct TenantScopedIndex {
    /// route_id → tenant ownership
    owners: RwLock<HashMap<String, TenantId>>,
}

impl TenantScopedIndex {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_owner(&self, route_id: impl Into<String>, owner: TenantId) {
        self.owners.write().unwrap().insert(route_id.into(), owner);
    }

    pub fn get_owner(&self, route_id: &str) -> Option<TenantId> {
        self.owners.read().unwrap().get(route_id).cloned()
    }

    pub fn unique_tenants(&self) -> HashSet<TenantId> {
        self.owners.read().unwrap().values().cloned().collect()
    }

    pub fn routes_for(&self, tenant: &TenantId) -> Vec<String> {
        self.owners
            .read()
            .unwrap()
            .iter()
            .filter(|(_, o)| *o == tenant)
            .map(|(r, _)| r.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> TenantId {
        TenantId::new(s).unwrap()
    }

    fn registry_with(tenants: Vec<Tenant>) -> Arc<TenantRegistry> {
        let r = TenantRegistry::new();
        for t in tenants {
            r.upsert(t).unwrap();
        }
        r
    }

    fn inputs<'a>(
        headers: &'a HashMap<String, String>,
        path: &'a str,
        host: &'a str,
        jwt_claims: &'a HashMap<String, String>,
    ) -> ResolveInputs<'a> {
        ResolveInputs { headers, path, host, jwt_claims }
    }

    fn h(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), (*v).to_string()))
            .collect()
    }

    // ── TenantId validation ──

    #[test]
    fn id_basic() {
        TenantId::new("acme-co_99").unwrap();
    }

    #[test]
    fn id_empty_rejects() {
        assert_eq!(TenantId::new(""), Err(TenantError::EmptyTenantId));
    }

    #[test]
    fn id_too_long_rejects() {
        let long = "a".repeat(129);
        assert_eq!(TenantId::new(long), Err(TenantError::TenantIdTooLong));
    }

    #[test]
    fn id_invalid_char_rejects() {
        assert!(matches!(
            TenantId::new("acme!"),
            Err(TenantError::InvalidTenantIdChar('!'))
        ));
    }

    #[test]
    fn id_display() {
        assert_eq!(format!("{}", id("a")), "a");
    }

    // ── Tenant validate ──

    #[test]
    fn tenant_default_validates() {
        Tenant::new(id("a"), "Acme").validate().unwrap();
    }

    #[test]
    fn tenant_with_domains_validates() {
        let mut t = Tenant::new(id("a"), "Acme");
        t.domains = vec!["a.example.com".into(), "b.example.com".into()];
        t.validate().unwrap();
    }

    #[test]
    fn tenant_invalid_domain_rejects() {
        let mut t = Tenant::new(id("a"), "Acme");
        t.domains = vec!["not/a/domain".into()];
        assert!(t.validate().is_err());
    }

    // ── Registry ──

    #[test]
    fn registry_upsert_and_get() {
        let r = TenantRegistry::new();
        r.upsert(Tenant::new(id("a"), "A")).unwrap();
        assert!(r.get(&id("a")).is_some());
    }

    #[test]
    fn registry_remove_clears_domain_index() {
        let r = TenantRegistry::new();
        let mut t = Tenant::new(id("a"), "A");
        t.domains = vec!["a.example.com".into()];
        r.upsert(t).unwrap();
        assert!(r.by_domain("a.example.com").is_some());
        r.remove(&id("a"));
        assert!(r.by_domain("a.example.com").is_none());
    }

    #[test]
    fn registry_by_domain_case_insensitive() {
        let r = TenantRegistry::new();
        let mut t = Tenant::new(id("a"), "A");
        t.domains = vec!["A.EXAMPLE.COM".into()];
        r.upsert(t).unwrap();
        assert!(r.by_domain("a.example.com").is_some());
    }

    #[test]
    fn registry_upsert_replaces_old_domains() {
        let r = TenantRegistry::new();
        let mut t = Tenant::new(id("a"), "A");
        t.domains = vec!["old.example.com".into()];
        r.upsert(t).unwrap();
        let mut t2 = Tenant::new(id("a"), "A");
        t2.domains = vec!["new.example.com".into()];
        r.upsert(t2).unwrap();
        assert!(r.by_domain("old.example.com").is_none());
        assert!(r.by_domain("new.example.com").is_some());
    }

    #[test]
    fn registry_list_sorted() {
        let r = TenantRegistry::new();
        r.upsert(Tenant::new(id("z"), "Z")).unwrap();
        r.upsert(Tenant::new(id("a"), "A")).unwrap();
        let names: Vec<_> = r.list().iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, vec!["A", "Z"]);
    }

    // ── Resolver ──

    #[test]
    fn resolve_header_first() {
        let r = registry_with(vec![Tenant::new(id("acme"), "A")]);
        let cfg = TenantResolverConfig::default();
        let h = h(&[("X-Tenant-Id", "acme")]);
        let claims = HashMap::new();
        let i = inputs(&h, "/", "x.com", &claims);
        let res = resolve(&cfg, &r, &i).unwrap();
        assert_eq!(res.id, id("acme"));
        assert_eq!(res.source, ResolveSource::Header);
    }

    #[test]
    fn resolve_jwt_claim() {
        let r = registry_with(vec![Tenant::new(id("acme"), "A")]);
        let cfg = TenantResolverConfig::default();
        let h = h(&[]);
        let mut claims = HashMap::new();
        claims.insert("tenant_id".into(), "acme".into());
        let i = inputs(&h, "/", "x.com", &claims);
        let res = resolve(&cfg, &r, &i).unwrap();
        assert_eq!(res.source, ResolveSource::Jwt);
    }

    #[test]
    fn resolve_path_prefix_strips() {
        let r = registry_with(vec![Tenant::new(id("acme"), "A")]);
        let cfg = TenantResolverConfig::default();
        let h = h(&[]);
        let claims = HashMap::new();
        let i = inputs(&h, "/t/acme/api/users", "x.com", &claims);
        let res = resolve(&cfg, &r, &i).unwrap();
        assert_eq!(res.source, ResolveSource::Path);
        assert_eq!(res.stripped_path.as_deref(), Some("/api/users"));
    }

    #[test]
    fn resolve_path_prefix_root_only() {
        let r = registry_with(vec![Tenant::new(id("acme"), "A")]);
        let cfg = TenantResolverConfig::default();
        let h = h(&[]);
        let claims = HashMap::new();
        let i = inputs(&h, "/t/acme", "x.com", &claims);
        let res = resolve(&cfg, &r, &i).unwrap();
        assert_eq!(res.stripped_path.as_deref(), Some("/"));
    }

    #[test]
    fn resolve_sni_from_domain() {
        let r = TenantRegistry::new();
        let mut t = Tenant::new(id("acme"), "A");
        t.domains = vec!["api.acme.com".into()];
        r.upsert(t).unwrap();
        let cfg = TenantResolverConfig::default();
        let h = h(&[]);
        let claims = HashMap::new();
        let i = inputs(&h, "/", "api.acme.com", &claims);
        let res = resolve(&cfg, &r, &i).unwrap();
        assert_eq!(res.source, ResolveSource::Sni);
    }

    #[test]
    fn resolve_unresolved_returns_error() {
        let r = registry_with(vec![]);
        let cfg = TenantResolverConfig::default();
        let h = h(&[]);
        let claims = HashMap::new();
        let i = inputs(&h, "/", "x.com", &claims);
        assert_eq!(resolve(&cfg, &r, &i), Err(TenantError::Unresolved));
    }

    #[test]
    fn resolve_unknown_tenant_returns_error() {
        let r = registry_with(vec![]);
        let cfg = TenantResolverConfig::default();
        let h = h(&[("X-Tenant-Id", "ghost")]);
        let claims = HashMap::new();
        let i = inputs(&h, "/", "x.com", &claims);
        assert!(matches!(
            resolve(&cfg, &r, &i),
            Err(TenantError::UnknownTenant(_))
        ));
    }

    #[test]
    fn resolve_disabled_tenant_returns_error() {
        let mut t = Tenant::new(id("acme"), "A");
        t.enabled = false;
        let r = registry_with(vec![t]);
        let cfg = TenantResolverConfig::default();
        let h = h(&[("X-Tenant-Id", "acme")]);
        let claims = HashMap::new();
        let i = inputs(&h, "/", "x.com", &claims);
        assert!(matches!(
            resolve(&cfg, &r, &i),
            Err(TenantError::TenantDisabled(_))
        ));
    }

    #[test]
    fn resolve_order_respected() {
        let r = registry_with(vec![
            Tenant::new(id("from-jwt"), "A"),
            Tenant::new(id("from-header"), "B"),
        ]);
        let mut cfg = TenantResolverConfig::default();
        // Reverse default order — Jwt first.
        cfg.order = vec![ResolveSource::Jwt, ResolveSource::Header];
        let h = h(&[("X-Tenant-Id", "from-header")]);
        let mut claims = HashMap::new();
        claims.insert("tenant_id".into(), "from-jwt".into());
        let i = inputs(&h, "/", "x.com", &claims);
        let res = resolve(&cfg, &r, &i).unwrap();
        assert_eq!(res.id, id("from-jwt"));
    }

    // ── Cross-tenant policy ──

    #[test]
    fn cross_tenant_same_tenant_passes() {
        let r = registry_with(vec![Tenant::new(id("a"), "A")]);
        check_cross_tenant(&CrossTenantPolicy::default(), &r, &id("a"), &id("a")).unwrap();
    }

    #[test]
    fn cross_tenant_default_deny() {
        let r = registry_with(vec![
            Tenant::new(id("a"), "A"),
            Tenant::new(id("b"), "B"),
        ]);
        let res = check_cross_tenant(&CrossTenantPolicy::default(), &r, &id("a"), &id("b"));
        assert!(matches!(res, Err(TenantError::CrossTenantDenied { .. })));
    }

    #[test]
    fn cross_tenant_explicit_allow_passes() {
        let mut a = Tenant::new(id("a"), "A");
        a.allowed_peers = vec![id("b")];
        let r = registry_with(vec![a, Tenant::new(id("b"), "B")]);
        check_cross_tenant(&CrossTenantPolicy::default(), &r, &id("a"), &id("b")).unwrap();
    }

    #[test]
    fn cross_tenant_default_allow_passes() {
        let r = registry_with(vec![
            Tenant::new(id("a"), "A"),
            Tenant::new(id("b"), "B"),
        ]);
        let policy = CrossTenantPolicy { default_action: PolicyAction::Allow };
        check_cross_tenant(&policy, &r, &id("a"), &id("b")).unwrap();
    }

    // ── Rate limiter ──

    #[test]
    fn rate_limit_burst_allows_n() {
        let r = registry_with(vec![{
            let mut t = Tenant::new(id("a"), "A");
            t.rate_limit_rpm = 60;
            t.burst = 5;
            t
        }]);
        let rl = TenantRateLimiter::new();
        let now = Instant::now();
        for _ in 0..5 {
            rl.allow(&r, &id("a"), now).unwrap();
        }
        let again = rl.allow(&r, &id("a"), now);
        assert!(matches!(again, Err(TenantError::RateLimited(_))));
    }

    #[test]
    fn rate_limit_refills() {
        let r = registry_with(vec![{
            let mut t = Tenant::new(id("a"), "A");
            t.rate_limit_rpm = 60; // 1 req/s
            t.burst = 1;
            t
        }]);
        let rl = TenantRateLimiter::new();
        let now = Instant::now();
        rl.allow(&r, &id("a"), now).unwrap();
        assert!(matches!(
            rl.allow(&r, &id("a"), now),
            Err(TenantError::RateLimited(_))
        ));
        // Advance the bucket clock by 2 seconds → 2 tokens refilled, capped at 1.
        let later = now + Duration::from_secs(2);
        rl.allow(&r, &id("a"), later).unwrap();
    }

    #[test]
    fn rate_limit_zero_means_unlimited() {
        let r = registry_with(vec![{
            let mut t = Tenant::new(id("a"), "A");
            t.rate_limit_rpm = 0;
            t.burst = 0;
            t
        }]);
        let rl = TenantRateLimiter::new();
        let now = Instant::now();
        for _ in 0..1000 {
            rl.allow(&r, &id("a"), now).unwrap();
        }
    }

    #[test]
    fn rate_limit_unknown_tenant_errors() {
        let r = registry_with(vec![]);
        let rl = TenantRateLimiter::new();
        let now = Instant::now();
        assert!(matches!(
            rl.allow(&r, &id("ghost"), now),
            Err(TenantError::UnknownTenant(_))
        ));
    }

    // ── TenantScopedIndex ──

    #[test]
    fn scoped_index_sets_and_gets_owner() {
        let s = TenantScopedIndex::new();
        s.set_owner("route-1", id("a"));
        assert_eq!(s.get_owner("route-1"), Some(id("a")));
    }

    #[test]
    fn scoped_index_routes_for_tenant() {
        let s = TenantScopedIndex::new();
        s.set_owner("r1", id("a"));
        s.set_owner("r2", id("a"));
        s.set_owner("r3", id("b"));
        let mut routes = s.routes_for(&id("a"));
        routes.sort();
        assert_eq!(routes, vec!["r1".to_string(), "r2".to_string()]);
    }

    #[test]
    fn scoped_index_unique_tenants() {
        let s = TenantScopedIndex::new();
        s.set_owner("r1", id("a"));
        s.set_owner("r2", id("a"));
        s.set_owner("r3", id("b"));
        assert_eq!(s.unique_tenants().len(), 2);
    }

    // ── Serde ──

    #[test]
    fn tenant_json_roundtrip() {
        let mut t = Tenant::new(id("acme"), "Acme");
        t.allowed_peers = vec![id("bcorp")];
        t.domains = vec!["acme.com".into()];
        let s = serde_json::to_string(&t).unwrap();
        let t2: Tenant = serde_json::from_str(&s).unwrap();
        assert_eq!(t, t2);
    }

    #[test]
    fn cross_tenant_policy_json_roundtrip() {
        let p = CrossTenantPolicy::default();
        let s = serde_json::to_string(&p).unwrap();
        let p2: CrossTenantPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn resolver_config_json_roundtrip() {
        let cfg = TenantResolverConfig::default();
        let s = serde_json::to_string(&cfg).unwrap();
        let cfg2: TenantResolverConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, cfg2);
    }
}
