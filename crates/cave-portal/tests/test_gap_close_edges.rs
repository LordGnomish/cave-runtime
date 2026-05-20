// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge-coverage integration tests for cave-portal public API.
//!
//! Targets modules that lacked any inline `#[cfg(test)]` block:
//! `models`, `dashboard`, `routes`, `ui`, `plugins`, `runtime_client`,
//! `catalog`, `admin::state` (free helpers), and the per-domain admin
//! type modules (`mlflow::types`, `keda::types`, `iceberg::types`,
//! `litellm::types`).
//!
//! Style: each test exercises one public surface — serde round-trip,
//! state filter helper boundary, error-variant Display, persona /
//! permission gating, parser fallback, or empty/non-empty divergence.

use cave_portal::admin::iceberg::types::{IcebergTable, IcebergViewError};
use cave_portal::admin::keda::types::{
    KedaAuthRef, KedaScaledObjectDetail, KedaScaleTargetRef, KedaTrigger,
};
use cave_portal::admin::litellm::types::{LiteLlmApiKey, LiteLlmModel, LiteLlmViewError};
use cave_portal::admin::mlflow::types::{MlflowExperiment, MlflowViewError};
use cave_portal::admin::permission::{AuthError, Permission, Persona, RequestCtx};
use cave_portal::admin::state::{AdminState, ActivityEntry, scope, tally_by_kind};
use cave_portal::admin::types::TenantId;
use cave_portal::admin::streams::connect;
use cave_portal::catalog::{
    CATALOG_SCHEMA_SQL, CatalogStore, Entity, EntityFilter, EntityMetadata, EntityRelation,
    Location, MemoryCatalogStore,
};
use cave_portal::dashboard::{get_dashboard, get_module_summary, get_nav, get_notifications, global_search, list_modules};
use cave_portal::models::{
    DashboardWidget, HealthStatus, LinkType, NavigationItem, Notification, NotificationSeverity,
    Service, ServiceLink, ServiceTier, UserPreference,
};
use cave_portal::plugins::ViewPersona;
use cave_portal::{PortalState, MODULE_NAME, router};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

// ── helpers ──────────────────────────────────────────────────────────────────

fn tenant(id: &str) -> TenantId {
    TenantId::new(id).expect("test tenant id should be valid")
}

// ── models: serde round-trip + defaults ──────────────────────────────────────

#[test]
fn service_tier_serializes_snake_case() {
    let t1 = serde_json::to_string(&ServiceTier::Tier1).unwrap();
    let t2 = serde_json::to_string(&ServiceTier::Tier2).unwrap();
    let t3 = serde_json::to_string(&ServiceTier::Tier3).unwrap();
    assert_eq!(t1, "\"tier1\"");
    assert_eq!(t2, "\"tier2\"");
    assert_eq!(t3, "\"tier3\"");
}

#[test]
fn service_round_trips_through_json() {
    let svc = Service {
        id: Uuid::nil(),
        name: "checkout".into(),
        description: "checkout API".into(),
        team: "payments".into(),
        tier: ServiceTier::Tier1,
        language: "rust".into(),
        repo_url: "https://example/repo".into(),
        tags: vec!["payments".into(), "tier-1".into()],
        registered_at: Utc::now(),
    };
    let json = serde_json::to_string(&svc).unwrap();
    let back: Service = serde_json::from_str(&json).unwrap();
    assert_eq!(svc, back);
}

#[test]
fn link_type_round_trips_all_variants() {
    for lt in [
        LinkType::RunBook,
        LinkType::Dashboard,
        LinkType::Docs,
        LinkType::Repo,
        LinkType::Chat,
    ] {
        let link = ServiceLink {
            service_id: Uuid::nil(),
            link_type: lt.clone(),
            url: "https://x".into(),
            label: "L".into(),
        };
        let j = serde_json::to_string(&link).unwrap();
        let back: ServiceLink = serde_json::from_str(&j).unwrap();
        assert_eq!(back.link_type, lt);
    }
}

#[test]
fn notification_severity_renders_snake_case() {
    let n = Notification {
        id: Uuid::nil(),
        module: "x".into(),
        title: "t".into(),
        body: "b".into(),
        severity: NotificationSeverity::Critical,
        created_at: Utc::now(),
        read: false,
        link: None,
    };
    let j = serde_json::to_string(&n).unwrap();
    assert!(j.contains("\"severity\":\"critical\""), "{j}");
}

#[test]
fn user_preference_default_picks_dark_grid() {
    let up = UserPreference::default();
    assert_eq!(up.theme, "dark");
    assert_eq!(up.dashboard_layout, "grid");
    assert!(!up.sidebar_collapsed);
    assert!(up.pinned_modules.is_empty());
    assert_eq!(up.user_id, Uuid::nil());
}

#[test]
fn health_status_serializes_snake_case() {
    assert_eq!(serde_json::to_string(&HealthStatus::Healthy).unwrap(), "\"healthy\"");
    assert_eq!(serde_json::to_string(&HealthStatus::Degraded).unwrap(), "\"degraded\"");
    assert_eq!(serde_json::to_string(&HealthStatus::Unhealthy).unwrap(), "\"unhealthy\"");
    assert_eq!(serde_json::to_string(&HealthStatus::Unknown).unwrap(), "\"unknown\"");
}

#[test]
fn dashboard_widget_carries_all_metadata_fields() {
    let w = DashboardWidget {
        module: "secrets".into(),
        display_name: "Secrets Scanner".into(),
        health: HealthStatus::Healthy,
        key_metric_label: "status".into(),
        key_metric_value: "operational".into(),
        link: "/modules/secrets".into(),
        upstream_replacement: "Vault".into(),
        category: "security".into(),
    };
    let j = serde_json::to_string(&w).unwrap();
    assert!(j.contains("\"display_name\":\"Secrets Scanner\""));
    assert!(j.contains("\"upstream_replacement\":\"Vault\""));
}

#[test]
fn navigation_item_optional_badge_round_trips() {
    let item = NavigationItem {
        id: "vulns".into(),
        label: "Vulnerability Mgmt".into(),
        icon: "shield".into(),
        path: "/modules/vulns".into(),
        category: "security".into(),
        upstream_replacement: "Snyk".into(),
        badge_count: Some(3),
    };
    let j = serde_json::to_string(&item).unwrap();
    let back: NavigationItem = serde_json::from_str(&j).unwrap();
    assert_eq!(back.badge_count, Some(3));
}

// ── dashboard: aggregation invariants ────────────────────────────────────────

#[test]
fn get_dashboard_contains_at_least_thirty_modules() {
    let d = get_dashboard();
    assert!(d.total_modules >= 30, "have only {}", d.total_modules);
    assert_eq!(d.modules.len(), d.total_modules);
    // Counts must add up.
    let sum = d.healthy_count + d.degraded_count + d.unhealthy_count + d.unknown_count;
    assert_eq!(sum, d.total_modules);
    // All seeded health is Healthy → no unknown.
    assert_eq!(d.healthy_count, d.total_modules);
}

#[test]
fn get_dashboard_module_links_match_pattern() {
    for w in get_dashboard().modules {
        let expected = format!("/modules/{}", w.module);
        assert_eq!(w.link, expected);
    }
}

#[test]
fn get_module_summary_known_id_returns_some() {
    let s = get_module_summary("secrets").expect("secrets is a known module");
    assert_eq!(s.module, "secrets");
    assert_eq!(s.category, "security");
    assert!(s.upstream_replacement.to_lowercase().contains("trufflehog")
        || s.upstream_replacement.to_lowercase().contains("gitleaks"));
}

#[test]
fn get_module_summary_unknown_id_returns_none() {
    assert!(get_module_summary("does-not-exist-zzz").is_none());
}

#[test]
fn get_module_summary_gateway_has_curated_kong_and_gravitee_features() {
    let s = get_module_summary("gateway").expect("gateway is known");
    let upstreams = s.stats.get("upstreams").expect("gateway carries upstreams").as_array().unwrap();
    let any_kong = upstreams.iter().any(|u| u.as_str().unwrap_or("").to_lowercase().contains("kong"));
    let any_grav = upstreams.iter().any(|u| u.as_str().unwrap_or("").to_lowercase().contains("gravitee"));
    assert!(any_kong, "kong missing");
    assert!(any_grav, "gravitee missing");
}

#[test]
fn list_modules_matches_dashboard_count() {
    let mods = list_modules();
    let d = get_dashboard();
    assert_eq!(mods.len(), d.total_modules);
}

#[test]
fn get_nav_groups_in_canonical_category_order() {
    let nav = get_nav();
    let labels: Vec<_> = nav.iter().map(|g| g.label.as_str()).collect();
    // Order must respect the documented `category_order` constant.
    let order = ["Security", "Observability", "Dev Tools", "Platform", "AI / Data"];
    let mut last_idx: i32 = -1;
    for label in labels {
        let here = order.iter().position(|o| *o == label).expect("label should be canonical");
        assert!(here as i32 > last_idx, "out-of-order: {label}");
        last_idx = here as i32;
    }
}

#[test]
fn get_nav_security_group_contains_secrets_and_certs() {
    let nav = get_nav();
    let sec = nav.iter().find(|g| g.label == "Security").expect("security group present");
    let ids: Vec<&str> = sec.items.iter().map(|i| i.id.as_str()).collect();
    assert!(ids.contains(&"secrets"));
    assert!(ids.contains(&"certs"));
}

#[test]
fn global_search_empty_query_returns_empty() {
    assert!(global_search("").is_empty());
    assert!(global_search("   ").is_empty());
}

#[test]
fn global_search_matches_module_id_substring() {
    let hits = global_search("vulns");
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|h| h.module == "vulns"));
}

#[test]
fn global_search_matches_upstream_replacement_case_insensitive() {
    // "SLACK" should match "chat" via upstream_replacement.
    let hits = global_search("SLACK");
    assert!(hits.iter().any(|h| h.module == "chat"), "{hits:?}");
}

#[test]
fn global_search_no_match_returns_empty() {
    assert!(global_search("zzzz-nonexistent-zzzz").is_empty());
}

#[test]
fn get_notifications_emits_at_least_one_critical_one_info_one_warning() {
    let ns = get_notifications();
    assert!(!ns.is_empty());
    let has_crit = ns.iter().any(|n| matches!(n.severity, NotificationSeverity::Critical));
    let has_warn = ns.iter().any(|n| matches!(n.severity, NotificationSeverity::Warning));
    let has_info = ns.iter().any(|n| matches!(n.severity, NotificationSeverity::Info));
    assert!(has_crit && has_warn && has_info, "missing severity variety: {ns:?}");
}

// ── ui: embedded HTML self-check ─────────────────────────────────────────────

#[test]
fn embedded_ui_returns_a_full_html_document() {
    let html = cave_portal::ui::embedded_ui();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("</html>"));
    assert!(html.contains("CAVE Platform Portal"));
}

// ── lib: PortalState + router ────────────────────────────────────────────────

#[test]
fn module_name_constant_is_portal() {
    assert_eq!(MODULE_NAME, "portal");
}

#[test]
fn default_portal_state_starts_empty() {
    let s = PortalState::default();
    let svcs = s.services.try_read().expect("uncontended read");
    assert!(svcs.is_empty());
    let cache = s.parity_cache.try_read().expect("uncontended read");
    assert!(cache.is_empty());
}

#[test]
fn router_constructs_without_panic() {
    // Smoke: building the router with an empty State must not panic.
    let _r = router(Arc::new(PortalState::default()));
}

// ── plugins: ViewPersona surface ─────────────────────────────────────────────

#[test]
fn view_persona_labels_are_lowercase_words() {
    assert_eq!(ViewPersona::Tenant.label(), "tenant");
    assert_eq!(ViewPersona::Operator.label(), "operator");
    assert_eq!(ViewPersona::Admin.label(), "admin");
}

#[test]
fn view_persona_equality_distinguishes_variants() {
    assert_ne!(ViewPersona::Tenant, ViewPersona::Operator);
    assert_eq!(ViewPersona::Admin, ViewPersona::Admin);
}

// ── permission / persona / RequestCtx edges ──────────────────────────────────

#[test]
fn persona_from_roles_picks_first_known_match() {
    assert_eq!(Persona::from_roles(&["unrelated", "platform_admin"]), Persona::PlatformAdmin);
    assert_eq!(Persona::from_roles(&["tenant_admin", "platform_admin"]), Persona::TenantAdmin);
    let empty: &[&str] = &[];
    assert_eq!(Persona::from_roles(empty), Persona::Anonymous);
    assert_eq!(Persona::from_roles(&["", "junk"]), Persona::Anonymous);
}

#[test]
fn persona_as_str_round_trip_stable_wire_names() {
    assert_eq!(Persona::PlatformAdmin.as_str(), "platform_admin");
    assert_eq!(Persona::TenantAdmin.as_str(), "tenant_admin");
    assert_eq!(Persona::Anonymous.as_str(), "anonymous");
}

#[test]
fn persona_is_platform_only_true_for_platform_admin() {
    assert!(Persona::PlatformAdmin.is_platform());
    assert!(!Persona::TenantAdmin.is_platform());
    assert!(!Persona::Anonymous.is_platform());
}

#[test]
fn request_ctx_developer_grants_only_its_own_tenant() {
    let ctx = RequestCtx::developer("acme", &[Permission::EtcdRead]);
    assert_eq!(ctx.tenant.as_str(), "acme");
    assert!(ctx.tenant_grants.contains("acme"));
    assert!(!ctx.tenant_grants.contains("evil"));
    assert!(ctx.has_webauthn);
    assert_eq!(ctx.persona, Persona::PlatformAdmin);
}

#[test]
fn request_ctx_authorise_rejects_missing_permission() {
    let ctx = RequestCtx::developer("acme", &[]);
    let err = ctx.authorise(Permission::EtcdRead).unwrap_err();
    matches!(err, AuthError::MissingPermission { missing: "etcd.kv.read" });
}

#[test]
fn request_ctx_authorise_rejects_when_webauthn_missing() {
    let mut ctx = RequestCtx::developer("acme", &[Permission::EtcdRead]);
    ctx.has_webauthn = false;
    let err = ctx.authorise(Permission::EtcdRead).unwrap_err();
    assert!(matches!(err, AuthError::WebAuthnRequired));
}

#[test]
fn request_ctx_require_persona_blocks_tenant_admin_from_platform() {
    let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
    let err = ctx.require_persona(Persona::PlatformAdmin).unwrap_err();
    assert!(matches!(err, AuthError::PersonaForbidden { .. }));
}

#[test]
fn request_ctx_require_persona_platform_can_pass_tenant_gate() {
    let ctx = RequestCtx::developer_as("acme", &[], Persona::PlatformAdmin);
    assert!(ctx.require_persona(Persona::TenantAdmin).is_ok());
    assert!(ctx.require_persona(Persona::PlatformAdmin).is_ok());
}

#[test]
fn auth_error_display_includes_principal_and_tenant() {
    let mut ctx = RequestCtx::developer("acme", &[Permission::EtcdRead]);
    ctx.tenant_grants.clear();
    let err = ctx.authorise(Permission::EtcdRead).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("acme"), "{msg}");
}

#[test]
fn permission_name_returns_stable_dotted_string() {
    assert_eq!(Permission::EtcdRead.name(), "etcd.kv.read");
    assert_eq!(Permission::StreamsAdmin.name(), "streams.topic.admin");
    assert_eq!(Permission::IcebergRead.name(), "iceberg.catalog.read");
}

// ── admin::state free helpers ────────────────────────────────────────────────

#[test]
fn scope_filters_to_target_tenant_only() {
    let acme = tenant("acme");
    let evil = tenant("evil");
    let rows = vec![
        ActivityEntry { tenant: acme.clone(), when_unix: 1, kind: "create", summary: "a".into() },
        ActivityEntry { tenant: evil.clone(), when_unix: 2, kind: "create", summary: "e".into() },
        ActivityEntry { tenant: acme.clone(), when_unix: 3, kind: "delete", summary: "a2".into() },
    ];
    let out = scope(&rows, &acme, |r| &r.tenant);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|r| r.tenant == acme));
}

#[test]
fn scope_empty_input_returns_empty() {
    let acme = tenant("acme");
    let rows: Vec<ActivityEntry> = vec![];
    assert!(scope(&rows, &acme, |r| &r.tenant).is_empty());
}

#[test]
fn tally_by_kind_groups_per_kind_and_skips_other_tenants() {
    let acme = tenant("acme");
    let evil = tenant("evil");
    let rows = vec![
        ActivityEntry { tenant: acme.clone(), when_unix: 1, kind: "create", summary: "".into() },
        ActivityEntry { tenant: acme.clone(), when_unix: 2, kind: "create", summary: "".into() },
        ActivityEntry { tenant: acme.clone(), when_unix: 3, kind: "delete", summary: "".into() },
        ActivityEntry { tenant: evil.clone(), when_unix: 4, kind: "create", summary: "".into() },
    ];
    let acme_tally = tally_by_kind(&rows, &acme);
    assert_eq!(acme_tally.get("create").copied(), Some(2));
    assert_eq!(acme_tally.get("delete").copied(), Some(1));

    let evil_tally = tally_by_kind(&rows, &evil);
    assert_eq!(evil_tally.get("create").copied(), Some(1));
    assert!(evil_tally.get("delete").is_none());
}

#[test]
fn tally_by_kind_no_rows_for_tenant_returns_empty() {
    let nobody = tenant("nobody");
    let acme = tenant("acme");
    let rows = vec![ActivityEntry {
        tenant: acme,
        when_unix: 1,
        kind: "x",
        summary: "".into(),
    }];
    assert!(tally_by_kind(&rows, &nobody).is_empty());
}

// ── admin::streams::connect tenant scoping + auth ───────────────────────────

#[test]
fn streams_connect_inspect_unknown_connector_returns_not_found() {
    let s = AdminState::seeded();
    let ctx = RequestCtx::developer("acme", &[Permission::StreamsRead]);
    let err = connect::inspect_connector(&s, &ctx, "zzzz-missing").unwrap_err();
    assert!(matches!(err, connect::ConnectViewError::ConnectorNotFound(_)));
}

#[test]
fn streams_connect_pause_without_admin_permission_is_refused() {
    let s = AdminState::seeded();
    // Read-only ctx — pause requires StreamsAdmin.
    let ctx = RequestCtx::developer("acme", &[Permission::StreamsRead]);
    let err = connect::pause_connector(&s, &ctx, "anything").unwrap_err();
    assert!(matches!(err, connect::ConnectViewError::Auth(_)));
}

#[test]
fn streams_connect_restart_unknown_task_returns_not_found() {
    let s = AdminState::seeded();
    let ctx = RequestCtx::developer(
        "acme",
        &[Permission::StreamsRead, Permission::StreamsAdmin],
    );
    let err = connect::restart_task(&s, &ctx, "no-such", 99).unwrap_err();
    match err {
        connect::ConnectViewError::TaskNotFound { connector, task } => {
            assert_eq!(connector, "no-such");
            assert_eq!(task, 99);
        }
        other => panic!("expected TaskNotFound, got {other:?}"),
    }
}

// ── catalog ──────────────────────────────────────────────────────────────────

#[test]
fn catalog_schema_sql_contains_expected_tables() {
    assert!(CATALOG_SCHEMA_SQL.contains("CREATE SCHEMA IF NOT EXISTS cave_portal"));
    assert!(CATALOG_SCHEMA_SQL.contains("cave_portal.catalog_entities"));
    assert!(CATALOG_SCHEMA_SQL.contains("cave_portal.catalog_locations"));
    assert!(CATALOG_SCHEMA_SQL.contains("cave_portal.catalog_entity_search"));
    assert!(CATALOG_SCHEMA_SQL.contains("cave_portal.catalog_refresh_state"));
}

#[test]
fn catalog_entity_metadata_defaults_namespace_to_default() {
    // When namespace is omitted from the JSON, serde default fires.
    let json = r#"{ "uid": "u-1", "name": "svc-x" }"#;
    let m: EntityMetadata = serde_json::from_str(json).unwrap();
    assert_eq!(m.namespace, "default");
    assert!(m.title.is_none());
    assert!(m.labels.is_empty());
    assert!(m.tags.is_empty());
}

#[test]
fn catalog_entity_relations_default_to_empty_vec() {
    let json = r#"{
        "api_version": "backstage.io/v1alpha1",
        "kind": "Component",
        "metadata": { "uid": "u-1", "name": "svc-x" }
    }"#;
    let e: Entity = serde_json::from_str(json).unwrap();
    assert!(e.relations.is_empty());
    assert!(e.spec.is_none());
}

#[test]
fn catalog_entity_filter_default_is_open() {
    let f = EntityFilter::default();
    assert!(f.kind.is_none());
    assert!(f.namespace.is_none());
    assert!(f.name.is_none());
    assert!(f.labels.is_empty());
}

#[test]
fn catalog_location_renames_type_to_type() {
    let loc = Location {
        id: "l-1".into(),
        type_: "url".into(),
        target: "https://x".into(),
        presence: Some("required".into()),
    };
    let json = serde_json::to_string(&loc).unwrap();
    assert!(json.contains("\"type\":\"url\""), "{json}");
    // Round-trip.
    let back: Location = serde_json::from_str(&json).unwrap();
    assert_eq!(back.type_, "url");
    assert_eq!(back.presence.as_deref(), Some("required"));
}

#[test]
fn catalog_entity_relation_renames_type_field() {
    let r = EntityRelation { type_: "ownedBy".into(), target_ref: "group:default/payments".into() };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("\"type\":\"ownedBy\""));
    let back: EntityRelation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.target_ref, "group:default/payments");
}

#[tokio::test]
async fn memory_catalog_store_get_missing_returns_none() {
    let store = MemoryCatalogStore::new();
    let got = store.entity_by_ref("component", "default", "missing-svc").await.unwrap();
    assert!(got.is_none());
}

#[tokio::test]
async fn memory_catalog_store_list_locations_starts_empty() {
    let store = MemoryCatalogStore::new();
    let locs = store.locations().await.unwrap();
    assert!(locs.is_empty());
}

// ── admin::*::types serde round-trip + error variants ───────────────────────

#[test]
fn mlflow_experiment_round_trips() {
    let e = MlflowExperiment {
        tenant: tenant("acme"),
        experiment_id: "exp-1".into(),
        name: "fraud-detection".into(),
        artifact_location: "s3://artifacts/exp-1".into(),
        lifecycle_stage: "active".into(),
        creation_time_ms: 1_700_000_000_000,
        last_update_time_ms: 1_700_000_001_000,
    };
    let j = serde_json::to_string(&e).unwrap();
    let back: MlflowExperiment = serde_json::from_str(&j).unwrap();
    assert_eq!(e, back);
}

#[test]
fn mlflow_view_error_display_carries_name() {
    let err = MlflowViewError::ExperimentNotFound("exp-x".into());
    assert!(format!("{err}").contains("exp-x"));
    let err2 = MlflowViewError::RunNotFound("run-y".into());
    assert!(format!("{err2}").contains("run-y"));
    let err3 = MlflowViewError::ModelNotFound("model-z".into());
    assert!(format!("{err3}").contains("model-z"));
}

#[test]
fn keda_scaled_object_detail_round_trips() {
    let d = KedaScaledObjectDetail {
        tenant: tenant("acme"),
        namespace: "default".into(),
        name: "so-1".into(),
        annotations: vec![("a".into(), "b".into())],
        scale_target_ref: KedaScaleTargetRef {
            api_version: "apps/v1".into(),
            kind: "Deployment".into(),
            name: "worker".into(),
            env_source_container_name: None,
        },
        min_replica_count: 0,
        max_replica_count: 10,
        idle_replica_count: None,
        polling_interval_secs: 30,
        cooldown_period_secs: 300,
        initial_cooldown_period_secs: 0,
        fallback: None,
        triggers: vec![KedaTrigger {
            kind: "cpu".into(),
            name: None,
            metadata: vec![("type".into(), "Utilization".into())],
            auth_ref: Some(KedaAuthRef { name: "auth".into(), kind: "TriggerAuthentication".into() }),
            metric_type: "Utilization".into(),
            use_cached_metrics: false,
        }],
        advanced: None,
        status: cave_portal::admin::keda::types::KedaScaledObjectStatus {
            last_active_time: None,
            original_replica_count: 1,
            health: cave_portal::admin::keda::types::KedaHealth {
                overall: "Healthy".into(),
                message: "ok".into(),
            },
            active_triggers: vec![],
            reason: "Idle".into(),
        },
    };
    let j = serde_json::to_string(&d).unwrap();
    let back: KedaScaledObjectDetail = serde_json::from_str(&j).unwrap();
    assert_eq!(d, back);
}

#[test]
fn iceberg_table_fqn_joins_namespace_and_name() {
    let t = IcebergTable {
        tenant: tenant("acme"),
        namespace: "warehouse".into(),
        name: "orders".into(),
        location: "s3://warehouse/orders".into(),
        format_version: 2,
        current_snapshot_id: Some(42),
        schema_id: 1,
        last_updated_ms: 1_700_000_000_000,
        row_count: 1000,
        file_count: 5,
        total_data_files_bytes: 1024,
        partition_spec_id: 0,
    };
    assert_eq!(t.fqn(), "warehouse.orders");
}

#[test]
fn iceberg_view_error_variants_format_payload() {
    let e1 = IcebergViewError::TableNotFound("ns.t".into());
    assert!(format!("{e1}").contains("ns.t"));
    let e2 = IcebergViewError::SnapshotNotFound(99);
    assert!(format!("{e2}").contains("99"));
}

#[test]
fn litellm_model_round_trips_fallback_chain() {
    let m = LiteLlmModel {
        tenant: tenant("acme"),
        name: "gpt-4-tenant-route".into(),
        provider: "openai".into(),
        model_id: "gpt-4".into(),
        status: "active".into(),
        rpm_limit: 60,
        tpm_limit: 90_000,
        fallback_chain: vec!["claude-3".into(), "gpt-3.5".into()],
        created_at_unix: 1_700_000_000,
    };
    let j = serde_json::to_string(&m).unwrap();
    let back: LiteLlmModel = serde_json::from_str(&j).unwrap();
    assert_eq!(m, back);
}

#[test]
fn litellm_api_key_optional_budget_round_trips() {
    let k = LiteLlmApiKey {
        tenant: tenant("acme"),
        key_id: "key-1".into(),
        label: "ci".into(),
        allowed_models: vec!["gpt-4".into()],
        status: "active".into(),
        max_budget_usd_cents: None,
        spent_usd_cents: 1234,
        created_at_unix: 1_700_000_000,
        expires_at_unix: Some(1_800_000_000),
    };
    let j = serde_json::to_string(&k).unwrap();
    let back: LiteLlmApiKey = serde_json::from_str(&j).unwrap();
    assert_eq!(k, back);
    assert!(back.max_budget_usd_cents.is_none());
    assert_eq!(back.expires_at_unix, Some(1_800_000_000));
}

#[test]
fn litellm_view_error_display_carries_identifier() {
    let e = LiteLlmViewError::ModelNotFound("m-1".into());
    assert!(format!("{e}").contains("m-1"));
    let e2 = LiteLlmViewError::RouteNotFound("r-1".into());
    assert!(format!("{e2}").contains("r-1"));
    let e3 = LiteLlmViewError::KeyNotFound("k-1".into());
    assert!(format!("{e3}").contains("k-1"));
}
