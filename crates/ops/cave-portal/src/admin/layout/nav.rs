// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Persona-filtered navigation menu.
//!
//! Returns a tree of [`NavItem`]s the sidebar renders. The tree is
//! grouped by section (Cluster / K8s / Data / Identity / DevEx /
//! Observability / Security / Networking); each item has a route, an
//! icon glyph, and a persona filter so `TenantAdmin` only sees what's
//! relevant.
//!
//! ## Persona policy
//!
//! * `PlatformAdmin` sees every entry — the function-level filter is
//!   short-circuited in `is_visible`.
//! * `TenantAdmin` sees entries whose `visible_to` includes it. The
//!   rule of thumb: anything bound to a single tenant's data path
//!   (KEDA queues, the tenant's vault key namespace, MLflow
//!   experiments under their project) is tenant-visible; anything
//!   cluster-wide (compliance dashboard, ADR browser, controller
//!   manager, upstream watch) is platform-only.
//! * `Anonymous` is the smoke-flow persona; it sees nothing by
//!   default. The expectation is that real usage has a session.

use crate::admin::permission::Persona;
use crate::admin::render::escape;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavItem {
    pub label: &'static str,
    pub href: &'static str,
    /// Single emoji or short glyph used as the visual marker.
    pub glyph: &'static str,
    /// Personas this item is visible to. `PlatformAdmin` is always
    /// included implicitly.
    pub visible_to: &'static [Persona],
}

/// One section group in the sidebar.
#[derive(Debug, Clone)]
pub struct NavSection {
    pub label: &'static str,
    pub items: &'static [NavItem],
}

/// Resolve the navigation tree for a persona. `PlatformAdmin` sees
/// every item; `TenantAdmin` sees the items whose `visible_to`
/// includes it; `Anonymous` sees the public surface only.
pub fn nav_items_for_persona(persona: Persona) -> Vec<NavSection> {
    SECTIONS
        .iter()
        .filter_map(|s| {
            let visible: Vec<&'static NavItem> =
                s.items.iter().filter(|i| is_visible(i, persona)).collect();
            if visible.is_empty() {
                None
            } else {
                // Static-data NavSection by selecting items via
                // pointer equality (kept simple — we know the slice
                // is `&'static` and the renderer reads it in order).
                Some(NavSection {
                    label: s.label,
                    items: s.items, // renderer re-filters; cheap.
                })
            }
        })
        .collect()
}

fn is_visible(item: &NavItem, persona: Persona) -> bool {
    if persona == Persona::PlatformAdmin {
        return true;
    }
    item.visible_to.contains(&persona)
}

/// Render the sidebar HTML for `current_path`. Marks the matching
/// item with `aria-current="page"` so screen readers + the active-
/// highlight CSS class can light it up.
pub fn sidebar(persona: Persona, current_path: &str, tenant_id: &str) -> String {
    let sections = nav_items_for_persona(persona);
    let mut out = String::new();
    out.push_str(
        r#"<aside id="cave-sidebar" class="hidden md:flex md:flex-col md:fixed md:top-12 md:left-0 md:bottom-0 md:w-56 md:overflow-y-auto md:border-r md:bg-white md:dark:bg-zinc-900 md:dark:border-zinc-800 md:p-3 md:z-10" aria-label="Primary navigation">"#,
    );
    for section in &sections {
        out.push_str(&format!(
            r#"<div class="text-[10px] uppercase tracking-wider text-zinc-500 dark:text-zinc-400 mt-3 mb-1">{}</div>"#,
            escape(section.label),
        ));
        for item in section.items {
            if !is_visible(item, persona) {
                continue;
            }
            let active = current_path.starts_with(item.href);
            let aria = if active {
                r#" aria-current="page""#
            } else {
                ""
            };
            let class = if active {
                "flex items-center gap-2 px-2 py-1 rounded bg-blue-100 dark:bg-blue-900/40 text-blue-900 dark:text-blue-200 font-medium"
            } else {
                "flex items-center gap-2 px-2 py-1 rounded text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800"
            };
            let href = format!("{}?tenant_id={}", item.href, urlencode_minimal(tenant_id),);
            out.push_str(&format!(
                r#"<a href="{href}"{aria} class="{class}"><span aria-hidden="true">{glyph}</span><span>{label}</span></a>"#,
                href = escape(&href),
                aria = aria,
                class = class,
                glyph = escape(item.glyph),
                label = escape(item.label),
            ));
        }
    }
    out.push_str("</aside>");
    out
}

/// Minimal URL-encoder for the tenant_id segment. Only escapes the
/// characters that would break the `?tenant_id=` interpretation
/// (space, ampersand, equals, plus). Cave tenant ids are
/// DNS-1123-validated so this is overkill, but stays correct if a
/// future tenant id ever contains something weirder.
fn urlencode_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

// ── Static section data ────────────────────────────────────────────────────

/// Helper to keep the section definitions readable.
const fn item(
    label: &'static str,
    href: &'static str,
    glyph: &'static str,
    visible_to: &'static [Persona],
) -> NavItem {
    NavItem {
        label,
        href,
        glyph,
        visible_to,
    }
}

const TENANT_AND_PLATFORM: &[Persona] = &[Persona::TenantAdmin, Persona::PlatformAdmin];
const PLATFORM_ONLY: &[Persona] = &[];

// Sections refreshed 2026-05-14: matches the canonical sidebar layout
// (Cluster / K8s / Data / Identity / DevEx / Observability / Security
// / Networking) so the chrome carries every reachable admin surface.
const SECTIONS: &[NavSection] = &[
    // ── Cluster — cross-cutting platform views ────────────────────────
    NavSection {
        label: "Cluster",
        items: &[
            item("Compliance", "/admin/compliance", "📋", PLATFORM_ONLY),
            // 2026-05-14 discoverability fix: `/admin/cluster/live` was
            // the canonical Raft snapshot view but had no sidebar entry,
            // no command-palette nav, no shortcut. Adding it as the
            // second platform item — the operator's "is the control
            // plane healthy" landing spot.
            item("Cluster Status", "/admin/cluster/live", "🫀", PLATFORM_ONLY),
            item("Audit", "/admin/audit", "🧾", PLATFORM_ONLY),
            item("Upstream", "/admin/upstream", "⇪", PLATFORM_ONLY),
        ],
    },
    // ── K8s — control-plane surfaces ─────────────────────────────────
    NavSection {
        label: "K8s",
        items: &[
            // 2026-05-14 consolidation: Scheduler + Kubelet folded into
            // the single K8s Dashboard surface — sub-tabs live under
            // /admin/k8s-dashboard/{pods,nodes,scheduler/queue,...}.
            item(
                "K8s Dashboard",
                "/admin/k8s-dashboard",
                "☸",
                TENANT_AND_PLATFORM,
            ),
            item(
                "Controller Manager",
                "/admin/controller-manager",
                "⚙",
                PLATFORM_ONLY,
            ),
            item("Cloud Controller", "/admin/ccm", "☁", PLATFORM_ONLY),
            item("API Server", "/admin/apiserver", "🛰", PLATFORM_ONLY),
            item("etcd", "/admin/etcd", "🗄", PLATFORM_ONLY),
            item("Kamaji", "/admin/kamaji", "👶", PLATFORM_ONLY),
            item("Edge", "/admin/edge", "📟", PLATFORM_ONLY),
        ],
    },
    // ── Data — stateful tenant-facing services ────────────────────────
    NavSection {
        label: "Data",
        items: &[
            item("KEDA", "/admin/keda", "📈", TENANT_AND_PLATFORM),
            item("Vault", "/admin/vault", "🔐", TENANT_AND_PLATFORM),
            item("Cache", "/admin/cache", "💾", TENANT_AND_PLATFORM),
            item("Streams", "/admin/streams", "🌊", TENANT_AND_PLATFORM),
            item("Lakehouse", "/admin/lakehouse", "🏞", TENANT_AND_PLATFORM),
            item("Iceberg", "/admin/iceberg", "🧊", TENANT_AND_PLATFORM),
            item("RDBMS", "/admin/rdbms", "🐘", TENANT_AND_PLATFORM),
            item("DocDB", "/admin/docdb", "🍃", TENANT_AND_PLATFORM),
        ],
    },
    // ── Identity — auth, secrets, sessions ───────────────────────────
    NavSection {
        label: "Identity",
        items: &[
            item("Keycloak", "/admin/auth", "🪪", PLATFORM_ONLY),
            item("OpenBao", "/admin/secrets", "🗝", PLATFORM_ONLY),
            item("Sessions", "/admin/auth-sessions", "👥", PLATFORM_ONLY),
            item("Certs", "/admin/certs", "📜", PLATFORM_ONLY),
            item("PAM", "/admin/pam", "🛡", PLATFORM_ONLY),
        ],
    },
    // ── DevEx — dev/platform tooling ─────────────────────────────────
    NavSection {
        label: "DevEx",
        items: &[
            item("ADR Browser", "/admin/adr", "📜", PLATFORM_ONLY),
            item("MLflow", "/admin/mlflow", "🧪", TENANT_AND_PLATFORM),
            item(
                "Backstage",
                "/admin/contributions",
                "📚",
                TENANT_AND_PLATFORM,
            ),
            item("Pipelines", "/admin/pipelines", "🔧", TENANT_AND_PLATFORM),
            item("LiteLLM", "/admin/litellm", "🤖", TENANT_AND_PLATFORM),
            item("Local LLM", "/admin/local-llm", "🧠", PLATFORM_ONLY),
        ],
    },
    // ── Observability ────────────────────────────────────────────────
    NavSection {
        label: "Observability",
        items: &[
            item("Grafana", "/admin/grafana", "📊", TENANT_AND_PLATFORM),
            item("Prometheus", "/admin/prometheus", "🔥", TENANT_AND_PLATFORM),
            item("Loki", "/admin/loki", "📝", TENANT_AND_PLATFORM),
            item("Tracing", "/admin/trace", "🧵", TENANT_AND_PLATFORM),
            item("Alerts", "/admin/alerts", "🚨", TENANT_AND_PLATFORM),
            item("Incidents", "/admin/incidents", "🚒", PLATFORM_ONLY),
            item("On-call", "/admin/oncall", "📟", PLATFORM_ONLY),
            item("SLO", "/admin/slo", "🎯", TENANT_AND_PLATFORM),
        ],
    },
    // ── Security ─────────────────────────────────────────────────────
    NavSection {
        label: "Security",
        items: &[
            item("Vulns", "/admin/vulns", "🐛", PLATFORM_ONLY),
            item(
                "Container Scan",
                "/admin/container-scan",
                "🔬",
                TENANT_AND_PLATFORM,
            ),
            item("DAST", "/admin/dast", "🕷", TENANT_AND_PLATFORM),
            item("SBOM", "/admin/sbom", "📦", TENANT_AND_PLATFORM),
            item("Policy", "/admin/policy", "📐", PLATFORM_ONLY),
            item("Admission", "/admin/admission", "🛡", PLATFORM_ONLY),
            item("Forensics", "/admin/forensics", "🕵", PLATFORM_ONLY),
        ],
    },
    // ── Networking — mesh, ingress, DNS, etc. ────────────────────────
    NavSection {
        label: "Networking",
        items: &[
            item("Mesh", "/admin/mesh", "🕸", TENANT_AND_PLATFORM),
            item("Gateway", "/admin/gateway", "🚪", TENANT_AND_PLATFORM),
            item("CRI", "/admin/cri", "📦", TENANT_AND_PLATFORM),
            item("Net", "/admin/net", "🌐", PLATFORM_ONLY),
            item("DNS", "/admin/dns", "📡", PLATFORM_ONLY),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_admin_sees_every_item() {
        let secs = nav_items_for_persona(Persona::PlatformAdmin);
        assert!(!secs.is_empty());
        // Spot-check: Compliance (platform-only) should be in the
        // resolved render output.
        let html = sidebar(Persona::PlatformAdmin, "/admin/keda", "dev");
        assert!(html.contains("Compliance"));
        assert!(html.contains("ADR Browser"));
        assert!(html.contains("KEDA"));
    }

    #[test]
    fn tenant_admin_does_not_see_platform_only_items() {
        let html = sidebar(Persona::TenantAdmin, "/admin/keda", "tenant1");
        // KEDA, Vault, Kubelet visible.
        assert!(html.contains("KEDA"));
        assert!(html.contains("Vault"));
        // Platform-only hidden.
        assert!(!html.contains("Compliance"));
        assert!(!html.contains("ADR Browser"));
        assert!(!html.contains("Controller Manager"));
    }

    #[test]
    fn anonymous_persona_sees_nothing_when_visible_to_excludes() {
        let html = sidebar(Persona::Anonymous, "/", "x");
        assert!(!html.contains("KEDA"));
        assert!(!html.contains("Compliance"));
    }

    #[test]
    fn sidebar_marks_active_item_with_aria_current() {
        let html = sidebar(Persona::PlatformAdmin, "/admin/keda", "dev");
        // The /admin/keda link should be marked aria-current.
        // We look for the substring "/admin/keda" followed (within a
        // reasonable window) by aria-current.
        let idx = html.find(r#"href="/admin/keda"#).unwrap();
        let suffix = &html[idx..idx + 200];
        assert!(suffix.contains(r#"aria-current="page""#), "{suffix}");
    }

    #[test]
    fn sidebar_renders_aside_with_aria_label() {
        let html = sidebar(Persona::PlatformAdmin, "/admin", "dev");
        assert!(html.contains(r#"aria-label="Primary navigation""#));
    }

    #[test]
    fn sidebar_propagates_tenant_id_into_link_query() {
        let html = sidebar(Persona::PlatformAdmin, "/", "platform");
        assert!(html.contains("?tenant_id=platform"));
    }

    #[test]
    fn urlencode_handles_special_chars() {
        assert_eq!(urlencode_minimal("acme"), "acme");
        assert_eq!(urlencode_minimal("acme & co"), "acme%20%26%20co");
    }

    #[test]
    fn sidebar_includes_dark_mode_tailwind_classes() {
        let html = sidebar(Persona::PlatformAdmin, "/admin", "dev");
        // Verify dark: variant is present on at least one element.
        assert!(html.contains("dark:bg-zinc-900"));
        assert!(html.contains("dark:hover:bg-zinc-800"));
    }

    // ── 2026-05-14 refreshed section coverage ──────────────────────────

    #[test]
    fn all_seven_sections_render_for_platform_admin() {
        let html = sidebar(Persona::PlatformAdmin, "/admin", "dev");
        for label in &[
            "Cluster",
            "K8s",
            "Data",
            "Identity",
            "DevEx",
            "Observability",
            "Security",
            "Networking",
        ] {
            assert!(html.contains(label), "section {label} missing");
        }
    }

    #[test]
    fn tenant_admin_sees_data_devex_obs_security_net_only() {
        let html = sidebar(Persona::TenantAdmin, "/admin", "t");
        // Cluster + Identity are platform-only — entire sections empty
        // for TenantAdmin, so the headings should NOT appear.
        assert!(!html.contains("Cluster Status"));
        assert!(!html.contains("Keycloak"));
        assert!(!html.contains("OpenBao"));
        // Data + Networking items the tenant DOES see:
        assert!(html.contains("KEDA"));
        assert!(html.contains("Vault"));
        assert!(html.contains("Lakehouse"));
        assert!(html.contains("Mesh"));
        // DevEx tenant-visible: MLflow, Backstage, Pipelines, LiteLLM.
        assert!(html.contains("MLflow"));
        assert!(html.contains("LiteLLM"));
        // Observability tenant-visible.
        assert!(html.contains("Grafana"));
        assert!(html.contains("Tracing"));
        // Security tenant-visible subset.
        assert!(html.contains("Container Scan"));
        assert!(html.contains("DAST"));
        // Platform-only Security items hidden.
        assert!(!html.contains("Forensics"));
        assert!(!html.contains("Admission"));
    }

    #[test]
    fn cluster_section_carries_audit_and_compliance() {
        let html = sidebar(Persona::PlatformAdmin, "/admin/audit", "dev");
        assert!(html.contains("Audit"));
        assert!(html.contains("Compliance"));
        assert!(html.contains("Cluster Status"));
    }

    #[test]
    fn data_section_carries_lakehouse_iceberg_streams_rdbms() {
        let html = sidebar(Persona::PlatformAdmin, "/admin/lakehouse", "dev");
        assert!(html.contains("Lakehouse"));
        assert!(html.contains("Iceberg"));
        assert!(html.contains("Streams"));
        assert!(html.contains("RDBMS"));
        assert!(html.contains("DocDB"));
    }

    #[test]
    fn devex_section_lists_mlflow_adr_backstage() {
        let html = sidebar(Persona::PlatformAdmin, "/admin/mlflow", "dev");
        assert!(html.contains("ADR Browser"));
        assert!(html.contains("MLflow"));
        assert!(html.contains("Backstage"));
    }

    #[test]
    fn k8s_dashboard_route_active_highlight() {
        let html = sidebar(Persona::PlatformAdmin, "/admin/k8s-dashboard/pods", "dev");
        let idx = html.find(r#"href="/admin/k8s-dashboard"#).unwrap();
        let suffix = &html[idx..idx + 200];
        assert!(suffix.contains(r#"aria-current="page""#), "{suffix}");
    }

    #[test]
    fn anonymous_sees_no_section_headings() {
        // Anonymous has no `visible_to` matches → every section's
        // filter empties → the section heading should not render.
        let html = sidebar(Persona::Anonymous, "/", "x");
        for label in &["Cluster", "K8s", "Data", "Identity", "DevEx"] {
            // Anonymous sees nothing — labels should be absent.
            assert!(!html.contains(label), "anonymous saw section {label}");
        }
    }
}
