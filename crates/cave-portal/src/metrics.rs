// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portal metrics — `/account` + `/admin/auth` Prom counters.
//!
//! Self-contained AtomicU64 counters keyed by `(page|action, label)`.
//! Exposed via [`PortalMetrics::render_openmetrics`] which the existing
//! `/metrics` endpoint can stitch into its body. No external Prom
//! client dep — this matches the in-tree style used by the rest of
//! cave-portal.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Process-singleton counter set.
pub struct PortalMetrics {
    page_views: Mutex<HashMap<(String, String), AtomicU64>>,
    actions: Mutex<HashMap<(String, String), AtomicU64>>,
}

static GLOBAL: OnceLock<PortalMetrics> = OnceLock::new();

impl PortalMetrics {
    pub fn global() -> &'static PortalMetrics {
        GLOBAL.get_or_init(PortalMetrics::new)
    }

    fn new() -> Self {
        Self {
            page_views: Mutex::new(HashMap::new()),
            actions: Mutex::new(HashMap::new()),
        }
    }

    /// `cave_portal_auth_page_views_total{page=...,persona=...}`.
    pub fn incr_page_view(&self, page: &str, persona: &str) {
        let key = (page.to_string(), persona.to_string());
        let mut map = self.page_views.lock().unwrap();
        map.entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// `cave_portal_auth_actions_total{action=...,result=...}`.
    pub fn incr_action(&self, action: &str, result: &str) {
        let key = (action.to_string(), result.to_string());
        let mut map = self.actions.lock().unwrap();
        map.entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn page_view_count(&self, page: &str, persona: &str) -> u64 {
        let map = self.page_views.lock().unwrap();
        map.get(&(page.to_string(), persona.to_string()))
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn action_count(&self, action: &str, result: &str) -> u64 {
        let map = self.actions.lock().unwrap();
        map.get(&(action.to_string(), result.to_string()))
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Render counters in OpenMetrics / Prom text format.
    pub fn render_openmetrics(&self) -> String {
        let mut out = String::new();
        out.push_str("# TYPE cave_portal_auth_page_views_total counter\n");
        let pv = self.page_views.lock().unwrap();
        let mut entries: Vec<_> = pv.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for ((page, persona), v) in entries {
            out.push_str(&format!(
                "cave_portal_auth_page_views_total{{page=\"{}\",persona=\"{}\"}} {}\n",
                escape_label(page),
                escape_label(persona),
                v.load(Ordering::Relaxed),
            ));
        }
        out.push_str("# TYPE cave_portal_auth_actions_total counter\n");
        let ac = self.actions.lock().unwrap();
        let mut entries: Vec<_> = ac.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for ((action, result), v) in entries {
            out.push_str(&format!(
                "cave_portal_auth_actions_total{{action=\"{}\",result=\"{}\"}} {}\n",
                escape_label(action),
                escape_label(result),
                v.load(Ordering::Relaxed),
            ));
        }
        out
    }
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incr_page_view_accumulates() {
        let m = PortalMetrics::new();
        m.incr_page_view("personal_info", "user");
        m.incr_page_view("personal_info", "user");
        m.incr_page_view("personal_info", "admin");
        assert_eq!(m.page_view_count("personal_info", "user"), 2);
        assert_eq!(m.page_view_count("personal_info", "admin"), 1);
        assert_eq!(m.page_view_count("missing", "user"), 0);
    }

    #[test]
    fn incr_action_accumulates() {
        let m = PortalMetrics::new();
        m.incr_action("password_update", "success");
        m.incr_action("password_update", "validation_error");
        m.incr_action("password_update", "success");
        assert_eq!(m.action_count("password_update", "success"), 2);
        assert_eq!(m.action_count("password_update", "validation_error"), 1);
    }

    #[test]
    fn render_openmetrics_includes_type_headers_and_samples() {
        let m = PortalMetrics::new();
        m.incr_page_view("personal_info", "user");
        m.incr_action("password_update", "success");
        let text = m.render_openmetrics();
        assert!(text.contains("# TYPE cave_portal_auth_page_views_total counter"));
        assert!(text.contains("# TYPE cave_portal_auth_actions_total counter"));
        assert!(text.contains(
            r#"cave_portal_auth_page_views_total{page="personal_info",persona="user"} 1"#
        ));
        assert!(text.contains(
            r#"cave_portal_auth_actions_total{action="password_update",result="success"} 1"#
        ));
    }

    #[test]
    fn render_openmetrics_escapes_label_specials() {
        let m = PortalMetrics::new();
        m.incr_page_view(r#"weird"page"#, "user");
        let text = m.render_openmetrics();
        assert!(text.contains(r#"page="weird\"page""#));
    }

    #[test]
    fn global_singleton_is_shared() {
        let g1 = PortalMetrics::global();
        let g2 = PortalMetrics::global();
        assert!(std::ptr::eq(g1, g2));
    }
}
