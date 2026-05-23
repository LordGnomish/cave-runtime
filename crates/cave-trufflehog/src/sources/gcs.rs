// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GCS source — Google Cloud Storage bucket listing. Mirrors
//! `pkg/sources/gcs/gcs.go`.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GcsOptions {
    pub buckets: Vec<String>,
    pub include_buckets: Vec<String>,
    pub exclude_buckets: Vec<String>,
    pub include_objects: Vec<String>,
    pub exclude_objects: Vec<String>,
    pub project_id: Option<String>,
}

pub struct GcsSource {
    pub options: GcsOptions,
    pub service_account_json: Option<String>,
}

impl GcsSource {
    pub fn new(options: GcsOptions) -> Self {
        Self {
            options,
            service_account_json: None,
        }
    }

    pub fn with_service_account(mut self, json: impl Into<String>) -> Self {
        self.service_account_json = Some(json.into());
        self
    }

    pub fn name(&self) -> &str {
        "gcs"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::Gcs
    }

    pub fn bucket_allowed(&self, bucket: &str) -> bool {
        if !self.options.include_buckets.is_empty()
            && !self
                .options
                .include_buckets
                .iter()
                .any(|b| b == bucket)
        {
            return false;
        }
        !self
            .options
            .exclude_buckets
            .iter()
            .any(|b| b == bucket)
    }

    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_allowed_by_include() {
        let s = GcsSource::new(GcsOptions {
            include_buckets: vec!["mine".into()],
            ..Default::default()
        });
        assert!(s.bucket_allowed("mine"));
        assert!(!s.bucket_allowed("other"));
    }

    #[test]
    fn bucket_blocked_by_exclude() {
        let s = GcsSource::new(GcsOptions {
            exclude_buckets: vec!["audit".into()],
            ..Default::default()
        });
        assert!(s.bucket_allowed("mine"));
        assert!(!s.bucket_allowed("audit"));
    }

    #[test]
    fn service_account_stored() {
        let s = GcsSource::new(GcsOptions::default()).with_service_account("{}");
        assert_eq!(s.service_account_json.as_deref(), Some("{}"));
        assert_eq!(s.kind(), SourceKind::Gcs);
    }
}
