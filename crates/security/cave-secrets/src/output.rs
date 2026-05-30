// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Finding output formatters.
//!
//! Faithful Rust ports of TruffleHog `pkg/output/plain.go` (PlainPrinter) and
//! `pkg/output/github_actions.go` (GitHubActionsPrinter), v3.63.7. Terminal
//! color is dropped (cave-secrets emits plain strings the caller can colorize),
//! but the emitted text shape — labels, the 🐷🔑 markers, the verified/
//! unverified split, sorted metadata, and the Actions `::warning` workflow
//! command with its sha256 dedupe — matches upstream.

use crate::models::SecretFinding;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

/// Title-case a key: uppercase the first letter of each whitespace-separated
/// word, lowercase the rest. Approximates upstream's
/// `cases.Title(language.AmericanEnglish)` for the single-/two-word metadata
/// keys TruffleHog emits.
pub fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    first.to_uppercase().collect::<String>()
                        + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── PlainPrinter (pkg/output/plain.go) ───────────────────────────────────────

/// The fields PlainPrinter renders, mirroring upstream `outputFormat` plus the
/// aggregated source-metadata map.
#[derive(Debug, Clone)]
pub struct PlainResult {
    pub detector_type: String,
    pub decoder_type: String,
    pub verified: bool,
    pub raw: String,
    /// Aggregated metadata (rendered sorted by key, Title-cased label).
    pub extra_data: BTreeMap<String, String>,
}

impl PlainResult {
    /// Bridge a cave-secrets [`SecretFinding`] into a PlainResult.
    /// `raw` is `TrimSpace`'d like upstream `outputFormat.Raw`.
    pub fn from_finding(f: &SecretFinding, verified: bool) -> Self {
        let mut extra_data = BTreeMap::new();
        extra_data.insert("file".to_string(), f.file_path.clone());
        if let Some(line) = f.line_number {
            extra_data.insert("line".to_string(), line.to_string());
        }
        if let Some(commit) = &f.commit {
            extra_data.insert("commit".to_string(), commit.clone());
        }
        PlainResult {
            detector_type: f.secret_type.to_string(),
            decoder_type: "PLAIN".to_string(),
            verified,
            raw: f.context.trim().to_string(),
            extra_data,
        }
    }
}

/// Render a finding in TruffleHog plain-text format.
pub fn plain_print(r: &PlainResult) -> String {
    let mut out = String::new();
    if r.verified {
        out.push_str("Found verified result 🐷🔑\n");
    } else {
        out.push_str("Found unverified result 🐷🔑❓\n");
    }
    out.push_str(&format!("Detector Type: {}\n", r.detector_type));
    out.push_str(&format!("Decoder Type: {}\n", r.decoder_type));
    out.push_str(&format!("Raw result: {}\n", r.raw));

    // BTreeMap iterates in sorted key order, matching upstream's
    // sort.Strings(aggregateDataKeys).
    for (k, v) in &r.extra_data {
        out.push_str(&format!("{}: {}\n", title_case(k), v));
    }
    // Upstream fmt.Println("") — trailing blank line.
    out.push('\n');
    out
}

// ── GitHubActionsPrinter (pkg/output/github_actions.go) ──────────────────────

/// Emits GitHub Actions `::warning` workflow commands per finding, suppressing
/// duplicates. Upstream keys its dedupe cache on the sha256 of
/// "<decoder>:<detector>:<status>:<file>:<line>".
#[derive(Debug, Default)]
pub struct GitHubActionsPrinter {
    dedupe: HashSet<String>,
}

impl GitHubActionsPrinter {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sha256 hex dedupe key for a finding, matching upstream's
    /// `fmt.Sprintf("%s:%s:%s:%s:%d", decoder, detector, status, file, line)`
    /// hashed with sha256 and hex-encoded.
    pub fn dedupe_key(
        detector_type: &str,
        decoder_type: &str,
        verified: bool,
        filename: &str,
        start_line: i64,
    ) -> String {
        let status = if verified { "verified" } else { "unverified" };
        let raw = format!(
            "{}:{}:{}:{}:{}",
            decoder_type, detector_type, status, filename, start_line
        );
        let mut h = Sha256::new();
        h.update(raw.as_bytes());
        hex::encode(h.finalize())
    }

    /// Render the `::warning` command for a finding, or `None` if an identical
    /// finding has already been printed (upstream returns nil after caching).
    pub fn print(
        &mut self,
        detector_type: &str,
        decoder_type: &str,
        verified: bool,
        filename: &str,
        start_line: i64,
    ) -> Option<String> {
        let key = Self::dedupe_key(detector_type, decoder_type, verified, filename, start_line);
        if !self.dedupe.insert(key) {
            return None;
        }

        let status = if verified { "verified" } else { "unverified" };
        let message = if decoder_type == "PLAIN" {
            format!("Found {} {} result 🐷🔑\n", status, detector_type)
        } else {
            format!(
                "Found {} {} result with {} encoding 🐷🔑\n",
                status, detector_type, decoder_type
            )
        };

        Some(format!(
            "::warning file={},line={},endLine={}::{}",
            filename, start_line, start_line, message
        ))
    }

    /// Convenience bridge for a cave-secrets [`SecretFinding`].
    pub fn print_finding(&mut self, f: &SecretFinding, verified: bool) -> Option<String> {
        self.print(
            &f.secret_type.to_string(),
            "PLAIN",
            verified,
            &f.file_path,
            f.line_number.map(|n| n as i64).unwrap_or(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_case_empty_word_segments() {
        assert_eq!(title_case("a  b"), "A  B");
    }

    #[test]
    fn plain_print_orders_block() {
        let r = PlainResult {
            detector_type: "GitHub".to_string(),
            decoder_type: "BASE64".to_string(),
            verified: false,
            raw: "ghp_xxx".to_string(),
            extra_data: BTreeMap::new(),
        };
        let out = plain_print(&r);
        let dt = out.find("Detector Type:").unwrap();
        let dc = out.find("Decoder Type:").unwrap();
        let raw = out.find("Raw result:").unwrap();
        assert!(dt < dc && dc < raw);
    }
}
