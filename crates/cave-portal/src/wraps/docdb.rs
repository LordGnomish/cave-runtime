//! cave-docdb portal wrap.

use super::Wrap;

pub fn wrap() -> Wrap {
    Wrap {
        module: "cave-docdb".to_string(),
        title: "Document DB (MongoDB-compatible)".to_string(),
        upstream: "mongodb/mongo".to_string(),
        upstream_commit: "d5cfd3d".to_string(),
        description: "MongoDB OP_MSG wire-compatible document database with \
                      replica-set topology, change streams, and aggregation pipelines."
            .to_string(),
        dashboard_path: "observability/dashboards/cave-docdb.json".to_string(),
        alerts_path: "observability/alerts/cave-docdb.yml".to_string(),
        live_data_endpoint: "/api/docdb/status".to_string(),
    }
}
