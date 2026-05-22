// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Data models for cave-portal.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Service {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub team: String,
    pub tier: ServiceTier,
    pub language: String,
    pub repo_url: String,
    pub tags: Vec<String>,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTier {
    Tier1,
    Tier2,
    Tier3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceLink {
    pub service_id: Uuid,
    pub link_type: LinkType,
    pub url: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LinkType {
    RunBook,
    Dashboard,
    Docs,
    Repo,
    Chat,
}

/// Overall health of a module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// A card on the main dashboard representing one module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardWidget {
    pub module: String,
    pub display_name: String,
    pub health: HealthStatus,
    pub key_metric_label: String,
    pub key_metric_value: String,
    pub link: String,
    pub upstream_replacement: String,
    pub category: String,
}

/// One entry in the sidebar navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationItem {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub path: String,
    pub category: String,
    pub upstream_replacement: String,
    pub badge_count: Option<u32>,
}

/// A grouped section of sidebar navigation items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationGroup {
    pub label: String,
    pub icon: String,
    pub items: Vec<NavigationItem>,
}

/// One hit from a global search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub module: String,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub link: String,
    pub relevance: f32,
}

/// Per-user portal preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreference {
    pub user_id: Uuid,
    pub theme: String,
    pub sidebar_collapsed: bool,
    pub pinned_modules: Vec<String>,
    pub notification_modules: Vec<String>,
    pub dashboard_layout: String,
}

impl Default for UserPreference {
    fn default() -> Self {
        Self {
            user_id: Uuid::nil(),
            theme: "dark".to_string(),
            sidebar_collapsed: false,
            pinned_modules: vec![],
            notification_modules: vec![],
            dashboard_layout: "grid".to_string(),
        }
    }
}

/// Severity level for a cross-module notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Critical,
}

/// A notification surfaced from any CAVE module into the portal feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: Uuid,
    pub module: String,
    pub title: String,
    pub body: String,
    pub severity: NotificationSeverity,
    pub created_at: DateTime<Utc>,
    pub read: bool,
    pub link: Option<String>,
}

/// Quick stats for a single module, shown in the modules listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub module: String,
    pub display_name: String,
    pub health: HealthStatus,
    pub upstream_replacement: String,
    pub category: String,
    pub stats: serde_json::Value,
}

/// Aggregated dashboard payload returned by GET /api/v1/portal/dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub modules: Vec<DashboardWidget>,
    pub total_modules: usize,
    pub healthy_count: usize,
    pub degraded_count: usize,
    pub unhealthy_count: usize,
    pub unknown_count: usize,
    pub generated_at: DateTime<Utc>,
}
