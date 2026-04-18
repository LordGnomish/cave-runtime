use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Teams
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct Team {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub members: Vec<String>,
    pub timezone: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub timezone: String,
    pub phone: Option<String>,
    pub slack_id: Option<String>,
    pub active: bool,
}

// ---------------------------------------------------------------------------
// Schedules and Rotations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde", rename_all = "snake_case")]
pub enum ScheduleType {
    Rotation,
    FixedShift,
    Override,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct Schedule {
    pub id: Uuid,
    pub team_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub timezone: String,
    pub schedule_type: ScheduleType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde", rename_all = "snake_case")]
pub enum RotationType {
    Daily,
    Weekly,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct Rotation {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub name: String,
    pub users: Vec<String>,
    pub start_date: DateTime<Utc>,
    pub rotation_type: RotationType,
    pub handoff_hour: u32,
    pub handoff_minute: u32,
    pub shift_duration_hours: u32,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct ShiftOverride {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub user: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Escalation Policies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde", rename_all = "snake_case")]
pub enum EscalationStepType {
    NotifyOnCall,
    NotifyUser { username: String },
    NotifyTeam { team_id: Uuid },
    Wait { minutes: u32 },
    RepeatFromStart,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct EscalationStep {
    pub order: u32,
    pub step_type: EscalationStepType,
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct EscalationPolicy {
    pub id: Uuid,
    pub team_id: Uuid,
    pub name: String,
    pub steps: Vec<EscalationStep>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Alerts and Silences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde", rename_all = "PascalCase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde", rename_all = "PascalCase")]
pub enum AlertState {
    Firing,
    Acknowledged,
    Resolved,
    Silenced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct Alert {
    pub id: Uuid,
    pub team_id: Uuid,
    pub title: String,
    pub severity: Severity,
    pub source: String,
    pub fingerprint: String,
    pub state: AlertState,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub ack_at: Option<DateTime<Utc>>,
    pub ack_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub escalation_policy_id: Option<Uuid>,
    pub current_escalation_step: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct Silence {
    pub id: Uuid,
    pub team_id: Uuid,
    pub matcher: HashMap<String, String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub created_by: String,
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// On-Call Assignments and Links
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct OnCallAssignment {
    pub team_id: Uuid,
    pub user: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub schedule_id: Uuid,
    pub rotation_id: Option<Uuid>,
    pub is_override: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(crate = "serde")]
pub struct IncidentLink {
    pub alert_id: Uuid,
    pub cave_incidents_id: Uuid,
}

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateTeamRequest {
    pub name: String,
    pub slug: String,
    pub timezone: String,
    pub members: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub timezone: String,
    pub phone: Option<String>,
    pub slack_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateScheduleRequest {
    pub team_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub timezone: String,
    pub schedule_type: ScheduleType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateRotationRequest {
    pub schedule_id: Uuid,
    pub name: String,
    pub users: Vec<String>,
    pub start_date: DateTime<Utc>,
    pub rotation_type: RotationType,
    pub handoff_hour: u32,
    pub handoff_minute: u32,
    pub shift_duration_hours: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateOverrideRequest {
    pub schedule_id: Uuid,
    pub user: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateEscalationPolicyRequest {
    pub team_id: Uuid,
    pub name: String,
    pub steps: Vec<EscalationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateAlertRequest {
    pub team_id: Uuid,
    pub title: String,
    pub severity: Severity,
    pub source: String,
    pub fingerprint: String,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub escalation_policy_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct AckRequest {
    pub user: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct SnoozeRequest {
    pub duration_minutes: u32,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct CreateSilenceRequest {
    pub team_id: Uuid,
    pub matcher: HashMap<String, String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub created_by: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "serde")]
pub struct WebhookPayload {
    pub source: String,
    pub alert_id: Option<String>,
    pub title: String,
    pub severity: String,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    /// Stable fingerprint used by [`crate::engine::dedupe_fingerprint`] to
    /// collapse repeated pages into a single alert.
    #[serde(default)]
    pub fingerprint: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_team_construction() {
        let team = Team {
            id: Uuid::new_v4(),
            name: "Platform".to_string(),
            slug: "platform".to_string(),
            description: Some("Platform team".to_string()),
            members: vec!["alice@example.com".to_string()],
            timezone: "UTC".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert_eq!(team.name, "Platform");
        assert_eq!(team.members.len(), 1);
    }

    #[test]
    fn test_user_construction() {
        let user = User {
            id: Uuid::new_v4(),
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            display_name: "Alice Smith".to_string(),
            timezone: "America/New_York".to_string(),
            phone: Some("+1234567890".to_string()),
            slack_id: Some("U123456".to_string()),
            active: true,
        };
        assert_eq!(user.username, "alice");
        assert!(user.active);
    }

    #[test]
    fn test_rotation_construction() {
        let rotation = Rotation {
            id: Uuid::new_v4(),
            schedule_id: Uuid::new_v4(),
            name: "Weekly rotation".to_string(),
            users: vec!["alice".to_string(), "bob".to_string()],
            start_date: Utc::now(),
            rotation_type: RotationType::Weekly,
            handoff_hour: 9,
            handoff_minute: 0,
            shift_duration_hours: 24,
            active: true,
        };
        assert_eq!(rotation.users.len(), 2);
        assert_eq!(rotation.handoff_hour, 9);
    }

    #[test]
    fn test_alert_construction() {
        let mut labels = HashMap::new();
        labels.insert("env".to_string(), "prod".to_string());
        let alert = Alert {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            title: "High CPU".to_string(),
            severity: Severity::High,
            source: "prometheus".to_string(),
            fingerprint: "abc123".to_string(),
            state: AlertState::Firing,
            labels,
            annotations: HashMap::new(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: None,
            escalation_policy_id: None,
            current_escalation_step: 0,
        };
        assert_eq!(alert.title, "High CPU");
        assert_eq!(alert.state, AlertState::Firing);
    }

    #[test]
    fn test_escalation_policy_construction() {
        let policy = EscalationPolicy {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            name: "Default escalation".to_string(),
            steps: vec![EscalationStep {
                order: 0,
                step_type: EscalationStepType::NotifyOnCall,
                timeout_seconds: 300,
            }],
            created_at: Utc::now(),
        };
        assert_eq!(policy.steps.len(), 1);
        assert_eq!(policy.steps[0].order, 0);
    }

    #[test]
    fn test_silence_construction() {
        let silence = Silence {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            matcher: HashMap::new(),
            start: Utc::now(),
            end: Utc::now(),
            created_by: "alice".to_string(),
            reason: Some("maintenance".to_string()),
        };
        assert_eq!(silence.created_by, "alice");
        assert!(silence.reason.is_some());
    }
}
