// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Detector trait + registry. Mirrors the upstream `pkg/detectors.Detector`
//! interface: `FromData`, `Keywords`, `Type`, `Description`.

use crate::detectors;
use crate::models::{DetectionResult, DetectorType};
use crate::verification::VerifierConfig;

pub trait Detector: Send + Sync {
    fn detector_type(&self) -> DetectorType;
    fn description(&self) -> &'static str;
    fn keywords(&self) -> &'static [&'static str];
    /// Pure-byte scan (no HTTP). Verification is an orthogonal pass managed
    /// by the engine to keep this trait async-free and unit-testable.
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult>;
    /// Default verifier configuration — engine consults this when the user
    /// has not overridden via a custom range.
    fn verifier_config(&self) -> VerifierConfig {
        VerifierConfig::ok_2xx()
    }
    /// Build the HTTP request the verifier should issue against the secret.
    /// Returning `None` means this detector has no live-verify path —
    /// engine reports `Verdict::Indeterminate`.
    fn build_verify_request(&self, _raw: &str) -> Option<VerifyRequest> {
        None
    }
}

/// Pre-built verify request — used by the verifier so detectors don't have
/// to pull a reqwest client themselves.
#[derive(Debug, Clone)]
pub struct VerifyRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

pub struct DetectorRegistry {
    pub detectors: Vec<Box<dyn Detector>>,
}

impl DetectorRegistry {
    pub fn empty() -> Self {
        Self { detectors: Vec::new() }
    }

    pub fn builtin() -> Self {
        Self {
            detectors: vec![
                Box::new(detectors::aws::AwsAccessKey),
                Box::new(detectors::github::GithubToken),
                Box::new(detectors::gitlab::GitlabToken),
                Box::new(detectors::slack::SlackToken),
                Box::new(detectors::stripe::StripeKey),
                Box::new(detectors::anthropic::AnthropicKey),
                Box::new(detectors::openai::OpenaiKey),
                Box::new(detectors::twilio::TwilioKey),
                Box::new(detectors::sendgrid::SendgridKey),
                Box::new(detectors::mailgun::MailgunKey),
                Box::new(detectors::square::SquareKey),
                Box::new(detectors::npm::NpmToken),
                Box::new(detectors::pypi::PypiToken),
                Box::new(detectors::jwt::JwtToken),
                Box::new(detectors::private_key::PrivateKey),
                Box::new(detectors::generic_api_key::GenericApiKey),
                Box::new(detectors::gcp::GcpServiceAccount),
                Box::new(detectors::azure::AzureStorageKey),
            ],
        }
    }

    /// All detectors that match at least one keyword in the chunk. Keyword
    /// pre-filter mirrors `pkg/engine.matchingDetectors` — the Aho–Corasick
    /// fast path is omitted (no measurable win at this scale for a workspace
    /// crate).
    pub fn matching<'a>(&'a self, data: &[u8]) -> Vec<&'a dyn Detector> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        self.detectors
            .iter()
            .filter(|d| d.keywords().iter().any(|k| s.contains(k)))
            .map(|b| b.as_ref())
            .collect()
    }

    pub fn scan(&self, data: &[u8]) -> Vec<DetectionResult> {
        let mut out = Vec::new();
        for d in self.matching(data) {
            out.extend(d.from_data(data));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_at_least_sixteen_builtins() {
        let r = DetectorRegistry::builtin();
        assert!(r.detectors.len() >= 16);
    }

    #[test]
    fn builtin_detector_types_are_unique() {
        let r = DetectorRegistry::builtin();
        let mut t: Vec<_> = r.detectors.iter().map(|d| d.detector_type()).collect();
        t.sort_by_key(|d| *d as u32);
        let before = t.len();
        t.dedup();
        assert_eq!(before, t.len());
    }

    #[test]
    fn matching_filters_by_keyword() {
        let r = DetectorRegistry::builtin();
        // Stripe keyword
        let m = r.matching(b"my key is sk_live_X");
        assert!(m.iter().any(|d| d.detector_type() == DetectorType::Stripe));
        // Random
        let m = r.matching(b"hello world");
        assert!(m.is_empty());
    }

    #[test]
    fn empty_registry_is_empty() {
        let r = DetectorRegistry::empty();
        assert!(r.detectors.is_empty());
        assert!(r.scan(b"sk_live_x").is_empty());
    }
}
