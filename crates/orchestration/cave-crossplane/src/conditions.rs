// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Condition propagation — Ready / Synced / Healthy from composed → XR → claim.
//!
//! Upstream: pkg/resource/condition.go +
//!           internal/controller/apiextensions/composite/composed.go::PropagateConditions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionType {
    Ready,
    Synced,
    Healthy,
}

impl ConditionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConditionType::Ready => "Ready",
            ConditionType::Synced => "Synced",
            ConditionType::Healthy => "Healthy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

impl ConditionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConditionStatus::True => "True",
            ConditionStatus::False => "False",
            ConditionStatus::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub condition_type: ConditionType,
    pub status: ConditionStatus,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition_time: DateTime<Utc>,
}

impl Condition {
    pub fn new(condition_type: ConditionType, status: ConditionStatus) -> Self {
        Self {
            condition_type,
            status,
            reason: None,
            message: None,
            last_transition_time: Utc::now(),
        }
    }

    pub fn with_reason(mut self, r: impl Into<String>) -> Self {
        self.reason = Some(r.into());
        self
    }

    pub fn with_message(mut self, m: impl Into<String>) -> Self {
        self.message = Some(m.into());
        self
    }

    /// Convert to upstream Kubernetes JSON shape.
    pub fn to_json(&self) -> Value {
        json!({
            "type": self.condition_type.as_str(),
            "status": self.status.as_str(),
            "reason": self.reason,
            "message": self.message,
            "lastTransitionTime": self.last_transition_time.to_rfc3339(),
        })
    }
}

/// Aggregate conditions from composed resources up into an XR status block.
pub fn propagate_composed_to_xr(xr: &Value, composed: &[Value]) -> Value {
    let mut xr = xr.clone();
    let all_ready = !composed.is_empty()
        && composed
            .iter()
            .all(|c| has_condition_with(c, "Ready", "True"));
    let all_synced = !composed.is_empty()
        && composed
            .iter()
            .all(|c| has_condition_with(c, "Synced", "True"));
    let any_unhealthy = composed
        .iter()
        .any(|c| has_condition_with(c, "Healthy", "False"));
    let ready_cond = Condition::new(
        ConditionType::Ready,
        if all_ready {
            ConditionStatus::True
        } else {
            ConditionStatus::False
        },
    );
    let synced_cond = Condition::new(
        ConditionType::Synced,
        if all_synced {
            ConditionStatus::True
        } else {
            ConditionStatus::Unknown
        },
    );
    let healthy_cond = Condition::new(
        ConditionType::Healthy,
        if any_unhealthy {
            ConditionStatus::False
        } else {
            ConditionStatus::True
        },
    );
    let new_conds = vec![
        ready_cond.to_json(),
        synced_cond.to_json(),
        healthy_cond.to_json(),
    ];
    set_conditions(&mut xr, new_conds);
    xr
}

/// Propagate XR conditions onto a bound claim.
pub fn propagate_xr_to_claim(claim: &Value, xr: &Value) -> Value {
    let mut claim = claim.clone();
    let xr_conds = xr
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    set_conditions(&mut claim, xr_conds);
    claim
}

fn has_condition_with(resource: &Value, ctype: &str, status: &str) -> bool {
    resource
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter().any(|cd| {
                cd.get("type").and_then(|v| v.as_str()) == Some(ctype)
                    && cd.get("status").and_then(|v| v.as_str()) == Some(status)
            })
        })
        .unwrap_or(false)
}

fn set_conditions(resource: &mut Value, conditions: Vec<Value>) {
    if !resource.is_object() {
        *resource = Value::Object(serde_json::Map::new());
    }
    let obj = resource.as_object_mut().unwrap();
    let status = obj
        .entry("status".to_string())
        .or_insert(Value::Object(serde_json::Map::new()))
        .as_object_mut();
    if let Some(s) = status {
        s.insert("conditions".to_string(), Value::Array(conditions));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond_resource(ready: &str, synced: &str, healthy: &str) -> Value {
        json!({
            "status":{
                "conditions":[
                    {"type":"Ready","status":ready},
                    {"type":"Synced","status":synced},
                    {"type":"Healthy","status":healthy}
                ]
            }
        })
    }

    #[test]
    fn condition_serialize_includes_type() {
        let c = Condition::new(ConditionType::Ready, ConditionStatus::True);
        let v = c.to_json();
        assert_eq!(v["type"], json!("Ready"));
        assert_eq!(v["status"], json!("True"));
    }

    #[test]
    fn condition_with_reason_message() {
        let c = Condition::new(ConditionType::Synced, ConditionStatus::False)
            .with_reason("ReconcileError")
            .with_message("could not patch");
        let v = c.to_json();
        assert_eq!(v["reason"], json!("ReconcileError"));
    }

    #[test]
    fn propagate_all_ready() {
        let composed = vec![
            cond_resource("True", "True", "True"),
            cond_resource("True", "True", "True"),
        ];
        let xr = propagate_composed_to_xr(&json!({}), &composed);
        let conds = xr["status"]["conditions"].as_array().unwrap();
        let ready = conds.iter().find(|c| c["type"] == "Ready").unwrap();
        assert_eq!(ready["status"], json!("True"));
    }

    #[test]
    fn propagate_one_unready_makes_false() {
        let composed = vec![
            cond_resource("True", "True", "True"),
            cond_resource("False", "True", "True"),
        ];
        let xr = propagate_composed_to_xr(&json!({}), &composed);
        let ready = xr["status"]["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["type"] == "Ready")
            .unwrap()
            .clone();
        assert_eq!(ready["status"], json!("False"));
    }

    #[test]
    fn propagate_unhealthy_emits_false() {
        let composed = vec![cond_resource("True", "True", "False")];
        let xr = propagate_composed_to_xr(&json!({}), &composed);
        let h = xr["status"]["conditions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["type"] == "Healthy")
            .unwrap()
            .clone();
        assert_eq!(h["status"], json!("False"));
    }

    #[test]
    fn propagate_empty_makes_ready_false() {
        let xr = propagate_composed_to_xr(&json!({}), &[]);
        let conds = xr["status"]["conditions"].as_array().unwrap();
        assert!(conds
            .iter()
            .any(|c| c["type"] == "Ready" && c["status"] == "False"));
    }

    #[test]
    fn propagate_xr_to_claim_copies_conditions() {
        let xr = json!({
            "status":{"conditions":[{"type":"Ready","status":"True"}]}
        });
        let claim = propagate_xr_to_claim(&json!({}), &xr);
        assert_eq!(claim["status"]["conditions"][0]["type"], json!("Ready"));
    }

    #[test]
    fn condition_status_strings() {
        assert_eq!(ConditionStatus::True.as_str(), "True");
        assert_eq!(ConditionStatus::False.as_str(), "False");
        assert_eq!(ConditionStatus::Unknown.as_str(), "Unknown");
    }

    #[test]
    fn condition_type_strings() {
        assert_eq!(ConditionType::Healthy.as_str(), "Healthy");
        assert_eq!(ConditionType::Synced.as_str(), "Synced");
    }

    #[test]
    fn propagate_to_claim_empty_xr() {
        let claim = propagate_xr_to_claim(&json!({"k":"v"}), &json!({}));
        assert!(claim["status"]["conditions"].as_array().unwrap().is_empty());
    }
}
