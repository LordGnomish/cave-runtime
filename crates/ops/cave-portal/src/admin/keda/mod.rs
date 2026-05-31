// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/keda` views — KEDA ScaledObject, ScaledJob, TriggerAuthentication,
//! scaler-catalog browser, and per-scaler metrics.
//!
//! Ported from upstream `kedacore/keda` v2.14:
//! * `apis/keda/v1alpha1/scaledobject_types.go`
//! * `apis/keda/v1alpha1/scaledjob_types.go`
//! * `apis/keda/v1alpha1/triggerauthentication_types.go`
//! * `pkg/scalers/*` for the catalog rows.
//!
//! The top-level overview (this module) shows the per-tenant ScaledObject
//! table + scaling-event tail. The sub-modules below render the dedicated
//! CRUD pages and the catalog.

pub mod metrics;
pub mod modifiers;
pub mod scaled_jobs;
pub mod scaled_objects;
pub mod scalers;
pub mod trigger_authentications;
pub mod types;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, KedaScaledObject, KedaScalerEvent, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KedaViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("scaled object {0} not found")]
    ScaledObjectNotFound(String),
}

pub fn list_scaled_objects(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<KedaScaledObject>, KedaViewError> {
    ctx.authorise(Permission::KedaRead)?;
    Ok(scope(
        &state.keda_scaled_objects.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect())
}

pub fn list_scaler_events(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<KedaScalerEvent>, KedaViewError> {
    ctx.authorise(Permission::KedaRead)?;
    Ok(scope(
        &state.keda_scaler_events.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect())
}

pub fn pause_scaled_object(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), KedaViewError> {
    ctx.authorise(Permission::KedaWrite)?;
    let mut sos = state.keda_scaled_objects.write().unwrap();
    let target = sos
        .iter_mut()
        .find(|s| s.tenant == ctx.tenant && s.name == name)
        .ok_or_else(|| KedaViewError::ScaledObjectNotFound(name.into()))?;
    target.paused = true;
    Ok(())
}

pub fn resume_scaled_object(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), KedaViewError> {
    ctx.authorise(Permission::KedaWrite)?;
    let mut sos = state.keda_scaled_objects.write().unwrap();
    let target = sos
        .iter_mut()
        .find(|s| s.tenant == ctx.tenant && s.name == name)
        .ok_or_else(|| KedaViewError::ScaledObjectNotFound(name.into()))?;
    target.paused = false;
    Ok(())
}

/// Per-trigger-type tally for the metrics summary panel.
pub fn trigger_breakdown(sos: &[KedaScaledObject]) -> Vec<(String, u32)> {
    let mut counts: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for s in sos {
        for t in &s.triggers {
            *counts.entry(t.clone()).or_insert(0) += 1;
        }
    }
    counts.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KedaViewError> {
    let sos = list_scaled_objects(state, ctx)?;
    let events = list_scaler_events(state, ctx)?;
    let triggers = trigger_breakdown(&sos);

    let so_rows: Vec<Vec<String>> = sos
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.target_ref.clone(),
                format!(
                    "{}/{}/{}",
                    s.min_replicas, s.current_replicas, s.max_replicas
                ),
                if s.paused {
                    "paused".into()
                } else {
                    "active".into()
                },
                s.triggers.join(","),
            ]
        })
        .collect();

    let trigger_rows: Vec<Vec<String>> = triggers
        .iter()
        .map(|(t, n)| vec![t.clone(), n.to_string()])
        .collect();

    let event_rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            vec![
                e.when_unix.to_string(),
                e.scaled_object.clone(),
                e.trigger.clone(),
                format!("{} → {}", e.from_replicas, e.to_replicas),
                e.verdict.into(),
            ]
        })
        .collect();

    let body = format!(
        r#"<nav class="mb-4 text-sm flex gap-3">
<a class="underline text-blue-700" href="/admin/keda/scalers?tenant_id={tenant}">scaler catalog</a>
<a class="underline text-blue-700" href="/admin/keda/metrics?tenant_id={tenant}">metrics</a>
<a class="underline text-blue-700" href="/admin/keda/modifiers?tenant_id={tenant}">formula playground</a>
</nav>
<section><h2 class="text-lg font-semibold mb-2">ScaledObjects ({n_so})</h2>{so_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Trigger types in use</h2>{trigger_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Scale events ({n_ev})</h2>{event_tbl}</section>"#,
        tenant = escape(ctx.tenant.as_str()),
        n_so = sos.len(),
        n_ev = events.len(),
        so_tbl = table(
            &["name", "target", "min/current/max", "state", "triggers"],
            &so_rows
        ),
        trigger_tbl = table(&["trigger", "count"], &trigger_rows),
        event_tbl = table(
            &[
                "when_unix",
                "scaled_object",
                "trigger",
                "replicas",
                "verdict"
            ],
            &event_rows
        ),
    );

    Ok(page_shell_full(
        ctx,
        "/admin/keda",
        &format!("keda · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
    "AutoscalingTab",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_scaled_objects_filters_to_owner() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
            "ScaledObjectTable",
            "acme"
        );
        let state = AdminState::seeded();
        let sos = list_scaled_objects(&state, &ctx(&[Permission::KedaRead])).unwrap();
        assert_eq!(sos.len(), 2);
        assert!(sos.iter().all(|s| s.tenant.as_str() == "acme"));
        assert!(!sos.iter().any(|s| s.name == "evil-worker"));
    }

    #[test]
    fn list_scaler_events_filters_to_owner() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
            "ScaleEventLog",
            "acme"
        );
        let state = AdminState::seeded();
        let events = list_scaler_events(&state, &ctx(&[Permission::KedaRead])).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.tenant.as_str() == "acme"));
    }

    #[test]
    fn pause_then_resume_round_trips() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
            "PauseToggle",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::KedaRead, Permission::KedaWrite]);
        pause_scaled_object(&state, &c, "ingest-worker").unwrap();
        let sos = list_scaled_objects(&state, &c).unwrap();
        assert!(
            sos.iter()
                .find(|s| s.name == "ingest-worker")
                .unwrap()
                .paused
        );
        resume_scaled_object(&state, &c, "ingest-worker").unwrap();
        let sos = list_scaled_objects(&state, &c).unwrap();
        assert!(
            !sos.iter()
                .find(|s| s.name == "ingest-worker")
                .unwrap()
                .paused
        );
    }

    #[test]
    fn pause_unknown_scaled_object_errors() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
            "PauseToggle",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::KedaRead, Permission::KedaWrite]);
        let err = pause_scaled_object(&state, &c, "nope").unwrap_err();
        assert!(matches!(err, KedaViewError::ScaledObjectNotFound(_)));
    }

    #[test]
    fn read_without_permission_is_refused() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
            "PermissionGate",
            "acme"
        );
        let state = AdminState::seeded();
        let err = list_scaled_objects(&state, &ctx(&[])).unwrap_err();
        assert!(matches!(err, KedaViewError::Auth(_)));
    }

    #[test]
    fn write_without_permission_is_refused() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
            "PermissionGate",
            "acme"
        );
        let state = AdminState::seeded();
        let err = pause_scaled_object(&state, &ctx(&[Permission::KedaRead]), "ingest-worker")
            .unwrap_err();
        assert!(matches!(err, KedaViewError::Auth(_)));
    }

    #[test]
    fn trigger_breakdown_counts_each_trigger_once_per_so() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/AutoscalingTab.tsx",
            "TriggerSummary",
            "acme"
        );
        let state = AdminState::seeded();
        let sos = list_scaled_objects(&state, &ctx(&[Permission::KedaRead])).unwrap();
        let breakdown = trigger_breakdown(&sos);
        // ingest-worker → kafka + prometheus; report-runner → cron
        let map: std::collections::HashMap<String, u32> = breakdown.into_iter().collect();
        assert_eq!(map.get("kafka"), Some(&1));
        assert_eq!(map.get("prometheus"), Some(&1));
        assert_eq!(map.get("cron"), Some(&1));
        assert!(map.get("cpu").is_none()); // only on the evil tenant
    }
}
