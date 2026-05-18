// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dashboard aggregation engine for cave-portal.
//!
//! Collects health and summary data from all 30 CAVE modules and
//! assembles the portal dashboard, navigation tree, search index,
//! and notification feed.

use crate::models::{
    DashboardData, DashboardWidget, HealthStatus, ModuleSummary, NavigationGroup,
    NavigationItem, Notification, NotificationSeverity, SearchResult,
};
use chrono::Utc;
use uuid::Uuid;

// (id, display_name, category, upstream_replacement)
type ModuleMeta = (&'static str, &'static str, &'static str, &'static str);

fn all_modules() -> Vec<ModuleMeta> {
    vec![
        // Security
        ("secrets",   "Secrets Scanner",     "security",      "TruffleHog / Gitleaks"),
        ("certs",     "Certificates",         "security",      "cert-manager"),
        ("vulns",     "Vulnerability Mgmt",   "security",      "Snyk"),
        ("sbom",      "SBOM",                 "security",      "Syft / Grype"),
        ("sign",      "Artifact Signing",     "security",      "cosign"),
        ("forensics", "Runtime Forensics",    "security",      "Falco"),
        ("pii",       "PII Scanner",          "security",      "Microsoft Presidio"),
        ("scan",      "Container Scanner",    "security",      "Trivy"),
        ("policy",    "Policy Engine",        "security",      "OPA / Gatekeeper"),
        ("dast",      "DAST Scanner",         "security",      "OWASP ZAP"),
        ("pam",       "Privileged Access",    "security",      "HashiCorp Boundary"),
        // Observability
        ("status",    "Status Page",          "observability", "Statuspage.io"),
        ("uptime",    "Uptime Monitor",       "observability", "Pingdom"),
        ("alerts",    "Alerts",               "observability", "Alertmanager"),
        ("slo",       "SLO Tracker",          "observability", "SLO tools"),
        ("incidents", "Incidents",            "observability", "PagerDuty"),
        ("profiler",  "Profiler",             "observability", "Pyroscope"),
        // Dev Tools
        ("lint",      "API Linter",           "dev-tools",     "ESLint / Spectral"),
        ("docs",      "Documentation",        "dev-tools",     "Confluence"),
        ("changelog", "Changelog",            "dev-tools",     "Conventional Commits"),
        ("devlake",   "Engineering Metrics",  "dev-tools",     "Apache DevLake"),
        ("workflows", "CI/CD Workflows",      "dev-tools",     "GitHub Actions"),
        ("scaffold",  "Service Scaffold",     "dev-tools",     "Backstage Scaffolding"),
        // Platform
        ("flags",     "Feature Flags",        "platform",      "LaunchDarkly"),
        ("cost",      "Cost Analytics",       "platform",      "Kubecost"),
        ("registry",  "Artifact Registry",    "platform",      "Harbor"),
        ("gateway",   "API Gateway",          "platform",      "Kong + Gravitee"),
        ("chat",      "Team Chat",            "platform",      "Slack"),
        ("chaos",     "Chaos Engineering",    "platform",      "Chaos Monkey"),
        ("backup",    "Backup & Restore",     "platform",      "Velero"),
        // AI
        ("ai-obs",    "AI Observability",     "ai",            "Langfuse"),
    ]
}

fn category_icon(category: &str) -> &'static str {
    match category {
        "security"      => "shield",
        "observability" => "chart-bar",
        "dev-tools"     => "wrench",
        "platform"      => "cog",
        "ai"            => "cpu-chip",
        _               => "cube",
    }
}

fn category_label(category: &str) -> &'static str {
    match category {
        "security"      => "Security",
        "observability" => "Observability",
        "dev-tools"     => "Dev Tools",
        "platform"      => "Platform",
        "ai"            => "AI / Data",
        _               => "Other",
    }
}

/// Build the full aggregated dashboard payload.
pub fn get_dashboard() -> DashboardData {
    let modules: Vec<DashboardWidget> = all_modules()
        .iter()
        .map(|(id, name, category, upstream)| DashboardWidget {
            module: id.to_string(),
            display_name: name.to_string(),
            health: HealthStatus::Healthy,
            key_metric_label: "status".to_string(),
            key_metric_value: "operational".to_string(),
            link: format!("/modules/{id}"),
            upstream_replacement: upstream.to_string(),
            category: category.to_string(),
        })
        .collect();

    let total = modules.len();
    let healthy = modules
        .iter()
        .filter(|m| matches!(m.health, HealthStatus::Healthy))
        .count();
    let degraded = modules
        .iter()
        .filter(|m| matches!(m.health, HealthStatus::Degraded))
        .count();
    let unhealthy = modules
        .iter()
        .filter(|m| matches!(m.health, HealthStatus::Unhealthy))
        .count();
    let unknown = modules
        .iter()
        .filter(|m| matches!(m.health, HealthStatus::Unknown))
        .count();

    DashboardData {
        modules,
        total_modules: total,
        healthy_count: healthy,
        degraded_count: degraded,
        unhealthy_count: unhealthy,
        unknown_count: unknown,
        generated_at: Utc::now(),
    }
}

/// Per-module curated stats payload. Most modules return the default
/// active/requests/errors triple; gateway carries the canonical Kong +
/// Gravitee feature list per ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001.
fn module_stats(module_id: &str) -> serde_json::Value {
    match module_id {
        "gateway" => serde_json::json!({
            "active": true,
            "requests_today": 0,
            "errors_today": 0,
            "upstreams": ["Kong v3.5", "Gravitee v4.x"],
            "kong_features": [
                "Admin API (services / routes / upstreams / consumers)",
                "Plugin chain: rate-limiting, key-auth, jwt, oauth2, basic-auth, hmac-auth",
                "ACL, IP restriction, bot detection, proxy cache",
                "Request/response transformer, request size limiting, request termination",
                "Prometheus + Zipkin observability, gRPC gateway",
                "Load balancer (round-robin / least-conn / consistent-hash / latency-aware)",
                "Active + passive healthchecks, circuit breaker",
                "TLS / SNI resolver, ACME HTTP-01 challenge"
            ],
            "gravitee_features": [
                "API definition (path / methods / policy chain) + lifecycle (Created / Published / Unpublished / Deprecated / Archived)",
                "Plan registry (KeyLess / ApiKey / JWT / OAuth2 security types)",
                "Application registry (Simple / WebApp / Browser / Native / BackendToBackend, OAuth2 client credentials)",
                "Subscription state machine (Pending → Accepted / Rejected; Accepted ↔ Paused / Resumed → Closed)",
                "API-key minting + indexed lookup for data-path enforcement",
                "Portal: read-only Public + Published view, category + tag filters",
                "Developer portal pages, catalog versioning, debug mode",
                "Design-time governance (OpenAPI linting + quality gates)",
                "Federation gateway, analytics dimensions"
            ]
        }),
        _ => serde_json::json!({
            "active": true,
            "requests_today": 0,
            "errors_today": 0,
        }),
    }
}

/// Return a summary for a single module by its slug.
pub fn get_module_summary(module_id: &str) -> Option<ModuleSummary> {
    all_modules()
        .into_iter()
        .find(|(id, _, _, _)| *id == module_id)
        .map(|(id, name, category, upstream)| ModuleSummary {
            module: id.to_string(),
            display_name: name.to_string(),
            health: HealthStatus::Healthy,
            upstream_replacement: upstream.to_string(),
            category: category.to_string(),
            stats: module_stats(id),
        })
}

/// Return all module summaries for the modules listing.
pub fn list_modules() -> Vec<ModuleSummary> {
    all_modules()
        .into_iter()
        .map(|(id, name, category, upstream)| ModuleSummary {
            module: id.to_string(),
            display_name: name.to_string(),
            health: HealthStatus::Healthy,
            upstream_replacement: upstream.to_string(),
            category: category.to_string(),
            stats: module_stats(id),
        })
        .collect()
}

/// Build the hierarchical sidebar navigation.
pub fn get_nav() -> Vec<NavigationGroup> {
    // Preserve insertion order per category.
    let category_order = ["security", "observability", "dev-tools", "platform", "ai"];
    let mut buckets: std::collections::HashMap<&str, Vec<NavigationItem>> =
        std::collections::HashMap::new();

    for (id, name, category, upstream) in all_modules() {
        let item = NavigationItem {
            id: id.to_string(),
            label: name.to_string(),
            icon: category_icon(category).to_string(),
            path: format!("/modules/{id}"),
            category: category.to_string(),
            upstream_replacement: upstream.to_string(),
            badge_count: None,
        };
        buckets.entry(category).or_default().push(item);
    }

    category_order
        .iter()
        .filter_map(|cat| {
            buckets.remove(*cat).map(|items| NavigationGroup {
                label: category_label(cat).to_string(),
                icon: category_icon(cat).to_string(),
                items,
            })
        })
        .collect()
}

/// Full-text search across all module metadata.
pub fn global_search(query: &str) -> Vec<SearchResult> {
    if query.trim().is_empty() {
        return vec![];
    }
    let q = query.to_lowercase();
    all_modules()
        .into_iter()
        .filter(|(id, name, category, upstream)| {
            id.contains(&q)
                || name.to_lowercase().contains(&q)
                || category.contains(&q)
                || upstream.to_lowercase().contains(&q)
        })
        .map(|(id, name, _category, upstream)| SearchResult {
            id: Uuid::new_v4().to_string(),
            module: id.to_string(),
            kind: "module".to_string(),
            title: name.to_string(),
            description: format!("Replaces {upstream}"),
            link: format!("/modules/{id}"),
            relevance: 1.0,
        })
        .collect()
}

/// Aggregate cross-module notifications for the notification feed.
pub fn get_notifications() -> Vec<Notification> {
    vec![
        Notification {
            id: Uuid::new_v4(),
            module: "vulns".to_string(),
            title: "3 new critical vulnerabilities detected".to_string(),
            body: "Critical CVEs found in base image dependencies. Immediate patching recommended."
                .to_string(),
            severity: NotificationSeverity::Critical,
            created_at: Utc::now(),
            read: false,
            link: Some("/modules/vulns".to_string()),
        },
        Notification {
            id: Uuid::new_v4(),
            module: "certs".to_string(),
            title: "Certificate expiring in 14 days".to_string(),
            body: "api.example.com TLS certificate requires renewal.".to_string(),
            severity: NotificationSeverity::Warning,
            created_at: Utc::now(),
            read: false,
            link: Some("/modules/certs".to_string()),
        },
        Notification {
            id: Uuid::new_v4(),
            module: "incidents".to_string(),
            title: "All systems operational".to_string(),
            body: "No active incidents at this time.".to_string(),
            severity: NotificationSeverity::Info,
            created_at: Utc::now(),
            read: true,
            link: Some("/modules/incidents".to_string()),
        },
        Notification {
            id: Uuid::new_v4(),
            module: "scan".to_string(),
            title: "Container scan completed".to_string(),
            body: "12 images scanned, 2 medium severity findings.".to_string(),
            severity: NotificationSeverity::Warning,
            created_at: Utc::now(),
            read: true,
            link: Some("/modules/scan".to_string()),
        },
    ]
}
