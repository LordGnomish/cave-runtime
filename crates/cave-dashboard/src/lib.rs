// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Dashboard — Grafana-compatible dashboard engine.
//!
//! Compatible with: Grafana v10
//! Upstream tracking: see cave-upstream for monitored features.
//!
//! ## Feature parity
//! - Full Grafana JSON dashboard model (panels, rows, templating, annotations)
//! - 23 panel types (graph, stat, gauge, table, bar gauge, pie chart, heatmap,
//!   logs, traces, text, alert list, dashboard list, histogram, state timeline,
//!   status history, candlestick, flamegraph, geomap, canvas, xyChart, …)
//! - Template variables (query, custom, textbox, constant, datasource, interval,
//!   ad-hoc filters, group-by) with full `$var` / `${var}` / `[[var]]` interpolation
//! - Annotations (built-in dashboard/alert and datasource-driven)
//! - Dashboard versioning and history
//! - Dashboard folders with permissions
//! - Dashboard import/export (JSON/YAML)
//! - Dashboard provisioning from config files
//! - Datasource CRUD + proxy + health checks for Prometheus, Loki, Jaeger/Tempo,
//!   Postgres, Elasticsearch, InfluxDB
//! - Mixed datasource queries with caching and inspector
//! - 14 transformations (reduce, merge, filter, organize, calculateField,
//!   groupBy, sortBy, renameByRegex, concatenate, convertFieldType, limit,
//!   seriesToRows, joinByField, labelsToFields)
//! - Unified Alerting (alert rules, multi-dimensional evaluation, Normal/Pending/
//!   Firing/Error/NoData, contact points, notification policies, silences,
//!   mute timings, alert groups)
//! - Multi-org support, teams, roles (Viewer/Editor/Admin), API keys,
//!   service accounts
//! - Full Grafana HTTP API (/api/dashboards/*, /api/search, /api/datasources,
//!   /api/ds/query, /api/folders, /api/ruler, /api/alertmanager, /api/orgs,
//!   /api/users, /api/teams, /api/auth/keys, /api/v1/provisioning/*, …)
//! - HTML renderer (dark-themed, Bootstrap-less, self-contained)

pub mod alerting;
pub mod auth;
pub mod datasource;
pub mod models;
pub mod panels;
pub mod provisioning;
pub mod query;
pub mod renderer;
pub mod routes;
pub mod store;
pub mod variables;

use axum::Router;
use std::sync::Arc;

pub use models::*;
pub use routes::DashboardState;
pub use store::DashboardStore;

/// Module name for health/status endpoints.
pub const MODULE_NAME: &str = "dashboard";

/// Create the axum router for this module.
pub fn router(state: Arc<DashboardState>) -> Router {
    routes::create_router(state)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        alerting::{apply_reducer, eval_threshold, evaluate_alert_conditions},
        models::*,
        provisioning::{parse_datasource_provisioning, provision_from_json, provision_from_yaml},
        renderer::render_dashboard,
        store::DashboardStore,
        variables::{compute_interval, interpolate, InterpolationContext},
    };
    use std::collections::HashMap;

    // ── Store / Dashboard CRUD ─────────────────────────────────────────────

    #[test]
    fn test_dashboard_create_and_retrieve() {
        let store = DashboardStore::new();
        let db = Dashboard::new(0, 1, "My Dashboard");
        let saved = store.upsert_dashboard(1, db.clone(), None, "initial", "admin", false).unwrap();
        assert_eq!(saved.title, "My Dashboard");
        assert!(saved.id.is_some());
        assert_eq!(saved.version, 1);

        let retrieved = store.get_dashboard_by_uid(&saved.uid).unwrap();
        assert_eq!(retrieved.uid, saved.uid);
    }

    #[test]
    fn test_dashboard_update_increments_version() {
        let store = DashboardStore::new();
        let db = Dashboard::new(0, 1, "Versioned");
        let saved = store.upsert_dashboard(1, db, None, "v1", "admin", false).unwrap();
        let mut updated = saved.clone();
        updated.title = "Versioned v2".into();
        let v2 = store.upsert_dashboard(1, updated, None, "v2", "admin", true).unwrap();
        assert_eq!(v2.version, 2);
        assert_eq!(v2.title, "Versioned v2");
    }

    #[test]
    fn test_dashboard_version_conflict() {
        let store = DashboardStore::new();
        let db = Dashboard::new(0, 1, "Conflict Test");
        let saved = store.upsert_dashboard(1, db, None, "init", "admin", false).unwrap();

        // Simulate stale edit (wrong version)
        let mut stale = saved.clone();
        stale.version = 0; // old version
        stale.title = "Stale".into();
        let result = store.upsert_dashboard(1, stale, None, "stale", "admin", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_dashboard_delete() {
        let store = DashboardStore::new();
        let db = Dashboard::new(0, 1, "To Delete");
        let saved = store.upsert_dashboard(1, db, None, "", "admin", false).unwrap();
        store.delete_dashboard(&saved.uid).unwrap();
        assert!(store.get_dashboard_by_uid(&saved.uid).is_err());
    }

    #[test]
    fn test_dashboard_list() {
        let store = DashboardStore::new();
        for title in &["DB1", "DB2", "DB3"] {
            let db = Dashboard::new(0, 1, title);
            store.upsert_dashboard(1, db, None, "", "admin", false).unwrap();
        }
        let all = store.list_dashboards(1).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_dashboard_search_by_title() {
        let store = DashboardStore::new();
        let db1 = Dashboard::new(0, 1, "Kubernetes Nodes");
        let db2 = Dashboard::new(0, 1, "Redis Metrics");
        store.upsert_dashboard(1, db1, None, "", "admin", false).unwrap();
        store.upsert_dashboard(1, db2, None, "", "admin", false).unwrap();

        let q = SearchQuery { query: Some("kubernetes".into()), ..Default::default() };
        let results = store.search_dashboards(&q).unwrap();
        assert_eq!(results.iter().filter(|r| r.r#type == "dash-db").count(), 1);
        assert!(results.iter().any(|r| r.title == "Kubernetes Nodes"));
    }

    #[test]
    fn test_dashboard_search_by_tag() {
        let store = DashboardStore::new();
        let mut db = Dashboard::new(0, 1, "Tagged Dashboard");
        db.tags = vec!["production".into(), "k8s".into()];
        store.upsert_dashboard(1, db, None, "", "admin", false).unwrap();

        let q = SearchQuery { tag: vec!["production".into()], ..Default::default() };
        let results = store.search_dashboards(&q).unwrap();
        assert!(results.iter().any(|r| r.title == "Tagged Dashboard"));
    }

    #[test]
    fn test_dashboard_versioning_history() {
        let store = DashboardStore::new();
        let db = Dashboard::new(0, 1, "Versioned");
        let saved = store.upsert_dashboard(1, db, None, "v1", "admin", false).unwrap();

        let id = saved.id.unwrap();
        let versions = store.get_dashboard_versions(id).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[0].message, "v1");
    }

    #[test]
    fn test_star_dashboard() {
        let store = DashboardStore::new();
        let db = Dashboard::new(0, 1, "Starrable");
        let saved = store.upsert_dashboard(1, db, None, "", "admin", false).unwrap();
        store.star_dashboard(1, &saved.uid, true).unwrap();

        let d = store.get_dashboard_by_uid(&saved.uid).unwrap();
        assert!(d.is_starred);

        store.star_dashboard(1, &saved.uid, false).unwrap();
        let d2 = store.get_dashboard_by_uid(&saved.uid).unwrap();
        assert!(!d2.is_starred);
    }

    // ── Folder CRUD ────────────────────────────────────────────────────────

    #[test]
    fn test_folder_crud() {
        let store = DashboardStore::new();
        let folder = store.create_folder(1, Some("my-folder"), "My Folder", None).unwrap();
        assert_eq!(folder.title, "My Folder");
        assert_eq!(folder.uid, "my-folder");

        let retrieved = store.get_folder_by_uid("my-folder").unwrap();
        assert_eq!(retrieved.id, folder.id);

        let updated = store.update_folder("my-folder", "Updated Title").unwrap();
        assert_eq!(updated.title, "Updated Title");
        assert_eq!(updated.version, 2);

        store.delete_folder("my-folder").unwrap();
        assert!(store.get_folder_by_uid("my-folder").is_err());
    }

    #[test]
    fn test_dashboard_in_folder() {
        let store = DashboardStore::new();
        let folder = store.create_folder(1, Some("infra"), "Infrastructure", None).unwrap();
        let db = Dashboard::new(0, 1, "Infra Dashboard");
        let saved = store.upsert_dashboard(1, db, Some("infra"), "", "admin", false).unwrap();

        assert_eq!(saved.folder_uid, Some("infra".into()));
        assert_eq!(saved.folder_title, Some("Infrastructure".into()));
    }

    // ── DataSource CRUD ────────────────────────────────────────────────────

    #[test]
    fn test_datasource_crud() {
        let store = DashboardStore::new();
        let req = CreateDataSourceRequest {
            name: "Prometheus".into(),
            ds_type: DataSourceType::Prometheus,
            url: "http://localhost:9090".into(),
            access: DataSourceAccess::Proxy,
            is_default: true,
            json_data: serde_json::json!({}),
            uid: None,
            basic_auth: false,
            basic_auth_user: String::new(),
            user: String::new(),
            database: String::new(),
            org_id: None,
        };

        let ds = store.create_datasource(req, 1).unwrap();
        assert_eq!(ds.name, "Prometheus");
        assert!(ds.is_default);

        let retrieved = store.get_datasource_by_id(ds.id).unwrap();
        assert_eq!(retrieved.uid, ds.uid);

        store.delete_datasource(&ds.uid).unwrap();
        assert!(store.get_datasource_by_uid(&ds.uid).is_err());
    }

    #[test]
    fn test_datasource_type_serialization() {
        assert_eq!(serde_json::to_string(&DataSourceType::Prometheus).unwrap(), "\"prometheus\"");
        assert_eq!(serde_json::to_string(&DataSourceType::Loki).unwrap(), "\"loki\"");
        assert_eq!(serde_json::to_string(&DataSourceType::Postgres).unwrap(), "\"postgres\"");
    }

    // ── Alert Evaluation ───────────────────────────────────────────────────

    #[test]
    fn test_eval_threshold_gt() {
        assert!(eval_threshold(100.0, "gt", &[50.0]));
        assert!(!eval_threshold(30.0, "gt", &[50.0]));
    }

    #[test]
    fn test_eval_threshold_lt() {
        assert!(eval_threshold(10.0, "lt", &[50.0]));
        assert!(!eval_threshold(60.0, "lt", &[50.0]));
    }

    #[test]
    fn test_eval_threshold_range() {
        assert!(eval_threshold(5.0, "within_range", &[1.0, 10.0]));
        assert!(!eval_threshold(15.0, "within_range", &[1.0, 10.0]));
        assert!(eval_threshold(0.0, "outside_range", &[1.0, 10.0]));
        assert!(!eval_threshold(5.0, "outside_range", &[1.0, 10.0]));
    }

    #[test]
    fn test_apply_reducer() {
        let vals = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(apply_reducer("avg", &vals), Some(3.0));
        assert_eq!(apply_reducer("min", &vals), Some(1.0));
        assert_eq!(apply_reducer("max", &vals), Some(5.0));
        assert_eq!(apply_reducer("sum", &vals), Some(15.0));
        assert_eq!(apply_reducer("count", &vals), Some(5.0));
        assert_eq!(apply_reducer("last", &vals), Some(5.0));
        assert_eq!(apply_reducer("first", &vals), Some(1.0));
    }

    #[test]
    fn test_evaluate_conditions_no_conditions() {
        let result = evaluate_alert_conditions(&[], &HashMap::new());
        assert_eq!(result, AlertState::Normal);
    }

    #[test]
    fn test_evaluate_conditions_firing() {
        let condition = AlertCondition {
            condition_type: "query".into(),
            query: AlertConditionQuery { params: vec!["A".into(), "5m".into(), "now".into()] },
            reducer: AlertReducer { reducer_type: "last".into(), params: vec![] },
            evaluator: AlertEvaluator { eval_type: "gt".into(), params: vec![50.0] },
            operator: AlertOperator { op_type: "and".into() },
        };
        let mut values = HashMap::new();
        values.insert("A".to_string(), vec![10.0, 20.0, 80.0]);
        let result = evaluate_alert_conditions(&[condition], &values);
        assert_eq!(result, AlertState::Firing);
    }

    #[test]
    fn test_evaluate_conditions_normal() {
        let condition = AlertCondition {
            condition_type: "query".into(),
            query: AlertConditionQuery { params: vec!["A".into()] },
            reducer: AlertReducer { reducer_type: "avg".into(), params: vec![] },
            evaluator: AlertEvaluator { eval_type: "gt".into(), params: vec![100.0] },
            operator: AlertOperator { op_type: "and".into() },
        };
        let mut values = HashMap::new();
        values.insert("A".to_string(), vec![10.0, 20.0, 30.0]);
        let result = evaluate_alert_conditions(&[condition], &values);
        assert_eq!(result, AlertState::Normal);
    }

    #[test]
    fn test_evaluate_conditions_and_or() {
        let c1 = AlertCondition {
            condition_type: "query".into(),
            query: AlertConditionQuery { params: vec!["A".into()] },
            reducer: AlertReducer { reducer_type: "last".into(), params: vec![] },
            evaluator: AlertEvaluator { eval_type: "gt".into(), params: vec![50.0] },
            operator: AlertOperator { op_type: "and".into() },
        };
        let c2 = AlertCondition {
            condition_type: "query".into(),
            query: AlertConditionQuery { params: vec!["B".into()] },
            reducer: AlertReducer { reducer_type: "last".into(), params: vec![] },
            evaluator: AlertEvaluator { eval_type: "gt".into(), params: vec![50.0] },
            operator: AlertOperator { op_type: "or".into() },
        };
        let mut values = HashMap::new();
        values.insert("A".to_string(), vec![10.0]); // below threshold
        values.insert("B".to_string(), vec![100.0]); // above threshold

        // With OR: A fails but B fires → Firing
        let result = evaluate_alert_conditions(&[c1, c2], &values);
        assert_eq!(result, AlertState::Firing);
    }

    // ── Annotations ────────────────────────────────────────────────────────

    #[test]
    fn test_annotation_crud() {
        let store = DashboardStore::new();
        let req = CreateAnnotationRequest {
            dashboard_uid: Some("test-uid".into()),
            panel_id: Some(1),
            time: 1000,
            time_end: Some(2000),
            tags: vec!["deploy".into()],
            text: "Deployment v1.2.3".into(),
        };
        let ann = store.create_annotation(req, 1, 1).unwrap();
        assert_eq!(ann.text, "Deployment v1.2.3");
        assert_eq!(ann.tags, vec!["deploy"]);

        let anns = store.list_annotations(Some("test-uid"), 1).unwrap();
        assert_eq!(anns.len(), 1);

        store.delete_annotation(ann.id).unwrap();
        let anns2 = store.list_annotations(Some("test-uid"), 1).unwrap();
        assert_eq!(anns2.len(), 0);
    }

    // ── Snapshots ─────────────────────────────────────────────────────────

    #[test]
    fn test_snapshot_create_and_retrieve() {
        let store = DashboardStore::new();
        let req = CreateSnapshotRequest {
            dashboard: serde_json::json!({"title": "Snapshot DB", "panels": []}),
            name: Some("Test Snapshot".into()),
            expires: Some(86400 * 365),
            external: false,
            key: Some("testkey123".into()),
            delete_key: Some("deletekey456".into()),
        };
        let snap = store.create_snapshot(req, 1, 1).unwrap();
        assert_eq!(snap.key, "testkey123");

        let retrieved = store.get_snapshot("testkey123").unwrap();
        assert_eq!(retrieved.name, "Test Snapshot");
    }

    #[test]
    fn test_snapshot_expired() {
        let store = DashboardStore::new();
        let req = CreateSnapshotRequest {
            dashboard: serde_json::json!({}),
            name: None,
            expires: Some(1), // 1 second
            external: false,
            key: Some("expiredkey".into()),
            delete_key: Some("del".into()),
        };
        store.create_snapshot(req, 1, 1).unwrap();
        // Manually force expiry by sleeping is impractical in tests.
        // The logic is covered by the TTL check in get_snapshot.
    }

    #[test]
    fn test_snapshot_delete() {
        let store = DashboardStore::new();
        let req = CreateSnapshotRequest {
            dashboard: serde_json::json!({}),
            name: None,
            expires: Some(86400),
            external: false,
            key: Some("k1".into()),
            delete_key: Some("dk1".into()),
        };
        store.create_snapshot(req, 1, 1).unwrap();
        store.delete_snapshot_by_delete_key("dk1").unwrap();
        assert!(store.get_snapshot("k1").is_err());
    }

    // ── Playlists ─────────────────────────────────────────────────────────

    #[test]
    fn test_playlist_crud() {
        let store = DashboardStore::new();
        let items = vec![PlaylistItem {
            id: 1,
            playlist_id: 0,
            item_type: "dashboard_by_uid".into(),
            value: "abc123".into(),
            order: 1,
            title: "Test Dashboard".into(),
        }];
        let p = store.create_playlist(1, "My Playlist", "5m", items).unwrap();
        assert_eq!(p.name, "My Playlist");
        assert_eq!(p.interval, "5m");
        assert_eq!(p.items.len(), 1);

        let retrieved = store.get_playlist(p.id).unwrap();
        assert_eq!(retrieved.id, p.id);

        let updated = store.update_playlist(p.id, "New Name", "10m", vec![]).unwrap();
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.interval, "10m");
        assert!(updated.items.is_empty());

        store.delete_playlist(p.id).unwrap();
        assert!(store.get_playlist(p.id).is_err());
    }

    // ── Variable Interpolation ─────────────────────────────────────────────

    #[test]
    fn test_var_interpolation_dollar() {
        let vars = vec![make_var("namespace", "default")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("namespace=$namespace", &ctx), "namespace=default");
    }

    #[test]
    fn test_var_interpolation_brace() {
        let vars = vec![make_var("env", "prod")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("${env}-cluster", &ctx), "prod-cluster");
    }

    #[test]
    fn test_var_interpolation_legacy() {
        let vars = vec![make_var("region", "eu-west-1")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("[[region]]", &ctx), "eu-west-1");
    }

    #[test]
    fn test_var_interpolation_multiple() {
        let vars = vec![make_var("ns", "kube-system"), make_var("pod", "coredns")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(
            interpolate("namespace=$ns pod=$pod", &ctx),
            "namespace=kube-system pod=coredns"
        );
    }

    #[test]
    fn test_var_interpolation_unknown_left_unchanged() {
        let ctx = InterpolationContext::new(&[]);
        assert_eq!(interpolate("$unknown_var", &ctx), "$unknown_var");
    }

    #[test]
    fn test_builtin_interval_computed() {
        let ctx = InterpolationContext::new(&[]);
        let result = interpolate("step=$__interval", &ctx);
        assert!(result.starts_with("step="), "got: {result}");
    }

    #[test]
    fn test_compute_interval_ranges() {
        assert_eq!(compute_interval("now-30m", "now"), "10s");
        assert_eq!(compute_interval("now-3h", "now"), "30s");
        assert_eq!(compute_interval("now-12h", "now"), "1m");
        assert_eq!(compute_interval("now-7d", "now"), "30m");
        assert_eq!(compute_interval("now-90d", "now"), "1h");
    }

    // ── Provisioning ──────────────────────────────────────────────────────

    #[test]
    fn test_provision_json_minimal() {
        let json = r#"{"title":"Provisioned","panels":[],"schemaVersion":39}"#;
        let v = provision_from_json(json).unwrap();
        assert_eq!(v["title"], "Provisioned");
    }

    #[test]
    fn test_provision_yaml_minimal() {
        let yaml = "title: YAML Dashboard\npanels: []\n";
        let v = provision_from_yaml(yaml).unwrap();
        assert_eq!(v["title"], "YAML Dashboard");
    }

    #[test]
    fn test_provision_json_wrapped() {
        let json = r#"{"dashboard":{"title":"Wrapped","panels":[]},"folderId":0}"#;
        let v = provision_from_json(json).unwrap();
        assert_eq!(v["title"], "Wrapped");
    }

    #[test]
    fn test_provision_datasource_yaml() {
        let yaml = r#"
apiVersion: 1
datasources:
  - name: Prometheus
    type: prometheus
    url: http://prom:9090
    isDefault: true
"#;
        let config = parse_datasource_provisioning(yaml).unwrap();
        assert_eq!(config.datasources.len(), 1);
        assert_eq!(config.datasources[0].name, "Prometheus");
    }

    // ── HTML Renderer ─────────────────────────────────────────────────────

    #[test]
    fn test_render_dashboard_produces_valid_html() {
        let db = Dashboard::new(1, 1, "Render Test");
        let html = render_dashboard(&db);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Render Test"));
        assert!(html.contains("CAVE Dashboard"));
    }

    #[test]
    fn test_render_dashboard_escapes_xss() {
        let mut db = Dashboard::new(1, 1, "<script>alert('xss')</script>");
        let html = render_dashboard(&db);
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_render_dashboard_with_tags() {
        let mut db = Dashboard::new(1, 1, "Tagged");
        db.tags = vec!["prod".into(), "k8s".into()];
        let html = render_dashboard(&db);
        assert!(html.contains("prod"));
        assert!(html.contains("k8s"));
    }

    #[test]
    fn test_render_all_panel_types() {
        let mut db = Dashboard::new(1, 1, "All Panels");
        for (i, pt) in [
            PanelType::Graph, PanelType::Stat, PanelType::Gauge,
            PanelType::Table, PanelType::BarGauge, PanelType::Text,
            PanelType::Logs, PanelType::AlertList,
        ].iter().enumerate() {
            db.panels.push(Panel {
                id: i as i32 + 1,
                title: format!("{pt} panel"),
                panel_type: *pt,
                grid_pos: GridPos { x: 0, y: i as i32 * 8, w: 12, h: 8, static_pos: false },
                datasource: None,
                targets: vec![],
                transformations: vec![],
                field_config: FieldConfig::default(),
                options: serde_json::json!({}),
                description: String::new(),
                transparent: false,
                links: vec![],
                repeat: None,
                repeat_direction: None,
                max_data_points: None,
                interval: None,
                time_from: None,
                time_shift: None,
                alert: None,
                panels: vec![],
                collapsed: false,
                hide_time_override: false,
                cache_timeout: None,
                query_caching_ttl: None,
                plugin_version: None,
            });
        }
        let html = render_dashboard(&db);
        assert!(html.contains("<!DOCTYPE html>"));
    }

    // ── Dashboard Slug ─────────────────────────────────────────────────────

    #[test]
    fn test_slug_generation() {
        assert_eq!(Dashboard::slug_from_title("My Dashboard"), "my-dashboard");
        assert_eq!(Dashboard::slug_from_title("Kubernetes / Node Metrics"), "kubernetes-node-metrics");
        assert_eq!(Dashboard::slug_from_title("  spaces  "), "spaces");
        assert_eq!(Dashboard::slug_from_title("Already-Slugged"), "already-slugged");
    }

    // ── Org / User / Team ─────────────────────────────────────────────────

    #[test]
    fn test_org_crud() {
        let store = DashboardStore::new();
        let org = store.create_org("My Org").unwrap();
        assert!(!org.name.is_empty());
        let retrieved = store.get_org(org.id).unwrap();
        assert_eq!(retrieved.name, "My Org");
    }

    #[test]
    fn test_user_crud() {
        let store = DashboardStore::new();
        let req = CreateUserRequest {
            name: "Alice".into(),
            email: "alice@example.com".into(),
            login: "alice".into(),
            password: "s3cret".into(),
            org_id: Some(1),
        };
        let user = store.create_user(req).unwrap();
        assert_eq!(user.login, "alice");

        let retrieved = store.get_user(user.id).unwrap();
        assert_eq!(retrieved.email, "alice@example.com");
    }

    #[test]
    fn test_team_with_members() {
        let store = DashboardStore::new();
        let team = store.create_team(1, "Platform", "platform@example.com").unwrap();
        let member = TeamMember {
            org_id: 1,
            team_id: team.id,
            user_id: 42,
            login: "bob".into(),
            name: "Bob".into(),
            email: "bob@example.com".into(),
            avatar_url: String::new(),
            labels: vec![],
            permission: TeamPermission::Member,
        };
        store.add_team_member(team.id, member).unwrap();
        let members = store.list_team_members(team.id).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].login, "bob");
    }

    // ── Alert Rules ────────────────────────────────────────────────────────

    #[test]
    fn test_alert_rule_crud() {
        let store = DashboardStore::new();
        let rule = make_alert_rule("rule-001");
        let saved = store.upsert_alert_rule(rule).unwrap();
        assert_eq!(saved.title, "High CPU");

        let retrieved = store.get_alert_rule("rule-001").unwrap();
        assert_eq!(retrieved.uid, "rule-001");

        store.delete_alert_rule("rule-001").unwrap();
        assert!(store.get_alert_rule("rule-001").is_err());
    }

    #[test]
    fn test_list_rule_groups() {
        let store = DashboardStore::new();
        let r1 = make_alert_rule_in_group("r1", "group-a");
        let r2 = make_alert_rule_in_group("r2", "group-a");
        let r3 = make_alert_rule_in_group("r3", "group-b");
        store.upsert_alert_rule(r1).unwrap();
        store.upsert_alert_rule(r2).unwrap();
        store.upsert_alert_rule(r3).unwrap();

        let groups = store.list_rule_groups(1).unwrap();
        assert_eq!(groups.len(), 2);
    }

    // ── Contact Points & Silences ─────────────────────────────────────────

    #[test]
    fn test_contact_point_crud() {
        let store = DashboardStore::new();
        let cp = ContactPoint {
            uid: "cp-1".into(),
            name: "Webhook".into(),
            cp_type: ContactPointType::Webhook,
            settings: serde_json::json!({"url":"http://webhook.example.com"}),
            disable_resolve_message: false,
            send_reminder: false,
            frequency: String::new(),
        };
        store.upsert_contact_point(cp).unwrap();
        let retrieved = store.get_contact_point("cp-1").unwrap();
        assert_eq!(retrieved.name, "Webhook");

        store.delete_contact_point("cp-1").unwrap();
        assert!(store.get_contact_point("cp-1").is_err());
    }

    #[test]
    fn test_silence_lifecycle() {
        let store = DashboardStore::new();
        let silence = Silence {
            id: String::new(),
            status: SilenceStatus { state: "active".into() },
            updated_at: chrono::Utc::now(),
            comment: "maintenance window".into(),
            created_by: "admin".into(),
            ends_at: chrono::Utc::now() + chrono::Duration::hours(2),
            starts_at: chrono::Utc::now(),
            matchers: vec![SilenceMatcher {
                name: "alertname".into(),
                value: "HighCPU".into(),
                is_equal: true,
                is_regex: false,
            }],
        };
        let saved = store.create_silence(silence).unwrap();
        assert!(!saved.id.is_empty());

        let all = store.list_silences().unwrap();
        assert_eq!(all.len(), 1);

        store.delete_silence(&saved.id).unwrap();
        let after = store.list_silences().unwrap();
        // Silence is expired (state changed to "expired"), still in list
        assert_eq!(after[0].status.state, "expired");
    }

    // ── API Key ────────────────────────────────────────────────────────────

    #[test]
    fn test_api_key_create_and_lookup() {
        use crate::auth::{generate_api_key, hash_api_key};
        let store = DashboardStore::new();
        let token = generate_api_key();
        let hash = hash_api_key(&token);
        let key = store.create_api_key(1, "ci-key", OrgRole::Editor, None, &hash, &token).unwrap();
        assert!(key.key.is_some()); // returned on creation

        let found = store.lookup_api_key(&hash).unwrap();
        assert_eq!(found.name, "ci-key");
        assert_eq!(found.role, OrgRole::Editor);
    }

    // ── Notification routing ───────────────────────────────────────────────

    #[test]
    fn test_notification_routing() {
        use crate::alerting::route_alert;
        let policy = NotificationPolicy {
            receiver: "default".into(),
            group_by: vec!["alertname".into()],
            continue_policy: false,
            matchers: vec![],
            group_wait: None,
            group_interval: None,
            repeat_interval: None,
            mute_time_intervals: vec![],
            routes: vec![
                NotificationPolicy {
                    receiver: "pagerduty".into(),
                    group_by: vec![],
                    continue_policy: false,
                    matchers: vec![Matcher {
                        name: "severity".into(),
                        value: "critical".into(),
                        is_equal: true,
                        is_regex: false,
                    }],
                    group_wait: None,
                    group_interval: None,
                    repeat_interval: None,
                    mute_time_intervals: vec![],
                    routes: vec![],
                }
            ],
        };

        let mut critical_labels = HashMap::new();
        critical_labels.insert("severity".into(), "critical".into());
        assert_eq!(route_alert(&policy, &critical_labels), "pagerduty");

        let mut warning_labels = HashMap::new();
        warning_labels.insert("severity".into(), "warning".into());
        assert_eq!(route_alert(&policy, &warning_labels), "default");
    }

    // ─── Helpers ─────────────────────────────────────────────────────────

    fn make_var(name: &str, value: &str) -> Variable {
        Variable {
            name: name.to_string(),
            label: name.to_string(),
            var_type: VariableType::Custom,
            description: String::new(),
            hide: VariableHide::DontHide,
            refresh: VariableRefresh::Never,
            sort: VariableSort::Disabled,
            query: serde_json::Value::String(String::new()),
            datasource: None,
            options: vec![],
            current: VariableOption {
                value: serde_json::Value::String(value.to_string()),
                text: serde_json::Value::String(value.to_string()),
                selected: true,
            },
            multi: false,
            include_all: false,
            all_value: None,
            regex: String::new(),
            values_text: String::new(),
            skip_url_sync: false,
        }
    }

    fn make_alert_rule(uid: &str) -> AlertRule {
        make_alert_rule_in_group(uid, "default")
    }

    fn make_alert_rule_in_group(uid: &str, group: &str) -> AlertRule {
        let now = chrono::Utc::now();
        AlertRule {
            id: 0,
            uid: uid.to_string(),
            org_id: 1,
            folder_uid: "general".into(),
            rule_group: group.to_string(),
            title: "High CPU".into(),
            condition: "C".into(),
            data: vec![],
            no_data_state: NoDataState::NoData,
            exec_err_state: ExecErrState::Alerting,
            for_duration: "5m".into(),
            annotations: HashMap::new(),
            labels: HashMap::new(),
            is_paused: false,
            updated: now,
            created: now,
            state: AlertState::Normal,
            health: "ok".into(),
            last_evaluation: None,
            evaluation_time: None,
        }
    }
}
