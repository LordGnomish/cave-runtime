// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sift "Error Pattern Logs" check.
//!
//! Upstream: grafana/sift — the Error-Pattern-Logs analyzer normalizes the
//! Loki log streams for the investigation's labelset into templates,
//! groups them, and surfaces the dominant error template as a finding.
//!
//! Here we operate on plain log lines (the kernel-event argument /
//! container-log text cave-forensics already collects). Normalization is
//! a lightweight Drain-style tokenizer: variable tokens (numbers, hex
//! pointers, dotted-quad IPs) collapse to placeholders so otherwise
//! identical lines fold into one template.

use serde::{Deserialize, Serialize};

/// A group of log lines that share a normalized template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternCluster {
    /// The normalized template (variable tokens replaced by placeholders).
    pub template: String,
    /// How many input lines collapsed into this template.
    pub count: usize,
    /// Up to [`MAX_EXAMPLES`] representative raw lines.
    pub examples: Vec<String>,
}

/// Cap on retained raw examples per cluster (Sift keeps a small sample for
/// the finding UI, not the full stream).
pub const MAX_EXAMPLES: usize = 5;

/// Error-indicating substrings (case-insensitive) used to decide which
/// clusters count as "errors" for [`dominant_error_pattern`].
const ERROR_MARKERS: &[&str] = &[
    "error", "fail", "panic", "exception", "fatal", "oom", "denied", "refused",
];

/// Normalize a single log line into a Drain-style template.
///
/// Token rules (checked in order):
///   * `0x…` hex literal               → `<HEX>`
///   * dotted-quad IPv4                 → `<IP>`
///   * all-digit token                 → `<NUM>`
/// Everything else is preserved verbatim. Token boundaries are ASCII
/// whitespace, so structure is retained.
pub fn normalize_template(line: &str) -> String {
    line.split_whitespace()
        .map(normalize_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_token(tok: &str) -> String {
    // Split trailing punctuation (e.g. "500," or "0x10.") so the variable
    // core is classified, then re-attach the punctuation.
    let core: &str = tok.trim_end_matches([',', '.', ';', ':', ')', ']', '"', '\'']);
    let suffix = &tok[core.len()..];

    let placeholder = if is_hex_literal(core) {
        Some("<HEX>")
    } else if is_ipv4(core) {
        Some("<IP>")
    } else if !core.is_empty() && core.bytes().all(|b| b.is_ascii_digit()) {
        Some("<NUM>")
    } else {
        None
    };

    match placeholder {
        Some(p) => format!("{p}{suffix}"),
        None => tok.to_string(),
    }
}

fn is_hex_literal(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_ipv4(s: &str) -> bool {
    let mut parts = 0;
    for octet in s.split('.') {
        parts += 1;
        if octet.is_empty() || octet.len() > 3 || !octet.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        if octet.parse::<u16>().map(|n| n > 255).unwrap_or(true) {
            return false;
        }
    }
    parts == 4
}

/// Cluster log lines by normalized template, returning clusters sorted by
/// descending count (ties broken by template for determinism).
pub fn cluster_log_lines(lines: &[&str]) -> Vec<PatternCluster> {
    // Preserve first-seen order of templates while accumulating.
    let mut order: Vec<String> = Vec::new();
    let mut clusters: std::collections::HashMap<String, PatternCluster> =
        std::collections::HashMap::new();

    for &line in lines {
        let template = normalize_template(line);
        let entry = clusters.entry(template.clone()).or_insert_with(|| {
            order.push(template.clone());
            PatternCluster {
                template: template.clone(),
                count: 0,
                examples: Vec::new(),
            }
        });
        entry.count += 1;
        if entry.examples.len() < MAX_EXAMPLES {
            entry.examples.push(line.to_string());
        }
    }

    let mut out: Vec<PatternCluster> = order
        .into_iter()
        .map(|t| clusters.remove(&t).expect("template present"))
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.template.cmp(&b.template)));
    out
}

/// Returns the single most frequent *error* cluster, or `None` when no
/// line matches an error marker. Mirrors the Sift finding: "the dominant
/// error pattern in this stream".
pub fn dominant_error_pattern(lines: &[&str]) -> Option<PatternCluster> {
    let error_lines: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            ERROR_MARKERS.iter().any(|m| lower.contains(m))
        })
        .collect();
    if error_lines.is_empty() {
        return None;
    }
    cluster_log_lines(&error_lines).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hex_and_ip_token_classifiers() {
        assert!(is_hex_literal("0xDEAD"));
        assert!(!is_hex_literal("0x"));
        assert!(!is_hex_literal("deadbeef"));
        assert!(is_ipv4("192.168.1.1"));
        assert!(!is_ipv4("256.0.0.1"));
        assert!(!is_ipv4("1.2.3"));
    }

    #[test]
    fn test_normalize_preserves_trailing_punctuation() {
        assert_eq!(normalize_template("code 500, retry"), "code <NUM>, retry");
    }

    #[test]
    fn test_examples_capped_at_max() {
        let lines: Vec<&str> = vec!["err 1", "err 2", "err 3", "err 4", "err 5", "err 6", "err 7"];
        let c = cluster_log_lines(&lines);
        assert_eq!(c[0].count, 7);
        assert_eq!(c[0].examples.len(), MAX_EXAMPLES);
    }

    #[test]
    fn test_empty_input_yields_no_clusters() {
        assert!(cluster_log_lines(&[]).is_empty());
        assert!(dominant_error_pattern(&[]).is_none());
    }
}
