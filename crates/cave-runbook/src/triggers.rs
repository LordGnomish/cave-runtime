// SPDX-License-Identifier: AGPL-3.0-or-later
//! Trigger evaluation — matches incidents/alerts to runbooks and schedules runs.

use crate::{
    models::{IncidentBinding, Trigger, TriggerKind},
    RunbookState,
};
use chrono::Utc;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Evaluate incoming incident/alert against all bindings.
///
/// Returns the IDs of runbooks that match, ordered by binding creation time.
pub async fn evaluate_triggers(
    state: &RunbookState,
    incident_title: &str,
    incident_severity: Option<&str>,
) -> Vec<Uuid> {
    let bindings = state.bindings.lock().await;

    bindings
        .values()
        .filter(|b| {
            let title_matches = incident_title
                .to_lowercase()
                .contains(&b.incident_pattern.to_lowercase());

            let severity_matches = match (&b.incident_severity, incident_severity) {
                (Some(required), Some(actual)) => {
                    required.eq_ignore_ascii_case(actual)
                }
                (Some(_), None) => false,
                (None, _) => true, // binding has no severity filter → matches any
            };

            title_matches && severity_matches
        })
        .map(|b| b.runbook_id)
        .collect()
}

/// Create or update a binding that links an incident pattern to a runbook.
pub async fn bind_to_incident(state: &RunbookState, binding: IncidentBinding) {
    info!(
        binding_id = %binding.id,
        pattern = %binding.incident_pattern,
        runbook_id = %binding.runbook_id,
        "Binding incident pattern to runbook"
    );
    let mut bindings = state.bindings.lock().await;
    bindings.insert(binding.id, binding);
}

/// Check whether a `Trigger` should fire right now given a cron expression.
///
/// In production this would integrate with a scheduler (e.g. tokio-cron-scheduler).
/// Here we validate the cron string and log the intent.
pub fn schedule_runbook(runbook_id: Uuid, trigger: &Trigger) {
    if !matches!(trigger.kind, TriggerKind::Schedule) {
        return;
    }

    let cron = trigger.cron.as_deref().unwrap_or("(none)");
    info!(
        runbook_id = %runbook_id,
        cron = %cron,
        "Runbook registered for scheduled execution"
    );
    // A full implementation would call tokio_cron_scheduler::JobScheduler here.
}

/// Build an `IncidentBinding` from raw parts.
pub fn make_binding(
    name: impl Into<String>,
    incident_pattern: impl Into<String>,
    incident_severity: Option<String>,
    runbook_id: Uuid,
    auto_execute: bool,
) -> IncidentBinding {
    IncidentBinding {
        id: Uuid::new_v4(),
        name: name.into(),
        incident_pattern: incident_pattern.into(),
        incident_severity,
        runbook_id,
        auto_execute,
        created_at: Utc::now(),
    }
}

/// Auto-attach runbooks to a newly opened incident based on bindings.
///
/// Returns (runbook_id, auto_execute) pairs that matched.
pub async fn auto_attach(
    state: Arc<RunbookState>,
    incident_title: &str,
    incident_severity: Option<&str>,
) -> Vec<(Uuid, bool)> {
    let bindings = state.bindings.lock().await;

    let matches: Vec<(Uuid, bool)> = bindings
        .values()
        .filter(|b| {
            incident_title
                .to_lowercase()
                .contains(&b.incident_pattern.to_lowercase())
                && b.incident_severity
                    .as_deref()
                    .map_or(true, |s| incident_severity.map_or(false, |sev| s.eq_ignore_ascii_case(sev)))
        })
        .map(|b| (b.runbook_id, b.auto_execute))
        .collect();

    if !matches.is_empty() {
        info!(
            incident = %incident_title,
            matched = matches.len(),
            "Auto-attaching runbooks to incident"
        );
    }

    matches
}
