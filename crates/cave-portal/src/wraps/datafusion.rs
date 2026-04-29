//! cave-datafusion portal wrap.

use super::Wrap;

pub fn wrap() -> Wrap {
    Wrap {
        module: "cave-datafusion".to_string(),
        title: "Embedded Query Engine (Apache DataFusion-compatible)".to_string(),
        upstream: "apache/datafusion".to_string(),
        upstream_commit: "3aefba7".to_string(),
        description: "Embedded SQL query engine with logical/physical plan \
                      layering, sqlparser frontend, and an in-memory aggregation runtime."
            .to_string(),
        dashboard_path: "observability/dashboards/cave-datafusion.json".to_string(),
        alerts_path: "observability/alerts/cave-datafusion.yml".to_string(),
        live_data_endpoint: "/api/datafusion/stats".to_string(),
    }
}
