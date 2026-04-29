//! cave-pg portal wrap.

use super::Wrap;

pub fn wrap() -> Wrap {
    Wrap {
        module: "cave-pg".to_string(),
        title: "PostgreSQL (CloudNativePG)".to_string(),
        upstream: "postgres/postgres + cloudnative-pg/cloudnative-pg".to_string(),
        upstream_commit: "ca9807d (postgres) / f8283b0 (cnpg)".to_string(),
        description: "PostgreSQL-wire-compatible server with the CloudNativePG \
                      operator's CRDs and reconcilers."
            .to_string(),
        dashboard_path: "observability/dashboards/cave-pg.json".to_string(),
        alerts_path: "observability/alerts/cave-pg.yml".to_string(),
        live_data_endpoint: "/api/pg/status".to_string(),
    }
}
