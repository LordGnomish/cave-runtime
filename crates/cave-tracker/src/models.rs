//! Data models for cave-tracker.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    Epic,
    Story,
    Task,
    Bug,
    SubTask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    P1,
    P2,
    P3,
    P4,
    P5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    ToDo,
    InProgress,
    InReview,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: Uuid,
    pub project_key: String,
    pub issue_number: i32,
    pub issue_type: IssueType,
    pub summary: String,
    pub description: Option<String>,
    pub assignee: Option<Uuid>,
    pub reporter: Uuid,
    pub priority: Priority,
    pub status: IssueStatus,
    pub labels: Vec<String>,
    pub components: Vec<String>,
    pub sprint_id: Option<Uuid>,
    pub story_points: Option<f32>,
    pub due_date: Option<DateTime<Utc>>,
    /// For sub-tasks: parent issue id.
    pub parent_id: Option<Uuid>,
    pub epic_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub original_estimate_minutes: Option<i32>,
    pub time_spent_minutes: Option<i32>,
    pub remaining_estimate_minutes: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SprintState {
    Planning,
    Active,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sprint {
    pub id: Uuid,
    pub project_key: String,
    pub name: String,
    pub goal: Option<String>,
    pub state: SprintState,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub author: Uuid,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueWatcher {
    pub issue_id: Uuid,
    pub user_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusTransition {
    pub from_status: IssueStatus,
    pub to_status: IssueStatus,
    pub name: String,
    pub conditions: Vec<TransitionCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransitionCondition {
    RequiredFields(Vec<String>),
    MinStoryPoints(f32),
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateIssueRequest {
    pub issue_type: IssueType,
    pub summary: String,
    pub description: Option<String>,
    pub assignee: Option<Uuid>,
    pub priority: Option<Priority>,
    pub labels: Option<Vec<String>>,
    pub components: Option<Vec<String>>,
    pub sprint_id: Option<Uuid>,
    pub story_points: Option<f32>,
    pub due_date: Option<DateTime<Utc>>,
    pub parent_id: Option<Uuid>,
    pub epic_id: Option<Uuid>,
    pub original_estimate_minutes: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateIssueRequest {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub assignee: Option<Uuid>,
    pub priority: Option<Priority>,
    pub labels: Option<Vec<String>>,
    pub components: Option<Vec<String>>,
    pub sprint_id: Option<Uuid>,
    pub story_points: Option<f32>,
    pub due_date: Option<DateTime<Utc>>,
    pub epic_id: Option<Uuid>,
    pub time_spent_minutes: Option<i32>,
    pub remaining_estimate_minutes: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransitionRequest {
    pub to_status: IssueStatus,
    pub fields: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateSprintRequest {
    pub name: String,
    pub goal: Option<String>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BulkOperation {
    Assign(Uuid),
    Transition(IssueStatus),
    AddLabel(String),
    RemoveLabel(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct BulkOperationRequest {
    pub issue_ids: Vec<Uuid>,
    pub operation: BulkOperation,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JqlQuery {
    pub raw_query: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JqlResult {
    pub issues: Vec<Issue>,
    pub total: usize,
}
