// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Auto-breadcrumb from URL path.
//!
//! Given a request path like `/admin/keda/scaledobjects/foo`, produce
//! a list of `Crumb { label, href }`:
//!
//!     Home → Admin → KEDA → ScaledObjects → foo
//!
//! Pretty labels are looked up in a static map (`PRETTY_NAMES`);
//! anything missing falls back to the segment with `-` / `_`
//! replaced by spaces and Title Case.

use crate::admin::render::escape;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crumb {
    pub label: String,
    /// Absolute href for the link. `None` for the last crumb (current
    /// page — rendered as plain text, not a link).
    pub href: Option<String>,
}

/// Build a `Vec<Crumb>` from a path. Always starts with the
/// "Home" crumb pointing at `/`. The final crumb's `href` is `None`
/// so the template can render it un-linked.
pub fn breadcrumb_for_path(path: &str) -> Vec<Crumb> {
    let mut out = vec![Crumb {
        label: "Home".into(),
        href: Some("/".into()),
    }];

    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        // Just home; mark it un-linked so the renderer shows it as
        // the current page.
        out[0].href = None;
        return out;
    }

    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    let mut cumulative = String::new();
    let last = segments.len().saturating_sub(1);
    for (i, seg) in segments.iter().enumerate() {
        cumulative.push('/');
        cumulative.push_str(seg);
        out.push(Crumb {
            label: pretty_label(seg),
            href: if i == last { None } else { Some(cumulative.clone()) },
        });
    }
    out
}

fn pretty_label(segment: &str) -> String {
    if let Some(name) = PRETTY_NAMES.iter().find(|(k, _)| *k == segment) {
        return name.1.to_string();
    }
    // Fallback: replace separators with spaces and Title Case each
    // word.
    let cleaned: String = segment
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    let mut out = String::with_capacity(cleaned.len());
    let mut start_word = true;
    for c in cleaned.chars() {
        if start_word {
            for u in c.to_uppercase() {
                out.push(u);
            }
            start_word = false;
        } else if c == ' ' {
            out.push(c);
            start_word = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Hand-curated friendly labels. Add to this as new routes land.
pub const PRETTY_NAMES: &[(&str, &str)] = &[
    ("admin", "Admin"),
    ("adr", "ADR Browser"),
    ("compliance", "Compliance"),
    ("upstream", "Upstream"),
    ("keda", "KEDA"),
    ("vault", "Vault"),
    ("kubelet", "Kubelet"),
    ("scheduler", "Scheduler"),
    ("apiserver", "API Server"),
    ("controller-manager", "Controller Manager"),
    ("cri", "CRI"),
    ("etcd", "etcd"),
    ("cache", "Cache"),
    ("net", "Networking"),
    ("ccm", "Cloud Controller"),
    ("scaledobjects", "ScaledObjects"),
    ("scaledjobs", "ScaledJobs"),
    ("triggerauthentications", "TriggerAuth"),
    ("grafana", "Grafana"),
    ("prometheus", "Prometheus"),
    ("loki", "Loki"),
    ("tempo", "Tempo"),
    ("kiali", "Kiali"),
    ("alerts", "Alerts"),
    ("backup", "Backup"),
    ("incidents", "Incidents"),
    ("artifacts", "Artifacts"),
    ("workflows", "Workflows"),
    ("rdbms", "RDBMS"),
    ("docdb", "DocDB"),
    ("streams", "Streams"),
];

/// Render the breadcrumb trail as semantic `<nav>` markup.
/// Each crumb is `<a>` except the last, which is `<span aria-current="page">`.
pub fn render(crumbs: &[Crumb]) -> String {
    let mut out = String::new();
    out.push_str(
        r#"<nav class="text-xs text-zinc-500 dark:text-zinc-400 mb-3" aria-label="Breadcrumb"><ol class="flex flex-wrap items-center gap-1">"#,
    );
    let last = crumbs.len().saturating_sub(1);
    for (i, c) in crumbs.iter().enumerate() {
        out.push_str("<li class=\"flex items-center gap-1\">");
        if let Some(href) = &c.href {
            out.push_str(&format!(
                r#"<a href="{}" class="hover:text-blue-600 dark:hover:text-blue-300 underline-offset-2 hover:underline">{}</a>"#,
                escape(href),
                escape(&c.label),
            ));
        } else {
            out.push_str(&format!(
                r#"<span aria-current="page" class="text-zinc-900 dark:text-zinc-100 font-medium">{}</span>"#,
                escape(&c.label),
            ));
        }
        if i < last {
            out.push_str(r#"<span aria-hidden="true" class="text-zinc-400">/</span>"#);
        }
        out.push_str("</li>");
    }
    out.push_str("</ol></nav>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_just_home_unlinked() {
        let cs = breadcrumb_for_path("/");
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].label, "Home");
        assert!(cs[0].href.is_none());
    }

    #[test]
    fn admin_keda_resolves_pretty_names() {
        let cs = breadcrumb_for_path("/admin/keda");
        assert_eq!(cs.len(), 3);
        assert_eq!(cs[0].label, "Home");
        assert_eq!(cs[0].href.as_deref(), Some("/"));
        assert_eq!(cs[1].label, "Admin");
        assert_eq!(cs[1].href.as_deref(), Some("/admin"));
        assert_eq!(cs[2].label, "KEDA");
        assert!(cs[2].href.is_none()); // current page
    }

    #[test]
    fn unknown_segment_titlecases_and_replaces_separators() {
        let cs = breadcrumb_for_path("/admin/some-unknown-thing");
        assert_eq!(cs[2].label, "Some Unknown Thing");
    }

    #[test]
    fn nested_route_chain_carries_cumulative_hrefs() {
        let cs = breadcrumb_for_path("/admin/keda/scaledobjects/foo");
        assert_eq!(cs.len(), 5);
        assert_eq!(cs[3].label, "ScaledObjects");
        assert_eq!(cs[3].href.as_deref(), Some("/admin/keda/scaledobjects"));
        assert_eq!(cs[4].label, "Foo");
        assert!(cs[4].href.is_none());
    }

    #[test]
    fn render_emits_nav_role_and_aria_current() {
        let cs = breadcrumb_for_path("/admin/keda");
        let html = render(&cs);
        assert!(html.contains(r#"aria-label="Breadcrumb""#));
        assert!(html.contains(r#"aria-current="page""#));
        assert!(html.contains("Home"));
        assert!(html.contains("KEDA"));
    }

    #[test]
    fn render_escapes_user_supplied_segment_label() {
        // Path passed through; the renderer must escape it.
        let cs = breadcrumb_for_path("/admin/<script>");
        let html = render(&cs);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn trailing_slash_is_tolerated() {
        let cs = breadcrumb_for_path("/admin/keda/");
        assert_eq!(cs.last().unwrap().label, "KEDA");
    }
}
