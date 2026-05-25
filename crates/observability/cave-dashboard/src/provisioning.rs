// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provisioning — load dashboards, datasources, and alert rules from YAML/JSON files.

use crate::models::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProvisioningError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Invalid provisioning config: {0}")]
    Invalid(String),
}

// ─── Dashboard Provisioning ───────────────────────────────────────────────────

/// Grafana dashboard provisioning config file format.
#[derive(Debug, Deserialize)]
pub struct DashboardProvisioningConfig {
    #[serde(rename = "apiVersion")]
    pub api_version: Option<i32>,
    pub providers: Vec<DashboardProvider>,
}

#[derive(Debug, Deserialize)]
pub struct DashboardProvider {
    pub name: String,
    #[serde(rename = "orgId", default = "default_org_id")]
    pub org_id: i64,
    #[serde(rename = "type", default = "default_provider_type")]
    pub provider_type: String,
    #[serde(rename = "disableDeletion", default)]
    pub disable_deletion: bool,
    #[serde(rename = "updateIntervalSeconds", default = "default_update_interval")]
    pub update_interval_seconds: u64,
    #[serde(rename = "allowUiUpdates", default)]
    pub allow_ui_updates: bool,
    pub options: DashboardProviderOptions,
}

#[derive(Debug, Deserialize)]
pub struct DashboardProviderOptions {
    pub path: String,
    #[serde(rename = "foldersFromFilesStructure", default)]
    pub folders_from_files_structure: bool,
}

fn default_org_id() -> i64 {
    1
}
fn default_provider_type() -> String {
    "file".into()
}
fn default_update_interval() -> u64 {
    10
}

/// A provisioned dashboard wrapper (JSON format that Grafana uses in files).
#[derive(Debug, Deserialize)]
pub struct ProvisionedDashboardFile {
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub refresh: Option<String>,
    #[serde(rename = "schemaVersion", default)]
    pub schema_version: Option<i32>,
    #[serde(default)]
    pub time: Option<serde_json::Value>,
    #[serde(default)]
    pub panels: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub templating: Option<serde_json::Value>,
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
}

/// Parse a provisioning config YAML file.
pub fn parse_provisioning_config(
    yaml: &str,
) -> Result<DashboardProvisioningConfig, ProvisioningError> {
    Ok(serde_yaml::from_str(yaml)?)
}

/// Parse a dashboard JSON file.
pub fn provision_from_json(json: &str) -> Result<serde_json::Value, ProvisioningError> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    // Handle both wrapped {"dashboard": {...}} and bare {...}
    Ok(if v.get("dashboard").is_some() {
        v.get("dashboard").unwrap().clone()
    } else {
        v
    })
}

/// Parse a dashboard YAML file.
pub fn provision_from_yaml(yaml: &str) -> Result<serde_json::Value, ProvisioningError> {
    let v: serde_yaml::Value = serde_yaml::from_str(yaml)?;
    let json_val = serde_json::to_value(&v)?;
    Ok(if json_val.get("dashboard").is_some() {
        json_val.get("dashboard").unwrap().clone()
    } else {
        json_val
    })
}

/// Load all dashboard files from a directory.
pub fn load_dashboards_from_dir(dir: &Path) -> Vec<Result<serde_json::Value, ProvisioningError>> {
    let mut results = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return results;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "json" => {
                let content = std::fs::read_to_string(&path).map_err(ProvisioningError::Io);
                results.push(content.and_then(|s| provision_from_json(&s)));
            }
            "yaml" | "yml" => {
                let content = std::fs::read_to_string(&path).map_err(ProvisioningError::Io);
                results.push(content.and_then(|s| provision_from_yaml(&s)));
            }
            _ => {}
        }
    }
    results
}

// ─── Datasource Provisioning ─────────────────────────────────────────────────

/// Grafana datasource provisioning config file format.
#[derive(Debug, Deserialize)]
pub struct DataSourceProvisioningConfig {
    #[serde(rename = "apiVersion")]
    pub api_version: Option<i32>,
    pub datasources: Vec<ProvisionedDataSource>,
    #[serde(rename = "deleteDatasources", default)]
    pub delete_datasources: Vec<DeleteDataSource>,
}

#[derive(Debug, Deserialize)]
pub struct ProvisionedDataSource {
    pub name: String,
    #[serde(rename = "type")]
    pub ds_type: String,
    pub url: String,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(rename = "orgId", default = "default_org_id")]
    pub org_id: i64,
    #[serde(rename = "isDefault", default)]
    pub is_default: bool,
    #[serde(rename = "basicAuth", default)]
    pub basic_auth: bool,
    #[serde(rename = "basicAuthUser", default)]
    pub basic_auth_user: String,
    #[serde(rename = "jsonData", default)]
    pub json_data: serde_json::Value,
    #[serde(rename = "secureJsonData", default)]
    pub secure_json_data: serde_json::Value,
    #[serde(default)]
    pub access: String,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub user: String,
    #[serde(rename = "editable", default = "default_true")]
    pub editable: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct DeleteDataSource {
    pub name: String,
    #[serde(rename = "orgId", default = "default_org_id")]
    pub org_id: i64,
}

impl ProvisionedDataSource {
    pub fn into_create_request(self) -> CreateDataSourceRequest {
        let ds_type = match self.ds_type.as_str() {
            "prometheus" => DataSourceType::Prometheus,
            "loki" => DataSourceType::Loki,
            "jaeger" => DataSourceType::Jaeger,
            "tempo" => DataSourceType::Tempo,
            "postgres" => DataSourceType::Postgres,
            "elasticsearch" => DataSourceType::Elasticsearch,
            "influxdb" => DataSourceType::InfluxDb,
            "graphite" => DataSourceType::Graphite,
            "mysql" => DataSourceType::Mysql,
            "mssql" => DataSourceType::Mssql,
            _ => DataSourceType::Unknown,
        };
        let access = match self.access.as_str() {
            "direct" | "browser" => DataSourceAccess::Direct,
            _ => DataSourceAccess::Proxy,
        };
        CreateDataSourceRequest {
            name: self.name,
            ds_type,
            url: self.url,
            access,
            is_default: self.is_default,
            json_data: self.json_data,
            uid: self.uid,
            basic_auth: self.basic_auth,
            basic_auth_user: self.basic_auth_user,
            user: self.user,
            database: self.database,
            org_id: Some(self.org_id),
        }
    }
}

/// Parse a datasource provisioning YAML file.
pub fn parse_datasource_provisioning(
    yaml: &str,
) -> Result<DataSourceProvisioningConfig, ProvisioningError> {
    Ok(serde_yaml::from_str(yaml)?)
}

// ─── Alert Rule Provisioning ─────────────────────────────────────────────────

/// Grafana alert rules provisioning config.
#[derive(Debug, Deserialize)]
pub struct AlertRuleProvisioningConfig {
    #[serde(rename = "apiVersion")]
    pub api_version: Option<i32>,
    pub groups: Vec<RuleGroupProvisioning>,
}

#[derive(Debug, Deserialize)]
pub struct RuleGroupProvisioning {
    #[serde(rename = "orgId", default = "default_org_id")]
    pub org_id: i64,
    pub name: String,
    pub folder: String,
    pub interval: String,
    pub rules: Vec<AlertRuleProvisioning>,
}

#[derive(Debug, Deserialize)]
pub struct AlertRuleProvisioning {
    pub uid: String,
    pub title: String,
    pub condition: String,
    pub data: Vec<serde_json::Value>,
    #[serde(rename = "noDataState", default)]
    pub no_data_state: String,
    #[serde(rename = "execErrState", default)]
    pub exec_err_state: String,
    #[serde(rename = "for", default)]
    pub for_duration: String,
    #[serde(default)]
    pub annotations: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
    #[serde(rename = "isPaused", default)]
    pub is_paused: bool,
}

impl AlertRuleProvisioning {
    pub fn into_alert_rule(self, org_id: i64, folder_uid: &str, rule_group: &str) -> AlertRule {
        let no_data_state = match self.no_data_state.as_str() {
            "Alerting" => NoDataState::Alerting,
            "OK" | "Ok" => NoDataState::Ok,
            "KeepState" => NoDataState::KeepState,
            _ => NoDataState::NoData,
        };
        let exec_err_state = match self.exec_err_state.as_str() {
            "OK" | "Ok" => ExecErrState::Ok,
            "Error" => ExecErrState::Error,
            "KeepState" => ExecErrState::KeepState,
            _ => ExecErrState::Alerting,
        };
        let data: Vec<AlertRuleQuery> = self
            .data
            .iter()
            .filter_map(|d| {
                Some(AlertRuleQuery {
                    ref_id: d
                        .get("refId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("A")
                        .to_string(),
                    query_type: d
                        .get("queryType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    relative_time_range: d
                        .get("relativeTimeRange")
                        .and_then(|v| serde_json::from_value::<RelativeTimeRange>(v.clone()).ok())
                        .unwrap_or_default(),
                    datasource_uid: d
                        .get("datasourceUid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    model: d.get("model").cloned().unwrap_or_default(),
                })
            })
            .collect();

        let now = chrono::Utc::now();
        AlertRule {
            id: 0,
            uid: self.uid,
            org_id,
            folder_uid: folder_uid.to_string(),
            rule_group: rule_group.to_string(),
            title: self.title,
            condition: self.condition,
            data,
            no_data_state,
            exec_err_state,
            for_duration: self.for_duration,
            annotations: self.annotations,
            labels: self.labels,
            is_paused: self.is_paused,
            updated: now,
            created: now,
            state: AlertState::Normal,
            health: "ok".into(),
            last_evaluation: None,
            evaluation_time: None,
        }
    }
}

/// Parse an alert rules provisioning YAML file.
pub fn parse_alert_rule_provisioning(
    yaml: &str,
) -> Result<AlertRuleProvisioningConfig, ProvisioningError> {
    Ok(serde_yaml::from_str(yaml)?)
}

// ─── Contact Points Provisioning ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ContactPointProvisioningConfig {
    #[serde(rename = "apiVersion")]
    pub api_version: Option<i32>,
    #[serde(rename = "contactPoints")]
    pub contact_points: Vec<ContactPointProvisioning>,
}

#[derive(Debug, Deserialize)]
pub struct ContactPointProvisioning {
    pub name: String,
    pub receivers: Vec<ContactPointReceiverProvisioning>,
    #[serde(rename = "orgId", default = "default_org_id")]
    pub org_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct ContactPointReceiverProvisioning {
    pub uid: String,
    #[serde(rename = "type")]
    pub cp_type: String,
    pub settings: serde_json::Value,
    #[serde(rename = "disableResolveMessage", default)]
    pub disable_resolve_message: bool,
}

impl ContactPointProvisioning {
    pub fn into_contact_points(self) -> Vec<ContactPoint> {
        self.receivers
            .into_iter()
            .map(|r| {
                let cp_type = match r.cp_type.as_str() {
                    "webhook" => ContactPointType::Webhook,
                    "email" => ContactPointType::Email,
                    "slack" => ContactPointType::Slack,
                    "pagerduty" => ContactPointType::PagerDuty,
                    "opsgenie" => ContactPointType::Opsgenie,
                    "telegram" => ContactPointType::Telegram,
                    "teams" => ContactPointType::Teams,
                    _ => ContactPointType::Webhook,
                };
                ContactPoint {
                    uid: r.uid,
                    name: self.name.clone(),
                    cp_type,
                    settings: r.settings,
                    disable_resolve_message: r.disable_resolve_message,
                    send_reminder: false,
                    frequency: String::new(),
                }
            })
            .collect()
    }
}

pub fn parse_contact_point_provisioning(
    yaml: &str,
) -> Result<ContactPointProvisioningConfig, ProvisioningError> {
    Ok(serde_yaml::from_str(yaml)?)
}

// ─── Notification Policy Provisioning ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct NotificationPolicyProvisioningConfig {
    #[serde(rename = "apiVersion")]
    pub api_version: Option<i32>,
    pub policies: Vec<NotificationPolicyProvisioning>,
}

#[derive(Debug, Deserialize)]
pub struct NotificationPolicyProvisioning {
    #[serde(rename = "orgId", default = "default_org_id")]
    pub org_id: i64,
    #[serde(flatten)]
    pub policy: serde_json::Value,
}

pub fn parse_notification_policy_provisioning(
    yaml: &str,
) -> Result<NotificationPolicyProvisioningConfig, ProvisioningError> {
    Ok(serde_yaml::from_str(yaml)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provision_from_json_bare() {
        let json = r#"{"title":"My Dashboard","panels":[],"schemaVersion":38}"#;
        let result = provision_from_json(json).unwrap();
        assert_eq!(result["title"], "My Dashboard");
    }

    #[test]
    fn test_provision_from_json_wrapped() {
        let json = r#"{"dashboard":{"title":"Wrapped","panels":[]}}"#;
        let result = provision_from_json(json).unwrap();
        assert_eq!(result["title"], "Wrapped");
    }

    #[test]
    fn test_provision_from_yaml() {
        let yaml = "title: YAML Dashboard\npanels: []\nschemaVersion: 38\n";
        let result = provision_from_yaml(yaml).unwrap();
        assert_eq!(result["title"], "YAML Dashboard");
    }

    #[test]
    fn test_provision_from_yaml_wrapped() {
        let yaml = "dashboard:\n  title: Wrapped YAML\n  panels: []\n";
        let result = provision_from_yaml(yaml).unwrap();
        assert_eq!(result["title"], "Wrapped YAML");
    }

    #[test]
    fn test_parse_datasource_provisioning() {
        let yaml = r#"
apiVersion: 1
datasources:
  - name: Prometheus
    type: prometheus
    url: http://localhost:9090
    isDefault: true
    orgId: 1
"#;
        let config = parse_datasource_provisioning(yaml).unwrap();
        assert_eq!(config.datasources.len(), 1);
        assert_eq!(config.datasources[0].name, "Prometheus");
        assert!(config.datasources[0].is_default);
    }

    #[test]
    fn test_parse_alert_rule_provisioning() {
        let yaml = r#"
apiVersion: 1
groups:
  - orgId: 1
    name: my-group
    folder: General
    interval: 1m
    rules:
      - uid: abc123
        title: High CPU
        condition: C
        data: []
        for: 5m
"#;
        let config = parse_alert_rule_provisioning(yaml).unwrap();
        assert_eq!(config.groups.len(), 1);
        assert_eq!(config.groups[0].rules[0].title, "High CPU");
    }
}
