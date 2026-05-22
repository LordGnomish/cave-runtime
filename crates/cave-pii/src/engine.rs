// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::PiiFinding;
use std::collections::HashMap;

/// Redact a matched string (replace middle chars with asterisks)
pub fn redact(matched: &str) -> String {
    if matched.len() <= 4 {
        return "*".repeat(matched.len());
    }
    let keep = 2;
    format!(
        "{}{}{}",
        &matched[..keep],
        "*".repeat(matched.len() - keep * 2),
        &matched[matched.len() - keep..]
    )
}

/// Check if content contains an email-like pattern (simple check)
pub fn find_emails(content: &str) -> Vec<(usize, String)> {
    content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            if line.contains('@') && line.contains('.') {
                let token = line
                    .split_whitespace()
                    .find(|w| w.contains('@') && w.contains('.'))?;
                Some((i + 1, token.to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// Simple credit card pattern (16 digits, optionally space/dash separated)
pub fn looks_like_credit_card(s: &str) -> bool {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.len() == 16
}

/// Count findings by type
pub fn count_by_type(findings: &[PiiFinding]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for f in findings {
        let key = format!("{:?}", f.pii_type);
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PiiFinding, PiiType};
    use uuid::Uuid;

    fn make_finding(pii_type: PiiType) -> PiiFinding {
        PiiFinding {
            detector_id: Uuid::new_v4(),
            pii_type,
            line_number: 1,
            redacted: "**".to_string(),
            confidence: 0.9,
        }
    }

    #[test]
    fn test_redact_short() {
        assert_eq!(redact("ab"), "**");
        assert_eq!(redact("abcd"), "****");
    }

    #[test]
    fn test_redact_long() {
        let result = redact("hello@ex.com");
        // 12 chars: keep first 2 "he", last 2 "om", middle 8 as "*"
        assert!(result.starts_with("he"));
        assert!(result.ends_with("om"));
        assert!(result.contains('*'));
        assert_eq!(result.len(), 12);
    }

    #[test]
    fn test_find_emails_found() {
        let content = "Hello user@example.com please check\nno email here";
        let found = find_emails(content);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 1);
        assert_eq!(found[0].1, "user@example.com");
    }

    #[test]
    fn test_find_emails_not_found() {
        let content = "just plain text\nno at sign here\nnothing suspicious";
        let found = find_emails(content);
        assert!(found.is_empty());
    }

    #[test]
    fn test_looks_like_credit_card_valid() {
        assert!(looks_like_credit_card("4111111111111111"));
    }

    #[test]
    fn test_looks_like_credit_card_invalid() {
        assert!(!looks_like_credit_card("123"));
    }

    #[test]
    fn test_count_by_type() {
        let findings = vec![
            make_finding(PiiType::Email),
            make_finding(PiiType::Email),
            make_finding(PiiType::CreditCard),
        ];
        let counts = count_by_type(&findings);
        assert_eq!(counts.get("Email"), Some(&2));
        assert_eq!(counts.get("CreditCard"), Some(&1));
    }
}
