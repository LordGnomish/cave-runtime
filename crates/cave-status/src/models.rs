use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusComponent {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub status: ComponentStatus,
    pub group: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ComponentStatus {
    Operational,
    DegradedPerformance,
    PartialOutage,
    MajorOutage,
    UnderMaintenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusPage {
    pub name: String,
    pub components: Vec<StatusComponent>,
    pub overall_status: ComponentStatus,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusIncident {
    pub id: Uuid,
    pub title: String,
    pub affected_components: Vec<Uuid>,
    pub impact: ComponentStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_component(status: ComponentStatus) -> StatusComponent {
        StatusComponent {
            id: Uuid::new_v4(),
            name: "API Gateway".to_string(),
            description: "Main entry point".to_string(),
            status,
            group: Some("core".to_string()),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_status_component_roundtrip() {
        let c = make_component(ComponentStatus::Operational);
        let json = serde_json::to_string(&c).unwrap();
        let decoded: StatusComponent = serde_json::from_str(&json).unwrap();
        assert_eq!(c, decoded);
    }

    #[test]
    fn test_component_status_major_outage_roundtrip() {
        let s = ComponentStatus::MajorOutage;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"major_outage\"");
        let decoded: ComponentStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(s, decoded);
    }

    #[test]
    fn test_status_page_roundtrip() {
        let page = StatusPage {
            name: "CAVE Platform".to_string(),
            components: vec![make_component(ComponentStatus::Operational)],
            overall_status: ComponentStatus::Operational,
            last_updated: Utc::now(),
        };
        let json = serde_json::to_string(&page).unwrap();
        let decoded: StatusPage = serde_json::from_str(&json).unwrap();
        assert_eq!(page.name, decoded.name);
        assert_eq!(page.overall_status, decoded.overall_status);
    }

    #[test]
    fn test_status_incident_roundtrip() {
        let incident = StatusIncident {
            id: Uuid::new_v4(),
            title: "Database connectivity issues".to_string(),
            affected_components: vec![Uuid::new_v4()],
            impact: ComponentStatus::PartialOutage,
            created_at: Utc::now(),
            resolved_at: None,
        };
        let json = serde_json::to_string(&incident).unwrap();
        let decoded: StatusIncident = serde_json::from_str(&json).unwrap();
        assert_eq!(incident, decoded);
    }

    #[test]
    fn test_component_no_group_roundtrip() {
        let mut c = make_component(ComponentStatus::UnderMaintenance);
        c.group = None;
        let json = serde_json::to_string(&c).unwrap();
        let decoded: StatusComponent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.group, None);
    }
}
