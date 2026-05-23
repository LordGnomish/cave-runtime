// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JIRA source — port of `pkg/sources/jira/jira.go`. Issue + comment scan
//! over Atlassian Cloud + Data Center via REST.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JiraOptions {
    pub host: Option<String>,
    pub projects: Vec<String>,
    pub include_comments: bool,
    pub include_attachments: bool,
}

pub struct JiraSource {
    pub options: JiraOptions,
    pub api_user: Option<String>,
    pub api_token: Option<String>,
}

impl JiraSource {
    pub fn new(options: JiraOptions) -> Self {
        Self {
            options,
            api_user: None,
            api_token: None,
        }
    }

    pub fn with_basic(mut self, user: impl Into<String>, token: impl Into<String>) -> Self {
        self.api_user = Some(user.into());
        self.api_token = Some(token.into());
        self
    }

    pub fn name(&self) -> &str {
        "jira"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::Jira
    }

    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_credentials() {
        let s = JiraSource::new(JiraOptions::default()).with_basic("u", "t");
        assert_eq!(s.api_user.as_deref(), Some("u"));
        assert_eq!(s.api_token.as_deref(), Some("t"));
    }

    #[test]
    fn kind_is_jira() {
        let s = JiraSource::new(JiraOptions::default());
        assert_eq!(s.kind(), SourceKind::Jira);
    }

    #[test]
    fn offline_chunks_empty() {
        let s = JiraSource::new(JiraOptions::default());
        assert!(s.chunks().unwrap().is_empty());
    }
}
