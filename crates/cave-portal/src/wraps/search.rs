//! cave-search portal wrap.

use super::Wrap;

pub fn wrap() -> Wrap {
    Wrap {
        module: "cave-search".to_string(),
        title: "Search (OpenSearch-compatible)".to_string(),
        upstream: "opensearch-project/OpenSearch".to_string(),
        upstream_commit: "96bb22d".to_string(),
        description: "OpenSearch-wire-compatible HTTP search server with JSON \
                      Query DSL, mappings, aggregations, and a single-node cluster."
            .to_string(),
        dashboard_path: "observability/dashboards/cave-search.json".to_string(),
        alerts_path: "observability/alerts/cave-search.yml".to_string(),
        live_data_endpoint: "/_cluster/health".to_string(),
    }
}
