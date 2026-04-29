//! cave-iceberg portal wrap.

use super::Wrap;

pub fn wrap() -> Wrap {
    Wrap {
        module: "cave-iceberg".to_string(),
        title: "Lakehouse Catalog (Apache Iceberg-compatible)".to_string(),
        upstream: "apache/iceberg".to_string(),
        upstream_commit: "c81534f".to_string(),
        description: "Iceberg REST-catalog HTTP server with optimistic-concurrency \
                      commits, snapshot management, and partition/schema evolution."
            .to_string(),
        dashboard_path: "observability/dashboards/cave-iceberg.json".to_string(),
        alerts_path: "observability/alerts/cave-iceberg.yml".to_string(),
        live_data_endpoint: "/v1/config".to_string(),
    }
}
