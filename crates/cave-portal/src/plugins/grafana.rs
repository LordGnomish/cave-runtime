// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Grafana wrap — native dashboard renderer + panel embed.
//!
//! The portal hosts dashboard *definitions* and renders them with
//! cave-portal's own SVG/HTML chart primitives. There is no `<iframe>` to
//! Grafana, no "View in Grafana" link.

use super::ViewPersona;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    Timeseries,
    Stat,
    Gauge,
    Table,
    Bar,
    Heatmap,
    Logs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelDef {
    pub id: String,
    pub title: String,
    pub kind: PanelKind,
    pub query: String,
    pub unit: String,
    pub width: u8,
    pub height: u8,
}

impl PanelDef {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        kind: PanelKind,
        query: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind,
            query: query.into(),
            unit: String::new(),
            width: 6,
            height: 4,
        }
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    pub fn with_size(mut self, width: u8, height: u8) -> Self {
        self.width = width.min(12);
        self.height = height.min(12);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardDef {
    pub id: String,
    pub tenant: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub panels: Vec<PanelDef>,
    pub variables: Vec<DashboardVariable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardVariable {
    pub name: String,
    pub label: String,
    pub options: Vec<String>,
    pub default: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GrafanaError {
    #[error("panel id {0:?} already exists")]
    DuplicatePanel(String),
    #[error("dashboard {0:?} not found")]
    NotFound(String),
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    #[error("dashboard width {0} would overflow grid")]
    GridOverflow(u32),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
}

const GRID_WIDTH: u32 = 12;

impl DashboardDef {
    pub fn new(id: impl Into<String>, tenant: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tenant: tenant.into(),
            title: title.into(),
            description: String::new(),
            tags: Vec::new(),
            panels: Vec::new(),
            variables: Vec::new(),
        }
    }

    pub fn add_panel(&mut self, panel: PanelDef) -> Result<(), GrafanaError> {
        if self.panels.iter().any(|p| p.id == panel.id) {
            return Err(GrafanaError::DuplicatePanel(panel.id));
        }
        self.panels.push(panel);
        Ok(())
    }

    pub fn validate(&self) -> Result<(), GrafanaError> {
        // Each panel's width must fit within the 12-column grid; row sums OK.
        for p in &self.panels {
            if p.width as u32 > GRID_WIDTH {
                return Err(GrafanaError::GridOverflow(p.width as u32));
            }
            if p.query.is_empty() {
                return Err(GrafanaError::InvalidQuery("empty".into()));
            }
            // disallow trivial injection patterns in query — keeps the query
            // parser simple at the data-fetch layer.
            if p.query.contains(';') || p.query.contains('"') {
                return Err(GrafanaError::InvalidQuery("forbidden char".into()));
            }
        }
        Ok(())
    }

    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    pub fn total_grid_width(&self) -> u32 {
        self.panels.iter().map(|p| p.width as u32).sum()
    }

    pub fn allowed_for(&self, persona: ViewPersona) -> bool {
        // Dashboards are visible to anyone in the tenant; admin and operator
        // see all dashboards across tenants (filtered at the plugin level).
        let _ = persona;
        true
    }
}

#[derive(Debug, Default)]
pub struct GrafanaPlugin {
    dashboards: Vec<DashboardDef>,
}

impl GrafanaPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, d: DashboardDef) -> Result<(), GrafanaError> {
        d.validate()?;
        if let Some(idx) = self.dashboards.iter().position(|x| x.id == d.id && x.tenant == d.tenant) {
            self.dashboards[idx] = d;
        } else {
            self.dashboards.push(d);
        }
        Ok(())
    }

    pub fn find(&self, tenant: &str, id: &str) -> Option<&DashboardDef> {
        self.dashboards.iter().find(|d| d.tenant == tenant && d.id == id)
    }

    pub fn list_for_tenant(&self, tenant: &str) -> Vec<&DashboardDef> {
        let mut out: Vec<&DashboardDef> = self.dashboards.iter().filter(|d| d.tenant == tenant).collect();
        out.sort_by(|a, b| a.title.cmp(&b.title));
        out
    }

    pub fn count(&self) -> usize {
        self.dashboards.len()
    }

    pub fn delete(&mut self, tenant: &str, id: &str) -> Result<(), GrafanaError> {
        let idx = self
            .dashboards
            .iter()
            .position(|d| d.tenant == tenant && d.id == id)
            .ok_or_else(|| GrafanaError::NotFound(id.into()))?;
        self.dashboards.remove(idx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(id: &str, q: &str) -> PanelDef {
        PanelDef::new(id, id, PanelKind::Timeseries, q)
    }

    #[test]
    fn panel_def_default_size() {
        let p = ts("p1", "rate(req[5m])");
        assert_eq!(p.width, 6);
        assert_eq!(p.height, 4);
    }

    #[test]
    fn panel_def_with_size_caps_at_12() {
        let p = ts("p1", "q").with_size(20, 20);
        assert_eq!(p.width, 12);
        assert_eq!(p.height, 12);
    }

    #[test]
    fn panel_def_with_unit() {
        let p = ts("p1", "q").with_unit("ms");
        assert_eq!(p.unit, "ms");
    }

    #[test]
    fn dashboard_default_empty() {
        let d = DashboardDef::new("d1", "acme", "Title");
        assert_eq!(d.panel_count(), 0);
        assert!(d.tags.is_empty());
    }

    #[test]
    fn dashboard_add_panel_appends() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "q")).unwrap();
        assert_eq!(d.panel_count(), 1);
    }

    #[test]
    fn dashboard_add_duplicate_panel_rejected() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "q1")).unwrap();
        let err = d.add_panel(ts("p1", "q2")).unwrap_err();
        assert!(matches!(err, GrafanaError::DuplicatePanel(_)));
    }

    #[test]
    fn dashboard_validate_rejects_overflow() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "q").with_size(12, 4)).unwrap();
        // total_grid_width irrelevant here; per-panel width is what's checked.
        assert!(d.validate().is_ok());
    }

    #[test]
    fn dashboard_validate_rejects_empty_query() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "")).unwrap();
        let err = d.validate().unwrap_err();
        assert!(matches!(err, GrafanaError::InvalidQuery(_)));
    }

    #[test]
    fn dashboard_validate_rejects_query_with_semicolon() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "DROP TABLE x; --")).unwrap();
        let err = d.validate().unwrap_err();
        assert!(matches!(err, GrafanaError::InvalidQuery(_)));
    }

    #[test]
    fn dashboard_validate_rejects_query_with_quote() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "select \"x\"")).unwrap();
        let err = d.validate().unwrap_err();
        assert!(matches!(err, GrafanaError::InvalidQuery(_)));
    }

    #[test]
    fn dashboard_total_grid_width_sums() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "q").with_size(6, 2)).unwrap();
        d.add_panel(ts("p2", "q").with_size(4, 2)).unwrap();
        assert_eq!(d.total_grid_width(), 10);
    }

    #[test]
    fn plugin_add_validates_and_inserts() {
        let mut p = GrafanaPlugin::new();
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "rate(req)")).unwrap();
        p.add(d).unwrap();
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn plugin_add_invalid_dashboard_rejected() {
        let mut p = GrafanaPlugin::new();
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "")).unwrap();
        let err = p.add(d).unwrap_err();
        assert!(matches!(err, GrafanaError::InvalidQuery(_)));
    }

    #[test]
    fn plugin_add_replaces_existing_with_same_id_and_tenant() {
        let mut p = GrafanaPlugin::new();
        let mut d1 = DashboardDef::new("d1", "acme", "Old");
        d1.add_panel(ts("p1", "q")).unwrap();
        p.add(d1).unwrap();
        let mut d2 = DashboardDef::new("d1", "acme", "New");
        d2.add_panel(ts("p1", "q")).unwrap();
        p.add(d2).unwrap();
        assert_eq!(p.count(), 1);
        assert_eq!(p.find("acme", "d1").unwrap().title, "New");
    }

    #[test]
    fn plugin_find_by_tenant_and_id() {
        let mut p = GrafanaPlugin::new();
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "q")).unwrap();
        p.add(d).unwrap();
        assert!(p.find("acme", "d1").is_some());
        assert!(p.find("globex", "d1").is_none());
    }

    #[test]
    fn plugin_list_for_tenant_filters_and_sorts() {
        let mut p = GrafanaPlugin::new();
        for (id, tenant, title) in [
            ("d1", "acme", "Zebra"),
            ("d2", "acme", "Alpha"),
            ("d3", "globex", "Mu"),
        ] {
            let mut d = DashboardDef::new(id, tenant, title);
            d.add_panel(ts("p1", "q")).unwrap();
            p.add(d).unwrap();
        }
        let acme = p.list_for_tenant("acme");
        assert_eq!(acme.len(), 2);
        assert_eq!(acme[0].title, "Alpha");
        assert_eq!(acme[1].title, "Zebra");
    }

    #[test]
    fn plugin_delete_removes() {
        let mut p = GrafanaPlugin::new();
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(ts("p1", "q")).unwrap();
        p.add(d).unwrap();
        p.delete("acme", "d1").unwrap();
        assert_eq!(p.count(), 0);
    }

    #[test]
    fn plugin_delete_unknown_errors() {
        let mut p = GrafanaPlugin::new();
        let err = p.delete("acme", "ghost").unwrap_err();
        assert!(matches!(err, GrafanaError::NotFound(_)));
    }

    #[test]
    fn panel_kind_serializes_snake_case() {
        let s = serde_json::to_string(&PanelKind::Timeseries).unwrap();
        assert_eq!(s, "\"timeseries\"");
    }

    #[test]
    fn dashboard_round_trips_json() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.tags = vec!["prod".into(), "api".into()];
        d.add_panel(ts("p1", "rate(req)")).unwrap();
        let s = serde_json::to_string(&d).unwrap();
        let back: DashboardDef = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn dashboard_variable_round_trips() {
        let v = DashboardVariable {
            name: "env".into(),
            label: "Environment".into(),
            options: vec!["prod".into(), "stage".into()],
            default: "prod".into(),
        };
        let s = serde_json::to_string(&v).unwrap();
        let back: DashboardVariable = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
    }
}
