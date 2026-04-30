use crate::ui::{MetricCard, Panel, Table};

/// First real screen — mirrors the cluster overview tile of the web portal.
///
/// Composition only; data is fetched from `cave-runtime`'s HTTP API by the
/// caller and pushed into these primitives. No backend wiring yet.
pub struct ClusterOverview {
    pub title_panel: Panel,
    pub headline_metrics: Vec<MetricCard>,
    pub node_table: Table,
}

impl ClusterOverview {
    pub fn new() -> Self {
        Self {
            title_panel: Panel::new("Cluster Overview"),
            headline_metrics: vec![
                MetricCard::new("Nodes", "0"),
                MetricCard::new("Pods", "0"),
                MetricCard::new("CPU Used", "0").with_unit("%"),
                MetricCard::new("Memory Used", "0").with_unit("%"),
            ],
            node_table: Table::new(vec![
                "Node".into(),
                "Status".into(),
                "CPU".into(),
                "Memory".into(),
                "Pods".into(),
            ]),
        }
    }

    // TODO(adr-portal-desktop-001): wire to /api/apiserver/* and /api/kubelet/*
    // once the desktop has an HTTP client. Endpoints already exist on cave-runtime.
}

impl Default for ClusterOverview {
    fn default() -> Self {
        Self::new()
    }
}
