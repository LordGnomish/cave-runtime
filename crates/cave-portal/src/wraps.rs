//! Upstream-UI consolidation registry.
//!
//! The portal's load-bearing invariant is the single-pane-of-glass story —
//! tenants and operators never need to open an upstream tool's web UI. This
//! module enumerates every upstream UI we replace, the plugin module that
//! does the replacement, and which personas can see each view.
//!
//! The registry is queryable through `/api/portal/wraps`; the dashboard
//! renders it as a "tools we've absorbed" list. Anything missing here is by
//! definition not consolidated yet — easy to spot at a glance.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapStatus {
    /// Native re-implementation, fully shipping.
    Native,
    /// Native re-implementation, partial coverage (missing some upstream views).
    Partial,
    /// Reverse-proxy fallback — eventually replaced by native.
    Proxy,
    /// Listed but not yet wrapped.
    Planned,
}

impl WrapStatus {
    pub fn label(&self) -> &'static str {
        match self {
            WrapStatus::Native => "native",
            WrapStatus::Partial => "partial",
            WrapStatus::Proxy => "proxy",
            WrapStatus::Planned => "planned",
        }
    }

    /// Is this wrap usable by tenants today?
    pub fn is_live(&self) -> bool {
        matches!(self, WrapStatus::Native | WrapStatus::Partial | WrapStatus::Proxy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WrapPersona {
    Tenant,
    Operator,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapEntry {
    /// e.g. "argocd" — used in `/portal/<id>` routes.
    pub id: String,
    /// Human-readable name of the upstream tool.
    pub upstream_name: String,
    /// e.g. "argo-cd.example.com" — the URL that no longer needs to be visited.
    pub upstream_ui_url: Option<String>,
    /// Cave plugin module path, e.g. `cave_portal::plugins::argocd`.
    pub plugin_module: String,
    /// Native portal route, e.g. `/portal/argocd`.
    pub portal_route: String,
    pub status: WrapStatus,
    /// Personas that can see at least *some* part of this wrap.
    pub personas: Vec<WrapPersona>,
    /// One-line description.
    pub summary: String,
    /// `cave-portal` test count attributable to this wrap. Surfaces TDD
    /// confidence at a glance in the dashboard.
    pub tests: u32,
}

#[derive(Debug, Default)]
pub struct WrapRegistry {
    entries: Vec<WrapEntry>,
}

impl WrapRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Built-in catalog: every wrap currently shipping in cave-portal.
    /// Each entry must point at a real plugin module under
    /// `cave_portal::plugins::*`. Adding a wrap is a two-step PR (plugin +
    /// registry entry) so the consolidation status stays honest.
    pub fn builtin() -> Self {
        use WrapPersona::*;
        use WrapStatus::*;
        let mut r = Self::default();
        let entries = [
            (
                "argocd",
                "Argo CD",
                Some("https://argo-cd.readthedocs.io/"),
                "cave_portal::plugins::argocd + argocd_advanced",
                "/portal/argocd",
                Native,
                vec![Tenant, Operator, Admin],
                "Application list, sync/rollback, health checks, sync windows, app-of-apps tree.",
                65, // 30 + 35 from advanced
            ),
            (
                "grafana",
                "Grafana",
                Some("https://grafana.com/"),
                "cave_portal::plugins::grafana + grafana_renderer + grafana_alerting",
                "/portal/dashboards",
                Native,
                vec![Tenant, Operator, Admin],
                "Dashboards, panels, variable resolution, native SVG render, alerting + folders + annotations + channels.",
                113, // 30 + 30 + 53
            ),
            (
                "vault",
                "OpenBao / HashiCorp Vault",
                Some("https://openbao.org/"),
                "cave_portal::plugins::vault + vault_advanced",
                "/portal/vault",
                Native,
                vec![Tenant, Admin],
                "Mounts, KV entries with versioning, policies (deny-overrides), AppRole, leases (issue/renew/revoke).",
                64, // 25 + 39
            ),
            (
                "hubble",
                "Cilium Hubble",
                Some("https://docs.cilium.io/en/stable/observability/hubble/"),
                "cave_portal::plugins::hubble",
                "/portal/network",
                Native,
                vec![Operator, Admin],
                "L7 flow log, service map (forwarded/dropped edges), top-noisy ranking, HTTP status histogram.",
                36,
            ),
            (
                "defectdojo",
                "DefectDojo",
                Some("https://www.defectdojo.org/"),
                "cave_portal::plugins::defectdojo",
                "/portal/findings",
                Native,
                vec![Tenant, Operator, Admin],
                "Finding list, severity SLA tracker, risk acceptance lifecycle, by-product roll-up.",
                43,
            ),
            (
                "reflex",
                "Reflex Engine",
                None, // internal — no upstream URL
                "cave_portal::plugins::reflex + reflex_advanced",
                "/portal/actions",
                Native,
                vec![Operator, Admin],
                "Action approval queue, escalation chains with SLA, bulk approve, breach summary.",
                52, // 20 + 32
            ),
            (
                "kubernetes",
                "Kubernetes Dashboard",
                Some("https://kubernetes.io/docs/tasks/access-application-cluster/web-ui-dashboard/"),
                "cave_portal::plugins::kubernetes",
                "/portal/kubernetes",
                Native,
                vec![Tenant, Operator, Admin],
                "Per-crate panels — apiserver, kubelet, scheduler, cri pod detail, net L7 placeholder.",
                30,
            ),
            (
                "techdocs",
                "Backstage TechDocs",
                Some("https://backstage.io/docs/features/techdocs/"),
                "cave_portal::plugins::techdocs",
                "/portal/docs",
                Native,
                vec![Tenant, Operator, Admin],
                "Service doc tree, substring search, tenant-scoped indexing.",
                17,
            ),
            (
                "scaffolder",
                "Backstage Scaffolder",
                Some("https://backstage.io/docs/features/software-templates/"),
                "cave_portal::plugins::scaffolder",
                "/portal/templates",
                Native,
                vec![Tenant, Admin],
                "Software templates, parameter validation (string/number/bool/choice), pattern + default + required.",
                17,
            ),
            (
                "search",
                "Backstage Search",
                Some("https://backstage.io/docs/features/search/"),
                "cave_portal::plugins::search",
                "/portal/search",
                Native,
                vec![Tenant, Operator, Admin],
                "Cross-domain index, weighted ranking (title/tag/body), per-doc boost, kind filter.",
                18,
            ),
            (
                "cost_insight",
                "Backstage Cost Insights",
                Some("https://backstage.io/docs/features/software-catalog/well-known-annotations#backstageiocost-insights-options"),
                "cave_portal::plugins::cost_insight",
                "/portal/cost",
                Native,
                vec![Tenant, Admin],
                "Daily/category roll-up, strict ISO-day validation, 7-day moving forecast.",
                15,
            ),
            (
                "badges",
                "shields.io",
                Some("https://shields.io/"),
                "cave_portal::plugins::badges",
                "/portal/badges",
                Native,
                vec![Tenant, Operator, Admin],
                "SVG badges (coverage/build/version) with HTML escaping and color tables.",
                20,
            ),
            // Planned wraps — listed so the gap is visible.
            (
                "harbor",
                "Harbor (container registry)",
                Some("https://goharbor.io/"),
                "cave_portal::plugins::harbor (planned)",
                "/portal/registry",
                Planned,
                vec![Tenant, Admin],
                "Image list, tag immutability, vulnerability scan, signing — planned.",
                0,
            ),
            (
                "renovate",
                "Renovate Dashboard",
                Some("https://docs.renovatebot.com/"),
                "cave_portal::plugins::renovate (planned)",
                "/portal/dependencies",
                Planned,
                vec![Tenant, Admin],
                "Per-repo dependency upgrade queue, schedule, auto-merge rules — planned.",
                0,
            ),
            (
                "tempo_jaeger",
                "Tempo / Jaeger UI",
                Some("https://grafana.com/oss/tempo/"),
                "cave_portal::plugins::tracing (planned)",
                "/portal/traces",
                Planned,
                vec![Tenant, Operator, Admin],
                "Trace browser with span detail, service deps graph — planned.",
                0,
            ),
            (
                "loki",
                "Grafana Loki UI",
                Some("https://grafana.com/oss/loki/"),
                "cave_portal::plugins::loki (planned)",
                "/portal/logs",
                Planned,
                vec![Tenant, Operator, Admin],
                "Log explorer, LogQL filter, label browser — planned.",
                0,
            ),
        ];
        for (
            id,
            upstream_name,
            url,
            plugin_module,
            portal_route,
            status,
            personas,
            summary,
            tests,
        ) in entries
        {
            r.entries.push(WrapEntry {
                id: id.to_string(),
                upstream_name: upstream_name.to_string(),
                upstream_ui_url: url.map(String::from),
                plugin_module: plugin_module.to_string(),
                portal_route: portal_route.to_string(),
                status,
                personas,
                summary: summary.to_string(),
                tests,
            });
        }
        r
    }

    pub fn add(&mut self, entry: WrapEntry) {
        if let Some(idx) = self.entries.iter().position(|e| e.id == entry.id) {
            self.entries[idx] = entry;
        } else {
            self.entries.push(entry);
        }
    }

    pub fn find(&self, id: &str) -> Option<&WrapEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn entries(&self) -> &[WrapEntry] {
        &self.entries
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }

    pub fn count_by_status(&self, status: WrapStatus) -> usize {
        self.entries.iter().filter(|e| e.status == status).count()
    }

    pub fn live(&self) -> Vec<&WrapEntry> {
        self.entries.iter().filter(|e| e.status.is_live()).collect()
    }

    pub fn planned(&self) -> Vec<&WrapEntry> {
        self.entries.iter().filter(|e| e.status == WrapStatus::Planned).collect()
    }

    pub fn for_persona(&self, persona: WrapPersona) -> Vec<&WrapEntry> {
        self.entries
            .iter()
            .filter(|e| e.personas.contains(&persona))
            .collect()
    }

    pub fn total_tests(&self) -> u32 {
        self.entries.iter().map(|e| e.tests).sum()
    }

    pub fn upstream_urls_replaced(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .entries
            .iter()
            .filter(|e| e.status.is_live())
            .filter_map(|e| e.upstream_ui_url.as_deref())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Quick consolidation report — used by the dashboard top-card.
    pub fn report(&self) -> WrapReport {
        WrapReport {
            total: self.count() as u32,
            native: self.count_by_status(WrapStatus::Native) as u32,
            partial: self.count_by_status(WrapStatus::Partial) as u32,
            proxy: self.count_by_status(WrapStatus::Proxy) as u32,
            planned: self.count_by_status(WrapStatus::Planned) as u32,
            tests_total: self.total_tests(),
            upstream_uis_replaced: self.upstream_urls_replaced().len() as u32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrapReport {
    pub total: u32,
    pub native: u32,
    pub partial: u32,
    pub proxy: u32,
    pub planned: u32,
    pub tests_total: u32,
    pub upstream_uis_replaced: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, status: WrapStatus, personas: Vec<WrapPersona>) -> WrapEntry {
        WrapEntry {
            id: id.into(),
            upstream_name: id.into(),
            upstream_ui_url: Some(format!("https://{id}/")),
            plugin_module: format!("cave_portal::plugins::{id}"),
            portal_route: format!("/portal/{id}"),
            status,
            personas,
            summary: format!("wraps {id}"),
            tests: 10,
        }
    }

    // ── WrapStatus ──────────────────────────────────────────────────────────

    #[test]
    fn status_label_distinct() {
        let mut seen = std::collections::HashSet::new();
        for s in [WrapStatus::Native, WrapStatus::Partial, WrapStatus::Proxy, WrapStatus::Planned] {
            assert!(seen.insert(s.label()));
        }
    }

    #[test]
    fn status_is_live_only_planned_is_dead() {
        assert!(WrapStatus::Native.is_live());
        assert!(WrapStatus::Partial.is_live());
        assert!(WrapStatus::Proxy.is_live());
        assert!(!WrapStatus::Planned.is_live());
    }

    #[test]
    fn status_serializes_snake_case() {
        let s = serde_json::to_string(&WrapStatus::Native).unwrap();
        assert_eq!(s, "\"native\"");
    }

    // ── WrapEntry ───────────────────────────────────────────────────────────

    #[test]
    fn wrap_entry_round_trips_json() {
        let e = entry("argocd", WrapStatus::Native, vec![WrapPersona::Tenant]);
        let s = serde_json::to_string(&e).unwrap();
        let back: WrapEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn wrap_persona_serializes_snake_case() {
        let s = serde_json::to_string(&WrapPersona::Operator).unwrap();
        assert_eq!(s, "\"operator\"");
    }

    // ── Registry — empty ────────────────────────────────────────────────────

    #[test]
    fn registry_starts_empty() {
        let r = WrapRegistry::new();
        assert_eq!(r.count(), 0);
        assert!(r.live().is_empty());
        assert!(r.planned().is_empty());
        assert_eq!(r.total_tests(), 0);
    }

    // ── Registry — add/replace ──────────────────────────────────────────────

    #[test]
    fn registry_add_inserts() {
        let mut r = WrapRegistry::new();
        r.add(entry("a", WrapStatus::Native, vec![WrapPersona::Tenant]));
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn registry_add_same_id_replaces() {
        let mut r = WrapRegistry::new();
        r.add(entry("a", WrapStatus::Planned, vec![]));
        r.add(entry("a", WrapStatus::Native, vec![WrapPersona::Tenant]));
        assert_eq!(r.count(), 1);
        assert_eq!(r.find("a").unwrap().status, WrapStatus::Native);
    }

    #[test]
    fn registry_find_unknown_returns_none() {
        let r = WrapRegistry::new();
        assert!(r.find("ghost").is_none());
    }

    // ── Registry — filters ──────────────────────────────────────────────────

    #[test]
    fn registry_count_by_status() {
        let mut r = WrapRegistry::new();
        r.add(entry("a", WrapStatus::Native, vec![]));
        r.add(entry("b", WrapStatus::Native, vec![]));
        r.add(entry("c", WrapStatus::Planned, vec![]));
        assert_eq!(r.count_by_status(WrapStatus::Native), 2);
        assert_eq!(r.count_by_status(WrapStatus::Planned), 1);
        assert_eq!(r.count_by_status(WrapStatus::Proxy), 0);
    }

    #[test]
    fn registry_live_excludes_planned() {
        let mut r = WrapRegistry::new();
        r.add(entry("a", WrapStatus::Native, vec![]));
        r.add(entry("b", WrapStatus::Planned, vec![]));
        let live = r.live();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "a");
    }

    #[test]
    fn registry_planned_only() {
        let mut r = WrapRegistry::new();
        r.add(entry("a", WrapStatus::Native, vec![]));
        r.add(entry("b", WrapStatus::Planned, vec![]));
        r.add(entry("c", WrapStatus::Planned, vec![]));
        assert_eq!(r.planned().len(), 2);
    }

    #[test]
    fn registry_for_persona_filters() {
        let mut r = WrapRegistry::new();
        r.add(entry("a", WrapStatus::Native, vec![WrapPersona::Tenant]));
        r.add(entry("b", WrapStatus::Native, vec![WrapPersona::Operator]));
        r.add(entry("c", WrapStatus::Native, vec![WrapPersona::Tenant, WrapPersona::Operator]));
        assert_eq!(r.for_persona(WrapPersona::Tenant).len(), 2);
        assert_eq!(r.for_persona(WrapPersona::Operator).len(), 2);
        assert_eq!(r.for_persona(WrapPersona::Admin).len(), 0);
    }

    #[test]
    fn registry_total_tests_sums() {
        let mut r = WrapRegistry::new();
        let mut a = entry("a", WrapStatus::Native, vec![]);
        a.tests = 30;
        let mut b = entry("b", WrapStatus::Native, vec![]);
        b.tests = 50;
        r.add(a);
        r.add(b);
        assert_eq!(r.total_tests(), 80);
    }

    #[test]
    fn registry_upstream_urls_dedups_and_skips_planned() {
        let mut r = WrapRegistry::new();
        let mut a = entry("a", WrapStatus::Native, vec![]);
        a.upstream_ui_url = Some("https://x/".into());
        let mut b = entry("b", WrapStatus::Native, vec![]);
        b.upstream_ui_url = Some("https://x/".into());
        let mut c = entry("c", WrapStatus::Planned, vec![]);
        c.upstream_ui_url = Some("https://y/".into());
        r.add(a);
        r.add(b);
        r.add(c);
        let urls = r.upstream_urls_replaced();
        assert_eq!(urls, vec!["https://x/"]);
    }

    #[test]
    fn registry_upstream_urls_omits_when_none() {
        let mut r = WrapRegistry::new();
        let mut a = entry("a", WrapStatus::Native, vec![]);
        a.upstream_ui_url = None;
        r.add(a);
        assert!(r.upstream_urls_replaced().is_empty());
    }

    // ── Registry — report ───────────────────────────────────────────────────

    #[test]
    fn registry_report_counts_match() {
        let mut r = WrapRegistry::new();
        r.add(entry("a", WrapStatus::Native, vec![]));
        r.add(entry("b", WrapStatus::Partial, vec![]));
        r.add(entry("c", WrapStatus::Proxy, vec![]));
        r.add(entry("d", WrapStatus::Planned, vec![]));
        let rep = r.report();
        assert_eq!(rep.total, 4);
        assert_eq!(rep.native, 1);
        assert_eq!(rep.partial, 1);
        assert_eq!(rep.proxy, 1);
        assert_eq!(rep.planned, 1);
    }

    // ── Builtin catalog ─────────────────────────────────────────────────────

    #[test]
    fn builtin_has_all_known_wraps() {
        let r = WrapRegistry::builtin();
        for id in [
            "argocd", "grafana", "vault", "hubble", "defectdojo", "reflex",
            "kubernetes", "techdocs", "scaffolder", "search", "cost_insight", "badges",
        ] {
            assert!(r.find(id).is_some(), "missing wrap {id}");
        }
    }

    #[test]
    fn builtin_has_planned_wraps() {
        let r = WrapRegistry::builtin();
        for id in ["harbor", "renovate", "tempo_jaeger", "loki"] {
            assert!(r.find(id).is_some(), "missing planned wrap {id}");
            assert_eq!(r.find(id).unwrap().status, WrapStatus::Planned);
        }
    }

    #[test]
    fn builtin_native_count_matches_known_set() {
        let r = WrapRegistry::builtin();
        // 12 native + 0 partial + 0 proxy + 4 planned
        assert_eq!(r.count_by_status(WrapStatus::Native), 12);
        assert_eq!(r.count_by_status(WrapStatus::Planned), 4);
    }

    #[test]
    fn builtin_total_tests_nonzero() {
        let r = WrapRegistry::builtin();
        assert!(r.total_tests() > 400, "tests should reflect substantial coverage");
    }

    #[test]
    fn builtin_every_native_has_portal_route() {
        let r = WrapRegistry::builtin();
        for e in r.live() {
            assert!(e.portal_route.starts_with("/portal/"), "bad route on {}", e.id);
        }
    }

    #[test]
    fn builtin_every_native_has_at_least_one_persona() {
        let r = WrapRegistry::builtin();
        for e in r.live() {
            assert!(!e.personas.is_empty(), "wrap {} has no personas", e.id);
        }
    }

    #[test]
    fn builtin_every_entry_has_summary() {
        let r = WrapRegistry::builtin();
        for e in r.entries() {
            assert!(!e.summary.is_empty(), "wrap {} has empty summary", e.id);
        }
    }

    #[test]
    fn builtin_no_duplicate_ids() {
        let r = WrapRegistry::builtin();
        let mut ids = std::collections::HashSet::new();
        for e in r.entries() {
            assert!(ids.insert(e.id.clone()), "duplicate id {}", e.id);
        }
    }

    #[test]
    fn builtin_report_total_consistent() {
        let r = WrapRegistry::builtin();
        let rep = r.report();
        assert_eq!(rep.total, rep.native + rep.partial + rep.proxy + rep.planned);
    }

    #[test]
    fn builtin_upstream_urls_replaced_count_matches_live_with_url() {
        let r = WrapRegistry::builtin();
        let want = r.live().iter().filter(|e| e.upstream_ui_url.is_some()).count();
        // dedup should not reduce it for the builtin set (each wrap has a unique URL or None)
        assert_eq!(r.upstream_urls_replaced().len(), want);
    }
}
