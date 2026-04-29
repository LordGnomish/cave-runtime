//! cave-cache portal wrap.

use super::Wrap;

pub fn wrap() -> Wrap {
    Wrap {
        module: "cave-cache".to_string(),
        title: "Cache (Valkey-compatible)".to_string(),
        upstream: "valkey-io/valkey".to_string(),
        upstream_commit: "16ed690".to_string(),
        description: "RESP3 wire-compatible in-memory cache with cluster, \
                      pub/sub, streams, and scripting."
            .to_string(),
        dashboard_path: "observability/dashboards/cave-cache.json".to_string(),
        alerts_path: "observability/alerts/cave-cache.yml".to_string(),
        live_data_endpoint: "/api/cache/info".to_string(),
    }
}
