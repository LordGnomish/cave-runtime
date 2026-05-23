// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitHub PAT / OAuth / App tokens — port of `pkg/detectors/github/v2/github.go`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct GithubToken;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    // ghp_ (PAT), gho_ (OAuth), ghu_ (user-to-server), ghs_ (server-to-server),
    // ghr_ (refresh), github_pat_ (fine-grained PAT)
    RE.get_or_init(|| {
        Regex::new(r"(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{82}")
            .unwrap()
    })
}

impl Detector for GithubToken {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Github
    }
    fn description(&self) -> &'static str {
        "GitHub personal access token / OAuth token / app installation token / fine-grained PAT"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| {
                DetectionResult::new(DetectorType::Github, m.as_str()).with_extra(
                    "rotation_guide",
                    "https://howtorotate.com/docs/tutorials/github/",
                )
            })
            .collect()
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "GET".into(),
            url: "https://api.github.com/user".into(),
            headers: vec![
                ("Authorization".into(), format!("token {}", raw)),
                ("User-Agent".into(), "cave-trufflehog".into()),
                ("Accept".into(), "application/vnd.github+json".into()),
            ],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ghp_pat() {
        let r =
            GithubToken.from_data(b"token=ghp_1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZab");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_gho_oauth() {
        let r = GithubToken
            .from_data(b"AUTH=gho_1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZab");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_fine_grained_pat() {
        let token = format!("github_pat_{}", "a".repeat(82));
        let r = GithubToken.from_data(token.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn verify_request_targets_user_api() {
        let req = GithubToken.build_verify_request("ghp_x").unwrap();
        assert_eq!(req.url, "https://api.github.com/user");
    }
}
