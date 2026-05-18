// SPDX-License-Identifier: AGPL-3.0-or-later
//! /api/v1/rules and /api/v1/alerts

use axum::{extract::State, Json};
use std::sync::Arc;
use crate::state::MetricsState;

pub async fn list_rules(State(state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    let groups: Vec<serde_json::Value> = state.rule_groups.read().iter().map(|group| {
        let recording: Vec<serde_json::Value> = group.recording_rules.iter().map(|r| {
            serde_json::json!({
                "type": "recording",
                "name": r.name,
                "query": r.expr,
                "labels": r.labels.0,
                "evaluationTime": 0.0,
                "lastEvaluation": "",
                "health": "ok",
                "lastError": "",
            })
        }).collect();

        let alerting: Vec<serde_json::Value> = group.alert_rules.iter().map(|r| {
            let alerts: Vec<serde_json::Value> = r.active.iter().map(|(fp, (state, ts))| {
                serde_json::json!({
                    "labels": {},
                    "annotations": r.annotations.0,
                    "state": format!("{:?}", state).to_lowercase(),
                    "activeAt": chrono::DateTime::<chrono::Utc>::from_timestamp_millis(*ts)
                        .map(|d| d.to_rfc3339()).unwrap_or_default(),
                    "value": "0",
                })
            }).collect();

            serde_json::json!({
                "type": "alerting",
                "name": r.name,
                "query": r.expr,
                "duration": r.for_ms as f64 / 1000.0,
                "labels": r.labels.0,
                "annotations": r.annotations.0,
                "alerts": alerts,
                "health": "ok",
                "lastError": "",
                "evaluationTime": 0.0,
                "lastEvaluation": "",
            })
        }).collect();

        let mut rules = recording;
        rules.extend(alerting);

        serde_json::json!({
            "name": group.name,
            "file": "",
            "rules": rules,
            "interval": group.interval.as_secs_f64(),
            "limit": 0,
            "evaluationTime": 0.0,
            "lastEvaluation": "",
        })
    }).collect();

    Json(serde_json::json!({
        "status": "success",
        "data": { "groups": groups }
    }))
}

pub async fn list_alerts(State(state): State<Arc<MetricsState>>) -> Json<serde_json::Value> {
    let alerts: Vec<serde_json::Value> = state.rule_groups.read().iter()
        .flat_map(|group| {
            group.alert_rules.iter().flat_map(|rule| {
                rule.active.iter().map(|(_, (alert_state, ts))| {
                    serde_json::json!({
                        "labels": rule.labels.0,
                        "annotations": rule.annotations.0,
                        "state": format!("{:?}", alert_state).to_lowercase(),
                        "activeAt": chrono::DateTime::<chrono::Utc>::from_timestamp_millis(*ts)
                            .map(|d| d.to_rfc3339()).unwrap_or_default(),
                        "value": "0",
                    })
                }).collect::<Vec<_>>()
            }).collect::<Vec<_>>()
        })
        .collect();

    Json(serde_json::json!({
        "status": "success",
        "data": { "alerts": alerts }
    }))
}
