// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A "Space" is the top-level container (like a GitBook space or Notion workspace section)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub custom_domain: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub visibility: Visibility,
    pub default_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Visibility {
    Public,
    Private,
    Unlisted,
}

/// A Page is a markdown document within a space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    pub space_id: String,
    pub slug: String,
    pub title: String,
    pub markdown_content: String,
    pub html_content: Option<String>,
    pub group_id: Option<String>,
    pub parent_id: Option<String>,
    pub order: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: String,
    pub metadata: HashMap<String, String>,
}

/// A PageGroup groups pages under a heading in the sidebar
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageGroup {
    pub id: String,
    pub space_id: String,
    pub title: String,
    pub order: u32,
    pub version: String,
}

/// A Version is a snapshot of a space's pages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocVersion {
    pub id: String,
    pub space_id: String,
    pub name: String,
    pub branch: Option<String>,
    pub is_default: bool,
    pub published: bool,
    pub created_at: DateTime<Utc>,
}

/// TocEntry in a table of contents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TocEntry {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub level: usize,
    pub children: Vec<TocEntry>,
    pub group_id: Option<String>,
    pub page_id: Option<String>,
}

/// Search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub page_id: String,
    pub space_id: String,
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    pub score: f32,
    pub version: String,
}

/// Custom domain mapping
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomDomain {
    pub domain: String,
    pub space_id: String,
    pub verified: bool,
    pub created_at: DateTime<Utc>,
}
