//! Dashboard provisioning — load dashboards from JSON / YAML files.

use thiserror::Error;

use crate::models::{Dashboard, TimeRange};

#[derive(Debug, Error)]
pub enum ProvisioningError {
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Missing required field: {0}")]
    MissingField(String),
}

/// A Grafana-compatible provisioning JSON structure.
///
/// The minimal required fields for importing a dashboard.
#[derive(Debug, serde::Deserialize)]
pub struct ProvisionedDashboard {
    pub uid: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub refresh: Option<String>,
    pub schema_version: Option<u32>,
    pub time: Option<ProvisionedTimeRange>,
    pub panels: Option<Vec<serde_json::Value>>,
    pub templating: Option<ProvisionedTemplating>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ProvisionedTimeRange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct ProvisionedTemplating {
    pub list: Option<Vec<serde_json::Value>>,
}

/// Parse a Grafana-compatible JSON dashboard definition.
pub fn provision_from_json(json: &str) -> Result<Dashboard, ProvisioningError> {
    let raw: ProvisionedDashboard = serde_json::from_str(json)?;
    build_dashboard(raw)
}

/// Parse a YAML dashboard definition (Grafana YAML provisioning format wraps the dashboard).
pub fn provision_from_yaml(yaml: &str) -> Result<Dashboard, ProvisioningError> {
    // Accept either a bare dashboard YAML or a Grafana provisioning file
    // where the dashboard is under the "dashboard" key.
    let value: serde_yaml::Value = serde_yaml::from_str(yaml)?;
    let dash_value = if value.get("dashboard").is_some() {
        value["dashboard"].clone()
    } else {
        value
    };
    let json = serde_json::to_string(&dash_value)?;
    let raw: ProvisionedDashboard = serde_json::from_str(&json)?;
    build_dashboard(raw)
}

fn build_dashboard(raw: ProvisionedDashboard) -> Result<Dashboard, ProvisioningError> {
    if raw.title.is_empty() {
        return Err(ProvisioningError::MissingField("title".to_string()));
    }

    let uid = raw.uid.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let time = raw
        .time
        .map(|t| TimeRange { from: t.from, to: t.to })
        .unwrap_or_default();

    let mut d = Dashboard::new(raw.title);
    d.uid = uid;
    d.description = raw.description.unwrap_or_default();
    d.tags = raw.tags.unwrap_or_default();
    d.refresh = raw.refresh.unwrap_or_default();
    d.schema_version = raw.schema_version.unwrap_or(38);
    d.time = time;

    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provision_from_json_minimal() {
        let json = r#"{"title": "My Dashboard"}"#;
        let d = provision_from_json(json).expect("should parse");
        assert_eq!(d.title, "My Dashboard");
        assert!(!d.uid.is_empty());
    }

    #[test]
    fn test_provision_from_json_full() {
        let json = r#"{
            "uid": "abc123",
            "title": "Full Dashboard",
            "description": "A test dashboard",
            "tags": ["infra", "cave"],
            "refresh": "30s",
            "schemaVersion": 38,
            "time": {"from": "now-1h", "to": "now"}
        }"#;
        let d = provision_from_json(json).expect("should parse");
        assert_eq!(d.uid, "abc123");
        assert_eq!(d.title, "Full Dashboard");
        assert_eq!(d.tags, vec!["infra", "cave"]);
        assert_eq!(d.time.from, "now-1h");
    }

    #[test]
    fn test_provision_from_json_missing_title() {
        let json = r#"{"uid": "no-title"}"#;
        let err = provision_from_json(json).expect_err("should fail");
        assert!(matches!(err, ProvisioningError::Json(_) | ProvisioningError::MissingField(_)));
    }

    #[test]
    fn test_provision_from_yaml() {
        let yaml = "title: YAML Dashboard\ntags:\n  - ops\nrefresh: \"1m\"\n";
        let d = provision_from_yaml(yaml).expect("should parse YAML");
        assert_eq!(d.title, "YAML Dashboard");
        assert_eq!(d.tags, vec!["ops"]);
        assert_eq!(d.refresh, "1m");
    }

    #[test]
    fn test_provision_from_yaml_wrapped() {
        let yaml = "dashboard:\n  title: Wrapped YAML\n  uid: wrap-uid\n";
        let d = provision_from_yaml(yaml).expect("should parse wrapped YAML");
        assert_eq!(d.title, "Wrapped YAML");
        assert_eq!(d.uid, "wrap-uid");
    }
}
