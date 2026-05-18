// SPDX-License-Identifier: AGPL-3.0-or-later
//! AlertManager data model.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::model::Labels;

/// An alert sent to AlertManager.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub starts_at: String,      // RFC3339
    pub ends_at: Option<String>,
    pub generator_url: String,
    pub fingerprint: String,
}

impl Alert {
    pub fn from_firing(alert: &crate::rules::FiringAlert, generator_url: &str) -> Self {
        use chrono::{DateTime, Utc};
        let starts_at = DateTime::<Utc>::from_timestamp_millis(alert.active_at_ms)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default();
        let ends_at = if alert.state == crate::rules::AlertState::Inactive {
            Some(starts_at.clone())
        } else {
            None
        };

        Alert {
            labels: alert.labels.0.clone().into_iter().collect(),
            annotations: alert.annotations.0.clone().into_iter().collect(),
            starts_at,
            ends_at,
            generator_url: generator_url.to_string(),
            fingerprint: format!("{:016x}", alert.labels.fingerprint()),
        }
    }
}

/// A group of alerts (AlertManager API v2 format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertGroup {
    pub labels: HashMap<String, String>,
    pub receiver: Receiver,
    pub alerts: Vec<Alert>,
}

/// Receiver configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receiver {
    pub name: String,
}

/// Routing tree node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub receiver: String,
    pub group_by: Vec<String>,
    pub group_wait: Option<String>,
    pub group_interval: Option<String>,
    pub repeat_interval: Option<String>,
    pub matchers: Vec<RouteMatcher>,
    pub routes: Vec<Route>,
    pub continue_matching: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteMatcher {
    pub name: String,
    pub value: String,
    pub is_regex: bool,
    pub is_equal: bool,
}

/// Silence definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Silence {
    pub id: String,
    pub matchers: Vec<SilenceMatcher>,
    pub starts_at: String,
    pub ends_at: String,
    pub created_by: String,
    pub comment: String,
    pub status: SilenceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilenceMatcher {
    pub name: String,
    pub value: String,
    pub is_regex: bool,
    pub is_equal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SilenceStatus {
    pub state: String, // "active", "pending", "expired"
}

/// Inhibition rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InhibitRule {
    pub source_matchers: Vec<SilenceMatcher>,
    pub target_matchers: Vec<SilenceMatcher>,
    pub equal: Vec<String>,
}

impl SilenceMatcher {
    pub fn matches(&self, labels: &Labels) -> bool {
        let val = labels.get(&self.name).unwrap_or("");
        if self.is_regex {
            let anchored = format!("^(?:{})$", self.value);
            regex::Regex::new(&anchored).map(|r| {
                let matched = r.is_match(val);
                if self.is_equal { matched } else { !matched }
            }).unwrap_or(false)
        } else if self.is_equal {
            val == self.value
        } else {
            val != self.value
        }
    }
}

impl Silence {
    /// Check if this silence matches the given labels.
    pub fn matches(&self, labels: &Labels) -> bool {
        self.matchers.iter().all(|m| m.matches(labels))
    }

    /// Check if this silence is currently active.
    pub fn is_active(&self, now_rfc3339: &str) -> bool {
        self.status.state == "active"
            && self.starts_at.as_str() <= now_rfc3339
            && self.ends_at.as_str() > now_rfc3339
    }
}
