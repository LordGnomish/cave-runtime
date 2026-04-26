//! Domain models for cave-docs-site.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocSite {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub base_url: String,
    pub team_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DocSite {
    pub fn new(name: String, slug: String, description: String, base_url: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            slug,
            description,
            base_url,
            team_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocPage {
    pub id: Uuid,
    pub site_id: Uuid,
    pub title: String,
    /// URL path within the site, e.g. "/getting-started"
    pub path: String,
    /// Raw markdown content
    pub content: String,
    pub order: u32,
    pub parent_id: Option<Uuid>,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DocPage {
    pub fn new(
        site_id: Uuid,
        title: String,
        path: String,
        content: String,
        order: u32,
        parent_id: Option<Uuid>,
        version: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            site_id,
            title,
            path,
            content,
            order,
            parent_id,
            version,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocVersion {
    pub id: Uuid,
    pub site_id: Uuid,
    /// e.g. "v1.0", "latest"
    pub label: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

impl DocVersion {
    pub fn new(site_id: Uuid, label: String, is_default: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            site_id,
            label,
            is_default,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndex {
    pub site_id: Uuid,
    pub version: String,
    pub entries: Vec<SearchEntry>,
    pub built_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEntry {
    pub page_id: Uuid,
    pub title: String,
    pub path: String,
    pub excerpt: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocTeam {
    pub id: Uuid,
    pub name: String,
    pub members: Vec<DocTeamMember>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocTeamMember {
    pub user_id: Uuid,
    pub email: String,
    pub role: TeamRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Owner,
    Editor,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocComment {
    pub id: Uuid,
    pub page_id: Uuid,
    pub author_id: Uuid,
    pub content: String,
    pub line_anchor: Option<u32>,
    pub resolved: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl DocComment {
    pub fn new(page_id: Uuid, author_id: Uuid, content: String, line_anchor: Option<u32>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            page_id,
            author_id,
            content,
            line_anchor,
            resolved: false,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocAnalytics {
    pub site_id: Uuid,
    pub page_views: u64,
    pub unique_visitors: u64,
    pub top_pages: Vec<PageViewStat>,
    pub search_queries: Vec<SearchQueryStat>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageViewStat {
    pub path: String,
    pub views: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQueryStat {
    pub query: String,
    pub count: u64,
    pub results_found: bool,
}
