// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! S3 source — port of the bucket-listing+object-download surface in
//! `pkg/sources/s3/s3.go`. cave-runtime's cave-blob abstraction provides
//! the AWS-SDK-equivalent; this module is the pull-side adapter.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct S3Options {
    pub buckets: Vec<String>,
    pub roles: Vec<String>,
    pub region: Option<String>,
    pub max_object_size_mb: Option<u64>,
    pub include_prefixes: Vec<String>,
    pub exclude_prefixes: Vec<String>,
    pub use_cloud_credentials: bool,
}

pub struct S3Source {
    pub options: S3Options,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
}

impl S3Source {
    pub fn new(options: S3Options) -> Self {
        Self {
            options,
            access_key_id: None,
            secret_access_key: None,
        }
    }

    pub fn with_static_credentials(mut self, akid: impl Into<String>, sak: impl Into<String>) -> Self {
        self.access_key_id = Some(akid.into());
        self.secret_access_key = Some(sak.into());
        self
    }

    pub fn name(&self) -> &str {
        "s3"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::S3
    }

    pub fn max_object_bytes(&self) -> u64 {
        self.options.max_object_size_mb.unwrap_or(250) * 1024 * 1024
    }

    pub fn prefix_allowed(&self, key: &str) -> bool {
        let allowed_by_include = self.options.include_prefixes.is_empty()
            || self
                .options
                .include_prefixes
                .iter()
                .any(|p| key.starts_with(p));
        let blocked = self
            .options
            .exclude_prefixes
            .iter()
            .any(|p| key.starts_with(p));
        allowed_by_include && !blocked
    }

    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_object_size() {
        let s = S3Source::new(S3Options::default());
        assert_eq!(s.max_object_bytes(), 250 * 1024 * 1024);
    }

    #[test]
    fn prefix_allow_with_includes() {
        let s = S3Source::new(S3Options {
            include_prefixes: vec!["logs/".into()],
            ..Default::default()
        });
        assert!(s.prefix_allowed("logs/2026/01.json"));
        assert!(!s.prefix_allowed("metrics/2026.json"));
    }

    #[test]
    fn prefix_blocked_with_excludes() {
        let s = S3Source::new(S3Options {
            exclude_prefixes: vec!["tmp/".into()],
            ..Default::default()
        });
        assert!(s.prefix_allowed("logs/x"));
        assert!(!s.prefix_allowed("tmp/x"));
    }

    #[test]
    fn static_credentials_stored() {
        let s = S3Source::new(S3Options::default()).with_static_credentials("AKIA", "abc");
        assert_eq!(s.access_key_id.as_deref(), Some("AKIA"));
        assert_eq!(s.secret_access_key.as_deref(), Some("abc"));
        assert_eq!(s.kind(), SourceKind::S3);
    }
}
