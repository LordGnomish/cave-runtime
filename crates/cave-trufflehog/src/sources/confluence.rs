// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Confluence source — REST page + space enumeration. Atlassian Cloud +
//! Data Center.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfluenceOptions {
    pub host: Option<String>,
    pub spaces: Vec<String>,
    pub include_attachments: bool,
    pub include_comments: bool,
}

pub struct ConfluenceSource {
    pub options: ConfluenceOptions,
    pub api_user: Option<String>,
    pub api_token: Option<String>,
}

impl ConfluenceSource {
    pub fn new(options: ConfluenceOptions) -> Self {
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
        "confluence"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::Confluence
    }

    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_confluence() {
        let s = ConfluenceSource::new(ConfluenceOptions::default());
        assert_eq!(s.kind(), SourceKind::Confluence);
    }

    #[test]
    fn store_credentials() {
        let s =
            ConfluenceSource::new(ConfluenceOptions::default()).with_basic("u", "t");
        assert!(s.api_user.is_some());
        assert!(s.api_token.is_some());
    }

    #[test]
    fn options_round_trip() {
        let opts = ConfluenceOptions {
            spaces: vec!["DOCS".into()],
            include_attachments: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&opts).unwrap();
        let back: ConfluenceOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(back.spaces, vec!["DOCS".to_string()]);
        assert!(back.include_attachments);
    }
}
