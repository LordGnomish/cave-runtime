// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deduplication — DefectDojo's four strategies.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738
//!         dojo/settings/settings.dist.py:1186-1202 (algo enum),
//!         dojo/finding/deduplication.py:88-111 (dispatch),
//!         dojo/settings/settings.dist.py:978-1135 (HASHCODE_FIELDS_PER_SCANNER).
//!
//! Algorithms ported:
//!   - `DEDUPE_ALGO_LEGACY`  → [legacy_key]
//!   - `DEDUPE_ALGO_HASH_CODE` → [hash_code_for] driven by parser-specific
//!     field tuples in [HASHCODE_FIELDS_PER_SCANNER]
//!   - `DEDUPE_ALGO_UNIQUE_ID_FROM_TOOL` → [unique_id_key]
//!   - `DEDUPE_ALGO_UNIQUE_ID_FROM_TOOL_OR_HASH_CODE` →
//!     `unique_id_key` ⊕ `hash_code_for`

use crate::finding::Finding;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub mod legacy;
pub mod scanner_fields;
pub use legacy::{
    dedup_key as legacy_vuln_dedup_key, deduplicate, is_sla_breached, sla_days, sla_deadline,
};
pub use scanner_fields::{HASHCODE_FIELDS_PER_SCANNER, HashField, fields_for_scanner};

/// DefectDojo's four canonical dedup algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupAlgorithm {
    /// `['title', 'cwe', 'line', 'file_path', 'description']`
    /// Source: settings.dist.py:975-977
    Legacy,
    /// Hash of scanner-specific field tuple (most common).
    /// Source: HASHCODE_FIELDS_PER_SCANNER lookup.
    HashCode,
    /// `unique_id_from_tool` only — for parsers with stable tool fingerprints.
    UniqueIdFromTool,
    /// Try `unique_id_from_tool` first, fall back to hash_code.
    UniqueIdFromToolOrHashCode,
}

impl DedupAlgorithm {
    /// Parse from the DefectDojo string identifier.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "legacy" => Some(Self::Legacy),
            "hash_code" => Some(Self::HashCode),
            "unique_id_from_tool" => Some(Self::UniqueIdFromTool),
            "unique_id_from_tool_or_hash_code" => Some(Self::UniqueIdFromToolOrHashCode),
            _ => None,
        }
    }
}

/// Compute the legacy dedup key — `(title, cwe, line, file_path, description)`.
/// Empty/missing fields fold into the key as empty strings to match
/// DefectDojo's exact behaviour (`getattr(f, attr, '') or ''`).
pub fn legacy_key(f: &Finding) -> String {
    format!(
        "legacy|{}|{}|{}|{}|{}",
        f.title,
        f.cwe.map(|c| c.to_string()).unwrap_or_default(),
        f.line.map(|l| l.to_string()).unwrap_or_default(),
        f.file_path.clone().unwrap_or_default(),
        f.description
    )
}

/// Hash a finding using the field set for its `found_by_scanner`.
/// Returns lowercase hex SHA-256 of `field_name:value` pairs joined by `|`.
/// When the scanner is unknown, falls back to the legacy field set.
///
/// Source: dojo/finding/helper.py::compute_hash_code (always-prefix
/// service + per-scanner fields, joined and hashed). DefectDojo uses
/// SHA-256 hex digest. Ref: dojo/utils.py::get_hash_code.
pub fn hash_code_for(f: &Finding, scanner: Option<&str>) -> String {
    let fields = match scanner {
        Some(name) => fields_for_scanner(name),
        None => &[
            HashField::Title,
            HashField::Cwe,
            HashField::Line,
            HashField::FilePath,
            HashField::Description,
        ],
    };
    let mut h = Sha256::new();
    // HASH_CODE_FIELDS_ALWAYS = ["service"] (settings.dist.py:1179)
    h.update(b"service:");
    h.update(f.service.as_deref().unwrap_or("").as_bytes());
    h.update(b"|");
    for field in fields {
        h.update(field.name().as_bytes());
        h.update(b":");
        h.update(field.value(f).as_bytes());
        h.update(b"|");
    }
    format!("{:x}", h.finalize())
}

/// Stable unique_id_from_tool key — None if the field isn't set.
pub fn unique_id_key(f: &Finding) -> Option<String> {
    f.unique_id_from_tool.as_ref().map(|s| format!("uid|{s}"))
}

/// Apply the chosen algorithm; returns `None` when the finding lacks
/// the input needed (e.g. UniqueIdFromTool with no unique_id_from_tool).
pub fn dedup_key(f: &Finding, algo: DedupAlgorithm, scanner: Option<&str>) -> Option<String> {
    match algo {
        DedupAlgorithm::Legacy => Some(legacy_key(f)),
        DedupAlgorithm::HashCode => Some(hash_code_for(f, scanner)),
        DedupAlgorithm::UniqueIdFromTool => unique_id_key(f),
        DedupAlgorithm::UniqueIdFromToolOrHashCode => {
            unique_id_key(f).or_else(|| Some(hash_code_for(f, scanner)))
        }
    }
}

/// Collapse a batch of findings, keeping the highest-severity sample
/// per dedup key. Stable: first-seen key wins position. Increments
/// `nb_occurences` on the survivor.
///
/// Source: dojo/finding/deduplication.py::_dedupe_batch_hash_code
/// (groups by hash_code, keeps existing finding, marks new ones duplicate).
pub fn deduplicate_batch(
    findings: Vec<Finding>,
    algo: DedupAlgorithm,
    scanner: Option<&str>,
) -> Vec<Finding> {
    let mut by_key: HashMap<String, Finding> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for f in findings {
        let key = match dedup_key(&f, algo, scanner) {
            Some(k) => k,
            None => {
                // Fields missing — keep as-is, can't dedup. Use a unique
                // synthetic key on the UUID so it survives.
                format!("orphan|{}", f.id)
            }
        };
        match by_key.get_mut(&key) {
            Some(existing) => {
                if f.severity.weight() > existing.severity.weight() {
                    let occ = existing.nb_occurences + 1;
                    let mut winner = f;
                    winner.nb_occurences = occ;
                    *existing = winner;
                } else {
                    existing.nb_occurences += 1;
                }
            }
            None => {
                order.push(key.clone());
                by_key.insert(key, f);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|k| by_key.remove(&k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::FindingSeverity;

    fn mk(title: &str, cwe: u32, severity: FindingSeverity) -> Finding {
        let mut f = Finding::new(title, severity);
        f.cwe = Some(cwe);
        f.file_path = Some("src/x.rs".into());
        f.line = Some(42);
        f.description = "boom".into();
        f
    }

    #[test]
    fn algorithm_parse_roundtrip() {
        assert_eq!(
            DedupAlgorithm::parse("legacy"),
            Some(DedupAlgorithm::Legacy)
        );
        assert_eq!(
            DedupAlgorithm::parse("hash_code"),
            Some(DedupAlgorithm::HashCode)
        );
        assert_eq!(
            DedupAlgorithm::parse("unique_id_from_tool"),
            Some(DedupAlgorithm::UniqueIdFromTool)
        );
        assert_eq!(
            DedupAlgorithm::parse("unique_id_from_tool_or_hash_code"),
            Some(DedupAlgorithm::UniqueIdFromToolOrHashCode)
        );
        assert_eq!(DedupAlgorithm::parse("bogus"), None);
    }

    #[test]
    fn legacy_key_uses_five_fields() {
        let a = mk("Xss", 79, FindingSeverity::High);
        let b = mk("Xss", 79, FindingSeverity::Critical);
        assert_eq!(legacy_key(&a), legacy_key(&b)); // same fields ⇒ same key
        let c = mk("Sqli", 89, FindingSeverity::High);
        assert_ne!(legacy_key(&a), legacy_key(&c));
    }

    #[test]
    fn unique_id_none_when_unset() {
        let f = mk("X", 79, FindingSeverity::Low);
        assert!(unique_id_key(&f).is_none());
    }

    #[test]
    fn unique_id_returns_key_when_set() {
        let mut f = mk("X", 79, FindingSeverity::Low);
        f.unique_id_from_tool = Some("fp-abc".into());
        assert_eq!(unique_id_key(&f), Some("uid|fp-abc".into()));
    }

    #[test]
    fn hash_code_changes_with_field_value() {
        let a = mk("X", 79, FindingSeverity::Low);
        let mut b = mk("X", 79, FindingSeverity::Low);
        b.line = Some(99);
        assert_ne!(
            hash_code_for(&a, Some("Bandit Scan")),
            hash_code_for(&b, Some("Bandit Scan"))
        );
    }

    #[test]
    fn hash_code_per_scanner_differs() {
        // Bandit dedupes on file/line/vuln_id_from_tool; ZAP dedupes on title/cwe/severity.
        // Same finding hashed against two scanners → different keys.
        let mut f = mk("X", 79, FindingSeverity::Low);
        f.vuln_id_from_tool = Some("B101".into());
        assert_ne!(
            hash_code_for(&f, Some("Bandit Scan")),
            hash_code_for(&f, Some("ZAP Scan"))
        );
    }

    #[test]
    fn hash_code_stable_across_calls() {
        let f = mk("X", 79, FindingSeverity::Low);
        let a = hash_code_for(&f, Some("ZAP Scan"));
        let b = hash_code_for(&f, Some("ZAP Scan"));
        assert_eq!(a, b);
        // SHA256 hex = 64 chars
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn uid_or_hash_prefers_uid_when_present() {
        let mut f = mk("X", 79, FindingSeverity::Low);
        f.unique_id_from_tool = Some("xyz".into());
        let key = dedup_key(
            &f,
            DedupAlgorithm::UniqueIdFromToolOrHashCode,
            Some("Bandit Scan"),
        )
        .unwrap();
        assert!(key.starts_with("uid|"));
    }

    #[test]
    fn uid_or_hash_falls_back_to_hash_when_uid_missing() {
        let f = mk("X", 79, FindingSeverity::Low);
        let key = dedup_key(
            &f,
            DedupAlgorithm::UniqueIdFromToolOrHashCode,
            Some("Bandit Scan"),
        )
        .unwrap();
        assert!(!key.starts_with("uid|"));
        assert_eq!(key.len(), 64); // sha256
    }

    #[test]
    fn deduplicate_batch_collapses_with_hash_code() {
        // Semgrep dedupes on title/cwe/line/file_path/description (no severity).
        let a = mk("X", 79, FindingSeverity::High);
        let b = mk("X", 79, FindingSeverity::Critical);
        let c = mk("X", 79, FindingSeverity::Medium);
        let out = deduplicate_batch(
            vec![a, b, c],
            DedupAlgorithm::HashCode,
            Some("Semgrep JSON Report"),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, FindingSeverity::Critical);
        assert_eq!(out[0].nb_occurences, 3);
    }

    #[test]
    fn deduplicate_batch_keeps_distinct_findings_apart() {
        let a = mk("X", 79, FindingSeverity::High);
        let b = mk("Y", 89, FindingSeverity::High);
        let out = deduplicate_batch(
            vec![a, b],
            DedupAlgorithm::HashCode,
            Some("Semgrep JSON Report"),
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn deduplicate_batch_preserves_first_seen_order() {
        let z = mk("Zebra", 79, FindingSeverity::High);
        let a = mk("Alpha", 89, FindingSeverity::High);
        let out = deduplicate_batch(
            vec![z, a],
            DedupAlgorithm::HashCode,
            Some("Semgrep JSON Report"),
        );
        assert_eq!(out[0].title, "Zebra");
        assert_eq!(out[1].title, "Alpha");
    }

    #[test]
    fn deduplicate_with_uid_groups_by_uid() {
        let mut a = mk("X", 79, FindingSeverity::High);
        a.unique_id_from_tool = Some("F1".into());
        let mut b = mk("Y", 89, FindingSeverity::High);
        b.unique_id_from_tool = Some("F1".into());
        let out = deduplicate_batch(vec![a, b], DedupAlgorithm::UniqueIdFromTool, None);
        assert_eq!(out.len(), 1);
    }
}
