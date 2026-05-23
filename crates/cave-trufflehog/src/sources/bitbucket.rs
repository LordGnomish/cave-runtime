// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bitbucket source — covers Cloud + Server (Data Center) REST listings of
//! workspaces / projects / repos. Repo content delegates to `GitSource`.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BitbucketOptions {
    pub workspaces: Vec<String>,
    pub projects: Vec<String>,
    pub repos: Vec<String>,
    pub host: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

pub struct BitbucketSource {
    pub options: BitbucketOptions,
    pub api_user: Option<String>,
    pub api_password: Option<String>,
}

impl BitbucketSource {
    pub fn new(options: BitbucketOptions) -> Self {
        Self {
            options,
            api_user: None,
            api_password: None,
        }
    }

    pub fn with_basic_auth(mut self, user: impl Into<String>, pw: impl Into<String>) -> Self {
        self.api_user = Some(user.into());
        self.api_password = Some(pw.into());
        self
    }

    pub fn name(&self) -> &str {
        "bitbucket"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::Bitbucket
    }
    pub fn host_or_default(&self) -> &str {
        self.options
            .host
            .as_deref()
            .unwrap_or("https://api.bitbucket.org")
    }

    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_host_is_bitbucket_org() {
        let s = BitbucketSource::new(BitbucketOptions::default());
        assert_eq!(s.host_or_default(), "https://api.bitbucket.org");
    }

    #[test]
    fn data_center_host_override() {
        let s = BitbucketSource::new(BitbucketOptions {
            host: Some("https://bitbucket.acme.internal".into()),
            ..Default::default()
        });
        assert_eq!(s.host_or_default(), "https://bitbucket.acme.internal");
    }

    #[test]
    fn basic_auth_stored() {
        let s = BitbucketSource::new(BitbucketOptions::default()).with_basic_auth("u", "p");
        assert_eq!(s.api_user.as_deref(), Some("u"));
        assert_eq!(s.api_password.as_deref(), Some("p"));
    }

    #[test]
    fn empty_offline_chunks() {
        let s = BitbucketSource::new(BitbucketOptions::default());
        assert!(s.chunks().unwrap().is_empty());
        assert_eq!(s.kind(), SourceKind::Bitbucket);
    }
}
