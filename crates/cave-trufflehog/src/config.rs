// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scan-time configuration. Port of `pkg/config/config.go` schema with
//! TOML loading, include/exclude path filters, branch filters, and an
//! allowlist by fingerprint or regex.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanConfig {
    pub concurrency: usize,
    pub verify: bool,
    pub include_paths: Vec<String>,
    pub exclude_paths: Vec<String>,
    pub include_branches: Vec<String>,
    pub allowlist_fingerprints: Vec<String>,
    pub allowlist_regex: Vec<String>,
    pub rate_limit_per_second: u32,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            concurrency: 8,
            verify: false,
            include_paths: Vec::new(),
            exclude_paths: vec![
                ".git".into(),
                "node_modules".into(),
                "target".into(),
            ],
            include_branches: Vec::new(),
            allowlist_fingerprints: Vec::new(),
            allowlist_regex: Vec::new(),
            rate_limit_per_second: 0,
        }
    }
}

impl ScanConfig {
    pub fn from_toml(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| Error::Config(e.to_string()))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))
    }

    pub fn path_allowed(&self, path: &str) -> bool {
        let included = self.include_paths.is_empty()
            || self.include_paths.iter().any(|p| path.contains(p));
        let excluded = self.exclude_paths.iter().any(|p| path.contains(p));
        included && !excluded
    }

    pub fn branch_allowed(&self, branch: &str) -> bool {
        self.include_branches.is_empty()
            || self.include_branches.iter().any(|b| b == branch)
    }

    pub fn is_allowlisted(&self, fingerprint: &str) -> bool {
        self.allowlist_fingerprints
            .iter()
            .any(|p| p == fingerprint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_excludes_common_artifacts() {
        let c = ScanConfig::default();
        assert!(!c.path_allowed("/repo/.git/config"));
        assert!(!c.path_allowed("/repo/node_modules/x.js"));
        assert!(!c.path_allowed("/repo/target/x"));
        assert!(c.path_allowed("/repo/src/main.rs"));
    }

    #[test]
    fn includes_narrow_scope() {
        let c = ScanConfig {
            include_paths: vec!["src/".into()],
            ..Default::default()
        };
        assert!(c.path_allowed("/repo/src/main.rs"));
        assert!(!c.path_allowed("/repo/docs/x.md"));
    }

    #[test]
    fn branch_filter_works() {
        let c = ScanConfig {
            include_branches: vec!["main".into()],
            ..Default::default()
        };
        assert!(c.branch_allowed("main"));
        assert!(!c.branch_allowed("feature"));
    }

    #[test]
    fn fingerprint_allowlist_matches() {
        let c = ScanConfig {
            allowlist_fingerprints: vec!["deadbeef".into()],
            ..Default::default()
        };
        assert!(c.is_allowlisted("deadbeef"));
        assert!(!c.is_allowlisted("cafebabe"));
    }

    #[test]
    fn toml_round_trip() {
        let c = ScanConfig {
            verify: true,
            concurrency: 16,
            ..Default::default()
        };
        let t = c.to_toml().unwrap();
        let back = ScanConfig::from_toml(&t).unwrap();
        assert!(back.verify);
        assert_eq!(back.concurrency, 16);
    }
}
