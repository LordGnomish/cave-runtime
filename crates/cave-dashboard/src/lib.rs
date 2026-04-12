//! CAVE Dashboard — Grafana-compatible dashboard and visualization engine.
//!
//! Replaces Grafana with a Rust-native implementation.
//! Supports: dashboards, panels, variables, alerting, snapshots, playlists,
//! provisioning, annotations, and a Grafana API v1–compatible HTTP interface.
//!
//! ## Upstream Tracking: Grafana
//! - GitHub: https://github.com/grafana/grafana
//! - Tracked: dashboard JSON model, panel types, alerting, API routes
//! - Parity target: Grafana v10.x feature set

pub mod alerting;
pub mod datasource;
pub mod models;
pub mod provisioning;
pub mod renderer;
pub mod routes;
pub mod store;
pub mod variables;

use std::sync::{Arc, Mutex};

use axum::Router;

use store::DashboardStore;

/// Shared module state — wraps the in-memory store behind `Arc<Mutex<…>>`.
pub struct DashboardState {
    pub store: Arc<Mutex<DashboardStore>>,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self { store: Arc::new(Mutex::new(DashboardStore::default())) }
    }
}

/// Create the axum router for the dashboard module.
pub fn router(state: Arc<DashboardState>) -> Router {
    routes::create_router(state)
}

/// Module name for identification.
pub const MODULE_NAME: &str = "dashboard";

// ─── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use crate::{
        alerting::evaluate_alert,
        models::{
            AlertCondition, AlertEvaluator, AlertOperator, AlertQuery, AlertReducer, AlertRule,
            AlertState, Annotation, AnnotationType, Dashboard, DataSource, DataSourceAccess,
            DataSourceType, Folder, Panel, PanelType, PlaylistItem, PlaylistItemType,
            Snapshot, Variable, VariableHide, VariableOption, VariableRefresh, VariableType,
        },
        provisioning::provision_from_json,
        renderer::render_dashboard_html,
        store::DashboardStore,
        variables::interpolate,
    };

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn fresh_store() -> DashboardStore {
        DashboardStore::default()
    }

    fn make_dashboard(title: &str) -> Dashboard {
        let mut d = Dashboard::new(title);
        d.tags = vec!["test".to_string()];
        d
    }

    fn make_var(name: &str, value: &str) -> Variable {
        Variable {
            name: name.to_string(),
            label: None,
            var_type: VariableType::Custom,
            query: None,
            options: vec![VariableOption {
                text: value.to_string(),
                value: value.to_string(),
                selected: true,
            }],
            current: Some(VariableOption {
                text: value.to_string(),
                value: value.to_string(),
                selected: true,
            }),
            multi: false,
            include_all: false,
            refresh: VariableRefresh::Never,
            hide: VariableHide::DontHide,
            description: None,
        }
    }

    fn make_alert_rule(name: &str, evaluator_type: &str, threshold: f64) -> AlertRule {
        AlertRule {
            id: 1,
            name: name.to_string(),
            message: format!("{name} alert"),
            frequency: "10s".to_string(),
            for_duration: "0s".to_string(),
            conditions: vec![AlertCondition {
                ref_id: "A".to_string(),
                evaluator: AlertEvaluator {
                    evaluator_type: evaluator_type.to_string(),
                    params: vec![threshold],
                },
                operator: AlertOperator { op_type: "and".to_string() },
                reducer: AlertReducer { reducer_type: "avg".to_string() },
                query: AlertQuery { params: vec![] },
            }],
            notifications: vec![],
            state: AlertState::Ok,
            no_data_state: Default::default(),
            exec_err_state: Default::default(),
        }
    }

    // ─── Dashboard CRUD ──────────────────────────────────────────────────────

    #[test]
    fn test_dashboard_create() {
        let mut store = fresh_store();
        let d = make_dashboard("My Dashboard");
        let uid = d.uid.clone();
        let saved = store.upsert_dashboard(d);
        assert_eq!(saved.title, "My Dashboard");
        assert!(saved.id > 0);
        assert_eq!(saved.uid, uid);
    }

    #[test]
    fn test_dashboard_read() {
        let mut store = fresh_store();
        let d = make_dashboard("Read Me");
        let uid = d.uid.clone();
        store.upsert_dashboard(d);
        let found = store.get_dashboard(&uid);
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Read Me");
    }

    #[test]
    fn test_dashboard_update() {
        let mut store = fresh_store();
        let mut d = make_dashboard("Original");
        store.upsert_dashboard(d.clone());
        d.title = "Updated".to_string();
        let updated = store.upsert_dashboard(d);
        assert_eq!(updated.title, "Updated");
        assert_eq!(updated.version, 2, "version should increment on update");
    }

    #[test]
    fn test_dashboard_delete() {
        let mut store = fresh_store();
        let d = make_dashboard("Gone");
        let uid = d.uid.clone();
        store.upsert_dashboard(d);
        assert!(store.delete_dashboard(&uid));
        assert!(store.get_dashboard(&uid).is_none());
    }

    #[test]
    fn test_dashboard_delete_nonexistent() {
        let mut store = fresh_store();
        assert!(!store.delete_dashboard("ghost-uid"));
    }

    #[test]
    fn test_dashboard_list() {
        let mut store = fresh_store();
        store.upsert_dashboard(make_dashboard("A"));
        store.upsert_dashboard(make_dashboard("B"));
        store.upsert_dashboard(make_dashboard("C"));
        assert_eq!(store.list_dashboards().len(), 3);
    }

    #[test]
    fn test_dashboard_search_by_title() {
        let mut store = fresh_store();
        store.upsert_dashboard(make_dashboard("Infra Overview"));
        store.upsert_dashboard(make_dashboard("App Metrics"));
        let results = store.search_dashboards(Some("infra"), None, None, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Infra Overview");
    }

    #[test]
    fn test_dashboard_search_by_tag() {
        let mut store = fresh_store();
        let mut d1 = make_dashboard("Tagged");
        d1.tags = vec!["ops".to_string(), "k8s".to_string()];
        let mut d2 = make_dashboard("Untagged");
        d2.tags = vec![];
        store.upsert_dashboard(d1);
        store.upsert_dashboard(d2);
        let results = store.search_dashboards(None, Some("ops"), None, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Tagged");
    }

    #[test]
    fn test_dashboard_star_unstar() {
        let mut store = fresh_store();
        let d = make_dashboard("Starworthy");
        let uid = d.uid.clone();
        store.upsert_dashboard(d);
        assert!(store.star_dashboard(&uid));
        assert!(store.get_dashboard(&uid).unwrap().is_starred);
        store.unstar_dashboard(&uid);
        assert!(!store.get_dashboard(&uid).unwrap().is_starred);
    }

    #[test]
    fn test_dashboard_search_starred() {
        let mut store = fresh_store();
        let d = make_dashboard("Fav");
        let uid = d.uid.clone();
        store.upsert_dashboard(d);
        store.star_dashboard(&uid);
        let starred = store.search_dashboards(None, None, None, Some(true));
        assert_eq!(starred.len(), 1);
        let unstarred = store.search_dashboards(None, None, None, Some(false));
        assert_eq!(unstarred.len(), 0);
    }

    // ─── Folders ─────────────────────────────────────────────────────────────

    #[test]
    fn test_folder_crud() {
        let mut store = fresh_store();
        let folder = Folder {
            id: 0,
            uid: Uuid::new_v4().to_string(),
            title: "Infrastructure".to_string(),
            url: "/dashboards/f/xyz/infrastructure".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let uid = folder.uid.clone();
        let saved = store.create_folder(folder);
        assert!(saved.id > 0);
        assert_eq!(saved.title, "Infrastructure");

        let updated = store.update_folder(&uid, "Infra Updated".to_string()).unwrap();
        assert_eq!(updated.title, "Infra Updated");

        assert!(store.delete_folder(&uid));
        assert!(store.get_folder(&uid).is_none());
    }

    #[test]
    fn test_dashboard_in_folder() {
        let mut store = fresh_store();
        let folder = Folder {
            id: 0,
            uid: "folder-uid".to_string(),
            title: "My Folder".to_string(),
            url: "/f/folder-uid/my-folder".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.create_folder(folder);

        let mut d = make_dashboard("In Folder");
        d.folder_uid = Some("folder-uid".to_string());
        let uid = d.uid.clone();
        store.upsert_dashboard(d);

        let results = store.search_dashboards(None, None, Some("folder-uid"), None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].uid, uid);
    }

    // ─── Panel Types ─────────────────────────────────────────────────────────

    #[test]
    fn test_all_panel_types_serialize() {
        let types = [
            PanelType::Graph,
            PanelType::Stat,
            PanelType::Gauge,
            PanelType::Table,
            PanelType::BarChart,
            PanelType::PieChart,
            PanelType::Heatmap,
            PanelType::Logs,
            PanelType::AlertList,
        ];
        for pt in &types {
            let json = serde_json::to_string(pt).expect("should serialize");
            let back: PanelType = serde_json::from_str(&json).expect("should deserialize");
            assert_eq!(pt, &back);
        }
    }

    #[test]
    fn test_panel_in_dashboard() {
        let mut d = make_dashboard("Panel Test");
        d.panels.push(Panel::new(1, "CPU Usage", PanelType::Graph));
        d.panels.push(Panel::new(2, "Memory", PanelType::Gauge));
        d.panels.push(Panel::new(3, "Events", PanelType::Logs));
        assert_eq!(d.panels.len(), 3);
        assert_eq!(d.panels[0].panel_type, PanelType::Graph);
        assert_eq!(d.panels[2].panel_type, PanelType::Logs);
    }

    // ─── Variable Interpolation ───────────────────────────────────────────────

    #[test]
    fn test_variable_interpolation_simple() {
        let vars = vec![make_var("env", "prod")];
        let out = interpolate("namespace=$env", &vars, None);
        assert_eq!(out, "namespace=prod");
    }

    #[test]
    fn test_variable_interpolation_brace() {
        let vars = vec![make_var("cluster", "us-east")];
        let out = interpolate("cluster=${cluster},region=eu", &vars, None);
        assert_eq!(out, "cluster=us-east,region=eu");
    }

    #[test]
    fn test_variable_interpolation_multiple() {
        let vars = vec![make_var("svc", "api"), make_var("env", "staging")];
        let out = interpolate("service=$svc env=$env", &vars, None);
        assert_eq!(out, "service=api env=staging");
    }

    #[test]
    fn test_variable_interpolation_unknown_kept() {
        let vars: Vec<Variable> = vec![];
        let out = interpolate("rate($metric[5m])", &vars, None);
        assert!(out.contains("$metric"), "unknown variable should remain");
    }

    // ─── DataSource CRUD ─────────────────────────────────────────────────────

    #[test]
    fn test_datasource_crud() {
        let mut store = fresh_store();
        let ds = DataSource {
            id: 0,
            uid: Uuid::new_v4().to_string(),
            name: "Prometheus".to_string(),
            datasource_type: DataSourceType::Prometheus,
            url: "http://cave-metrics:9090".to_string(),
            access: DataSourceAccess::Proxy,
            is_default: true,
            basic_auth: false,
            json_data: serde_json::Value::Null,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let saved = store.create_datasource(ds);
        assert!(saved.id > 0);
        assert_eq!(saved.name, "Prometheus");
        assert_eq!(saved.datasource_type, DataSourceType::Prometheus);

        let found = store.get_datasource(saved.id).unwrap();
        assert_eq!(found.url, "http://cave-metrics:9090");

        assert!(store.delete_datasource(saved.id));
        assert!(store.get_datasource(saved.id).is_none());
    }

    #[test]
    fn test_datasource_types() {
        for (name, dt) in [
            ("prometheus", DataSourceType::Prometheus),
            ("loki", DataSourceType::Loki),
            ("jaeger", DataSourceType::Jaeger),
        ] {
            let json = serde_json::to_string(&dt).unwrap();
            assert!(json.contains(name), "json should contain type name");
        }
    }

    // ─── Alert Evaluation ────────────────────────────────────────────────────

    #[test]
    fn test_alert_evaluation_firing() {
        let rule = make_alert_rule("cpu-high", "gt", 80.0);
        let result = evaluate_alert(&rule, 95.0);
        assert_eq!(result.state, AlertState::Alerting);
        assert!(result.value > 80.0);
    }

    #[test]
    fn test_alert_evaluation_ok() {
        let rule = make_alert_rule("cpu-high", "gt", 80.0);
        let result = evaluate_alert(&rule, 50.0);
        assert_eq!(result.state, AlertState::Ok);
    }

    #[test]
    fn test_alert_evaluation_lt() {
        let rule = make_alert_rule("low-mem", "lt", 20.0);
        assert_eq!(evaluate_alert(&rule, 10.0).state, AlertState::Alerting);
        assert_eq!(evaluate_alert(&rule, 30.0).state, AlertState::Ok);
    }

    // ─── Snapshot ────────────────────────────────────────────────────────────

    #[test]
    fn test_snapshot_create_and_retrieve() {
        let mut store = fresh_store();
        let snap = Snapshot {
            id: 0,
            key: "snap-key-abc".to_string(),
            delete_key: "del-key-abc".to_string(),
            name: "My Snapshot".to_string(),
            dashboard: serde_json::json!({ "title": "Snap" }),
            expires: None,
            created_at: Utc::now(),
            external: false,
            external_url: None,
        };
        let saved = store.create_snapshot(snap);
        assert!(saved.id > 0);
        let found = store.get_snapshot("snap-key-abc").unwrap();
        assert_eq!(found.name, "My Snapshot");
        assert!(!found.is_expired());
    }

    #[test]
    fn test_snapshot_expiry() {
        let past = Utc::now() - chrono::Duration::hours(1);
        let snap = Snapshot {
            id: 1,
            key: "expired-key".to_string(),
            delete_key: "del".to_string(),
            name: "Old".to_string(),
            dashboard: serde_json::Value::Null,
            expires: Some(past),
            created_at: past,
            external: false,
            external_url: None,
        };
        assert!(snap.is_expired());
    }

    // ─── Playlist ────────────────────────────────────────────────────────────

    #[test]
    fn test_playlist_crud() {
        let mut store = fresh_store();
        let playlist = crate::models::Playlist {
            id: Uuid::new_v4().to_string(),
            name: "Daily Rotation".to_string(),
            interval: "5m".to_string(),
            items: vec![
                PlaylistItem {
                    item_type: PlaylistItemType::DashboardByUid,
                    value: "uid-1".to_string(),
                    order: 1,
                    title: "Dashboard 1".to_string(),
                },
                PlaylistItem {
                    item_type: PlaylistItemType::DashboardByTag,
                    value: "ops".to_string(),
                    order: 2,
                    title: "Ops Dashboards".to_string(),
                },
            ],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let id = playlist.id.clone();
        let saved = store.create_playlist(playlist);
        assert_eq!(saved.name, "Daily Rotation");
        assert_eq!(saved.items.len(), 2);

        let found = store.get_playlist(&id).unwrap();
        assert_eq!(found.interval, "5m");

        assert!(store.delete_playlist(&id));
        assert!(store.get_playlist(&id).is_none());
    }

    // ─── Annotations ─────────────────────────────────────────────────────────

    #[test]
    fn test_annotation_crud() {
        let mut store = fresh_store();
        let ann = Annotation {
            id: 0,
            dashboard_uid: "dash-uid".to_string(),
            panel_id: Some(1),
            time: Utc::now(),
            time_end: None,
            tags: vec!["deploy".to_string()],
            text: "Deployed v1.2.3".to_string(),
            annotation_type: AnnotationType::Manual,
        };
        let saved = store.create_annotation(ann);
        assert!(saved.id > 0);

        let list = store.list_annotations(Some("dash-uid"));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].text, "Deployed v1.2.3");

        assert!(store.delete_annotation(saved.id));
        assert!(store.list_annotations(None).is_empty());
    }

    // ─── Provisioning ────────────────────────────────────────────────────────

    #[test]
    fn test_provisioning_from_json() {
        let json = r#"{
            "uid": "prov-uid",
            "title": "Provisioned Dashboard",
            "tags": ["provisioned", "infra"],
            "time": { "from": "now-24h", "to": "now" }
        }"#;
        let d = provision_from_json(json).expect("should provision");
        assert_eq!(d.uid, "prov-uid");
        assert_eq!(d.title, "Provisioned Dashboard");
        assert_eq!(d.tags.len(), 2);
        assert_eq!(d.time.from, "now-24h");
    }

    // ─── HTML Renderer ───────────────────────────────────────────────────────

    #[test]
    fn test_dashboard_html_render_basic() {
        let mut d = make_dashboard("Render Test");
        d.uid = "render-uid".to_string();
        d.panels.push(Panel::new(1, "CPU", PanelType::Graph));
        d.panels.push(Panel::new(2, "Mem", PanelType::Gauge));
        let html = render_dashboard_html(&d);
        assert!(html.contains("Render Test"), "title should appear in HTML");
        assert!(html.contains("render-uid"), "uid should appear");
        assert!(html.contains("CPU"), "panel title should appear");
        assert!(html.contains("time series"), "graph badge should appear");
        assert!(html.contains("<!DOCTYPE html>"), "should be valid HTML");
    }

    #[test]
    fn test_dashboard_html_tags() {
        let mut d = make_dashboard("Tagged Dash");
        d.tags = vec!["ops".to_string(), "k8s".to_string()];
        let html = render_dashboard_html(&d);
        assert!(html.contains("ops"));
        assert!(html.contains("k8s"));
    }

    #[test]
    fn test_dashboard_url_and_slug() {
        let mut d = Dashboard::new("My Great Dashboard");
        d.uid = "dash-uid-123".to_string();
        assert_eq!(d.slug(), "my-great-dashboard");
        assert_eq!(d.url(), "/d/dash-uid-123/my-great-dashboard");
    }
}
