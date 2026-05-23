// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitLab personal/group/project access tokens — port of
//! `pkg/detectors/gitlab/v3/gitlab.go`. `glpat-`/`glrt-` + 20-char body.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct GitlabToken;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    RE.get_or_init(|| Regex::new(r"gl(?:pat|rt|cbt|deploy|agent)-[A-Za-z0-9_-]{20,80}").unwrap())
}

impl Detector for GitlabToken {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Gitlab
    }
    fn description(&self) -> &'static str {
        "GitLab personal access token, group / project token, CI-CD job token, deploy token, agent token"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["glpat-", "glrt-", "glcbt-", "gldeploy-", "glagent-"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| {
                DetectionResult::new(DetectorType::Gitlab, m.as_str()).with_extra(
                    "rotation_guide",
                    "https://howtorotate.com/docs/tutorials/gitlab/",
                )
            })
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://gitlab.com/api/v4/user".into(),
            headers: vec![("PRIVATE-TOKEN".into(), raw.into())],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_glpat() {
        let r = GitlabToken.from_data(b"glpat-1234567890abcdefghij");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_glrt() {
        let r = GitlabToken.from_data(b"glrt-1234567890abcdefghij");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_deploy_token() {
        let r = GitlabToken.from_data(b"gldeploy-1234567890abcdefghij");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rejects_random_text() {
        assert!(GitlabToken.from_data(b"plain text").is_empty());
    }
}
