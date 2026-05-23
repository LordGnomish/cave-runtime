// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Slack bot / user / workspace / refresh tokens — port of
//! `pkg/detectors/slack/slack.go`.

use crate::detector::{Detector, VerifyRequest};
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct SlackToken;

static BOT: OnceLock<Regex> = OnceLock::new();
static USER: OnceLock<Regex> = OnceLock::new();
static WS: OnceLock<Regex> = OnceLock::new();
static REF: OnceLock<Regex> = OnceLock::new();

fn bot() -> &'static Regex {
    BOT.get_or_init(|| Regex::new(r"xoxb-[0-9]{10,13}-[0-9]{10,13}[a-zA-Z0-9\-]*").unwrap())
}
fn user() -> &'static Regex {
    USER.get_or_init(|| Regex::new(r"xoxp-[0-9]{10,13}-[0-9]{10,13}[a-zA-Z0-9\-]*").unwrap())
}
fn ws() -> &'static Regex {
    WS.get_or_init(|| Regex::new(r"xoxa-[0-9]{10,13}-[0-9]{10,13}[a-zA-Z0-9\-]*").unwrap())
}
fn refresh() -> &'static Regex {
    REF.get_or_init(|| Regex::new(r"xoxr-[0-9]{10,13}-[0-9]{10,13}[a-zA-Z0-9\-]*").unwrap())
}

impl Detector for SlackToken {
    fn detector_type(&self) -> DetectorType {
        DetectorType::Slack
    }
    fn description(&self) -> &'static str {
        "Slack bot, user, workspace and refresh tokens (xoxb / xoxp / xoxa / xoxr)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["xoxb-", "xoxp-", "xoxa-", "xoxr-"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (label, r) in [
            ("bot", bot()),
            ("user", user()),
            ("workspace", ws()),
            ("refresh", refresh()),
        ] {
            for m in r.find_iter(s) {
                out.push(
                    DetectionResult::new(DetectorType::Slack, m.as_str())
                        .with_extra("token_type", label)
                        .with_extra(
                            "rotation_guide",
                            "https://howtorotate.com/docs/tutorials/slack/",
                        ),
                );
            }
        }
        out
    }
    fn build_verify_request(&self, raw: &str) -> Option<VerifyRequest> {
        Some(VerifyRequest {
            method: "POST".into(),
            url: "https://slack.com/api/auth.test".into(),
            headers: vec![
                ("Authorization".into(), format!("Bearer {}", raw)),
                (
                    "Content-Type".into(),
                    "application/json; charset=utf-8".into(),
                ),
            ],
            body: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_token_type() {
        let s = b"xoxb-1111111111-2222222222-AbCdEf\nxoxp-1111111111-2222222222-Aa\nxoxa-1111111111-2222222222-Bb\nxoxr-1111111111-2222222222-Cc";
        let r = SlackToken.from_data(s);
        let mut types: Vec<_> = r
            .iter()
            .map(|x| x.extra_data.get("token_type").unwrap().clone())
            .collect();
        types.sort();
        assert_eq!(types, vec!["bot", "refresh", "user", "workspace"]);
    }

    #[test]
    fn rejects_short_payload() {
        assert!(SlackToken.from_data(b"xoxb-123-abc-def").is_empty());
    }

    #[test]
    fn verify_request_is_post_with_auth() {
        let req = SlackToken.build_verify_request("xoxb-x").unwrap();
        assert_eq!(req.method, "POST");
        assert!(
            req.headers
                .iter()
                .any(|(k, _)| k == "Authorization")
        );
    }
}
