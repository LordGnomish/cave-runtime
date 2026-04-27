//! Matcher evaluation: anchored regex semantics matching Alertmanager.

use crate::models::{MatchType, Matcher};
use std::collections::HashMap;

/// Stable fingerprint for an alert based on name + sorted labels.
pub fn compute_fingerprint(name: &str, labels: &HashMap<String, String>) -> String {
    let mut parts: Vec<String> = labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
    parts.sort();
    let body = format!("{name}:{}", parts.join(","));
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(body.as_bytes());
    hex::encode(&digest[..8])
}

/// Whether a single matcher matches a label-set. Regex matchers are
/// fully-anchored to mirror Alertmanager semantics; an unparseable regex
/// falls back to a substring check (legacy behaviour).
pub fn matcher_matches(matcher: &Matcher, labels: &HashMap<String, String>) -> bool {
    let value = labels.get(&matcher.label);
    match (matcher.effective_type(), value) {
        (MatchType::Equal, Some(v)) => v == &matcher.value,
        (MatchType::Equal, None) => matcher.value.is_empty(),
        (MatchType::NotEqual, Some(v)) => v != &matcher.value,
        (MatchType::NotEqual, None) => !matcher.value.is_empty(),
        (MatchType::Regex, Some(v)) => regex_match(&matcher.value, v),
        (MatchType::Regex, None) => regex_match(&matcher.value, ""),
        (MatchType::NotRegex, Some(v)) => !regex_match(&matcher.value, v),
        (MatchType::NotRegex, None) => !regex_match(&matcher.value, ""),
    }
}

fn regex_match(pattern: &str, value: &str) -> bool {
    // Alertmanager anchors regexes: `^(pattern)$`
    let anchored = format!("^(?:{})$", pattern);
    match regex::Regex::new(&anchored) {
        Ok(re) => re.is_match(value),
        Err(_) => value.contains(pattern), // legacy fallback
    }
}

/// All matchers must hold for the label-set.
pub fn all_match(matchers: &[Matcher], labels: &HashMap<String, String>) -> bool {
    matchers.iter().all(|m| matcher_matches(m, labels))
}

/// Any matcher holds for the label-set.
pub fn any_match(matchers: &[Matcher], labels: &HashMap<String, String>) -> bool {
    matchers.iter().any(|m| matcher_matches(m, labels))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lbl<I, K, V>(it: I) -> HashMap<String, String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        it.into_iter().map(|(k, v)| (k.into(), v.into())).collect()
    }

    #[test]
    fn test_compute_fingerprint_deterministic() {
        let l = lbl([("a", "1"), ("b", "2")]);
        assert_eq!(compute_fingerprint("X", &l), compute_fingerprint("X", &l));
    }

    #[test]
    fn test_compute_fingerprint_label_order_independent() {
        let a = lbl([("a", "1"), ("b", "2")]);
        let b = lbl([("b", "2"), ("a", "1")]);
        assert_eq!(compute_fingerprint("X", &a), compute_fingerprint("X", &b));
    }

    #[test]
    fn test_compute_fingerprint_name_changes_hash() {
        let l = lbl([("a", "1")]);
        assert_ne!(compute_fingerprint("X", &l), compute_fingerprint("Y", &l));
    }

    #[test]
    fn test_equal_matcher_present() {
        let l = lbl([("env", "prod")]);
        assert!(matcher_matches(&Matcher::equal("env", "prod"), &l));
        assert!(!matcher_matches(&Matcher::equal("env", "stage"), &l));
    }

    #[test]
    fn test_equal_matcher_missing_label_only_matches_empty() {
        let l = lbl([("env", "prod")]);
        assert!(!matcher_matches(&Matcher::equal("missing", "x"), &l));
        assert!(matcher_matches(&Matcher::equal("missing", ""), &l));
    }

    #[test]
    fn test_not_equal_matcher() {
        let l = lbl([("env", "prod")]);
        assert!(matcher_matches(&Matcher::not_equal("env", "stage"), &l));
        assert!(!matcher_matches(&Matcher::not_equal("env", "prod"), &l));
        // missing label, expecting non-empty value: NotEqual("x") on missing is true
        assert!(matcher_matches(&Matcher::not_equal("missing", "x"), &l));
    }

    #[test]
    fn test_regex_matcher_anchored() {
        let l = lbl([("env", "production")]);
        // anchored: prod won't match "production"
        assert!(!matcher_matches(&Matcher::regex("env", "prod"), &l));
        // anchored: prod.* matches
        assert!(matcher_matches(&Matcher::regex("env", "prod.*"), &l));
        // alternation
        assert!(matcher_matches(&Matcher::regex("env", "prod.*|stag.*"), &l));
    }

    #[test]
    fn test_not_regex_matcher() {
        let l = lbl([("env", "production")]);
        assert!(matcher_matches(&Matcher::not_regex("env", "stag.*"), &l));
        assert!(!matcher_matches(&Matcher::not_regex("env", "prod.*"), &l));
    }

    #[test]
    fn test_invalid_regex_falls_back_to_substring() {
        let l = lbl([("env", "production")]);
        // "[" alone is invalid → substring "" check on "production" → contains is true
        let m = Matcher::regex("env", "[");
        // anchored compile fails; fallback substring check: "production".contains("[") = false
        assert!(!matcher_matches(&m, &l));
    }

    #[test]
    fn test_all_match() {
        let l = lbl([("env", "prod"), ("svc", "api")]);
        assert!(all_match(
            &[Matcher::equal("env", "prod"), Matcher::equal("svc", "api")],
            &l
        ));
        assert!(!all_match(
            &[Matcher::equal("env", "prod"), Matcher::equal("svc", "web")],
            &l
        ));
        assert!(all_match(&[], &l)); // vacuous
    }

    #[test]
    fn test_any_match() {
        let l = lbl([("env", "prod")]);
        assert!(any_match(
            &[Matcher::equal("env", "stage"), Matcher::equal("env", "prod")],
            &l
        ));
        assert!(!any_match(
            &[Matcher::equal("env", "stage"), Matcher::equal("env", "qa")],
            &l
        ));
    }

    #[test]
    fn test_legacy_is_regex_true_treated_as_regex() {
        let l = lbl([("env", "prod123")]);
        let m = Matcher {
            label: "env".into(),
            value: "prod\\d+".into(),
            is_regex: true,
            match_type: MatchType::Equal, // legacy populates is_regex only
        };
        assert!(matcher_matches(&m, &l));
    }
}
