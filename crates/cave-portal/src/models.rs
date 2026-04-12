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
