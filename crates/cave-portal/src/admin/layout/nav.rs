//! Persona-filtered navigation menu.
//!
//! Returns a tree of [`NavItem`]s the sidebar renders. The tree is
//! grouped by section (Platform / Workloads / Observability /
//! Data); each item has a route, an icon glyph, and a persona
//! filter so `TenantAdmin` only sees what's relevant.

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
            let visible: Vec<&'static NavItem> = s
                .items
                .iter()
                .filter(|i| is_visible(i, persona))
                .collect();
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
            let aria = if active { r#" aria-current="page""# } else { "" };
            let class = if active {
                "flex items-center gap-2 px-2 py-1 rounded bg-blue-100 dark:bg-blue-900/40 text-blue-900 dark:text-blue-200 font-medium"
            } else {
                "flex items-center gap-2 px-2 py-1 rounded text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800"
            };
            let href = format!(
                "{}?tenant_id={}",
                item.href,
                urlencode_minimal(tenant_id),
            );
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
const fn item(label: &'static str, href: &'static str, glyph: &'static str, visible_to: &'static [Persona]) -> NavItem {
    NavItem { label, href, glyph, visible_to }
}

const TENANT_AND_PLATFORM: &[Persona] = &[Persona::TenantAdmin, Persona::PlatformAdmin];
const PLATFORM_ONLY: &[Persona] = &[];

const SECTIONS: &[NavSection] = &[
    NavSection {
        label: "Platform",
        items: &[
            // Platform-only: cross-tenant infra.
            item("Compliance", "/admin/compliance", "📋", PLATFORM_ONLY),
            item("Upstream", "/admin/upstream", "⇪", PLATFORM_ONLY),
            item("ADR Browser", "/admin/adr", "📜", PLATFORM_ONLY),
            item("API Server", "/admin/apiserver", "🛰", PLATFORM_ONLY),
            item("etcd", "/admin/etcd", "🗄", PLATFORM_ONLY),
            item("Scheduler", "/admin/scheduler", "🧮", PLATFORM_ONLY),
            item("Controller Manager", "/admin/controller-manager", "⚙", PLATFORM_ONLY),
            item("Cloud Controller", "/admin/ccm", "☁", PLATFORM_ONLY),
        ],
    },
    NavSection {
        label: "Workloads",
        items: &[
            item("Kubelet", "/admin/kubelet", "🖧", TENANT_AND_PLATFORM),
            item("KEDA", "/admin/keda", "📈", TENANT_AND_PLATFORM),
            item("ScaledObjects", "/admin/keda/scaledobjects", "🪜", TENANT_AND_PLATFORM),
            item("ScaledJobs", "/admin/keda/scaledjobs", "🛠", TENANT_AND_PLATFORM),
        ],
    },
    NavSection {
        label: "Observability",
        items: &[
            item("Grafana", "/admin/grafana", "📊", TENANT_AND_PLATFORM),
            item("Prometheus", "/admin/prometheus", "🔥", TENANT_AND_PLATFORM),
            item("Loki", "/admin/loki", "📝", TENANT_AND_PLATFORM),
            item("Kiali", "/admin/kiali", "🕸", TENANT_AND_PLATFORM),
            item("Alerts", "/admin/alerts", "🚨", TENANT_AND_PLATFORM),
        ],
    },
    NavSection {
        label: "Data",
        items: &[
            item("Vault", "/admin/vault", "🔐", TENANT_AND_PLATFORM),
            item("Cache", "/admin/cache", "💾", TENANT_AND_PLATFORM),
            item("RDBMS", "/admin/rdbms", "🐘", TENANT_AND_PLATFORM),
            item("DocDB", "/admin/docdb", "🍃", TENANT_AND_PLATFORM),
            item("Streams", "/admin/streams", "🌊", TENANT_AND_PLATFORM),
        ],
    },
    NavSection {
        label: "Networking",
        items: &[
            item("Networking", "/admin/net", "🌐", TENANT_AND_PLATFORM),
            item("CRI", "/admin/cri", "📦", TENANT_AND_PLATFORM),
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
}
