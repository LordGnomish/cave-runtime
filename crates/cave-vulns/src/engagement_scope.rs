// SPDX-License-Identifier: AGPL-3.0-or-later
//! Engagement-scoped queries. DefectDojo's UI/API consistently filters
//! findings by Product → Engagement → Test scope; this module bundles those
//! filters into a single `EngagementScope` predicate and a streaming filter
//! over a finding iterator.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/finding/views.py
//!         (`engagement_filter`, `product_filter`) + dojo/utils.py
//!         (`get_punchcard_data` scope tuple).

use crate::finding::{Finding, FindingSeverity};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngagementScope {
    pub product_id: Option<Uuid>,
    pub engagement_id: Option<Uuid>,
    pub test_id: Option<Uuid>,
    /// Minimum severity (inclusive) to include.
    pub min_severity: Option<FindingSeverity>,
    /// If true, only include `active=true` findings.
    pub only_active: bool,
    /// If true, exclude false-positives even if active.
    pub exclude_false_positive: bool,
    /// If true, exclude duplicates.
    pub exclude_duplicates: bool,
    /// Substring match against `title` (case-insensitive).
    pub title_contains: Option<String>,
}

impl EngagementScope {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn in_product(mut self, p: Uuid) -> Self {
        self.product_id = Some(p);
        self
    }
    pub fn in_engagement(mut self, e: Uuid) -> Self {
        self.engagement_id = Some(e);
        self
    }
    pub fn in_test(mut self, t: Uuid) -> Self {
        self.test_id = Some(t);
        self
    }
    pub fn min_severity(mut self, s: FindingSeverity) -> Self {
        self.min_severity = Some(s);
        self
    }
    pub fn only_active(mut self) -> Self {
        self.only_active = true;
        self
    }
    pub fn exclude_false_positive(mut self) -> Self {
        self.exclude_false_positive = true;
        self
    }
    pub fn exclude_duplicates(mut self) -> Self {
        self.exclude_duplicates = true;
        self
    }
    pub fn title_contains(mut self, s: impl Into<String>) -> Self {
        self.title_contains = Some(s.into());
        self
    }

    /// Predicate against a finding + its scope tuple
    /// `(product_id, engagement_id)`. We accept the parent IDs as inputs
    /// because Finding holds only `test_id` directly; the caller looks the
    /// rest up via `hierarchy::Test`.
    pub fn includes(
        &self,
        f: &Finding,
        product_id: Option<Uuid>,
        engagement_id: Option<Uuid>,
    ) -> bool {
        if let Some(p) = self.product_id {
            if product_id != Some(p) {
                return false;
            }
        }
        if let Some(e) = self.engagement_id {
            if engagement_id != Some(e) {
                return false;
            }
        }
        if let Some(t) = self.test_id {
            if f.test_id != Some(t) {
                return false;
            }
        }
        if let Some(min) = &self.min_severity {
            if f.severity.weight() < min.weight() {
                return false;
            }
        }
        if self.only_active && !f.state.active {
            return false;
        }
        if self.exclude_false_positive && f.state.false_p {
            return false;
        }
        if self.exclude_duplicates && f.state.duplicate {
            return false;
        }
        if let Some(s) = &self.title_contains {
            let lower = f.title.to_ascii_lowercase();
            if !lower.contains(&s.to_ascii_lowercase()) {
                return false;
            }
        }
        true
    }
}

/// One finding paired with its (product, engagement) coordinates.
#[derive(Debug, Clone)]
pub struct ScopedFinding<'a> {
    pub finding: &'a Finding,
    pub product_id: Option<Uuid>,
    pub engagement_id: Option<Uuid>,
}

/// Apply scope to an iterator. Returns references to the matching findings.
pub fn apply_scope<'a, I>(scope: &EngagementScope, items: I) -> Vec<&'a Finding>
where
    I: IntoIterator<Item = ScopedFinding<'a>>,
{
    items
        .into_iter()
        .filter(|s| scope.includes(s.finding, s.product_id, s.engagement_id))
        .map(|s| s.finding)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Finding, FindingSeverity, StateTransition};

    fn mkf(sev: FindingSeverity, title: &str) -> Finding {
        Finding::new(title, sev)
    }

    #[test]
    fn empty_scope_matches_everything() {
        let f = mkf(FindingSeverity::Low, "x");
        let s = EngagementScope::new();
        assert!(s.includes(&f, Some(Uuid::nil()), Some(Uuid::nil())));
    }

    #[test]
    fn product_scope_filters() {
        let f = mkf(FindingSeverity::Low, "x");
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        let s = EngagementScope::new().in_product(p1);
        assert!(s.includes(&f, Some(p1), None));
        assert!(!s.includes(&f, Some(p2), None));
        assert!(!s.includes(&f, None, None));
    }

    #[test]
    fn engagement_scope_filters() {
        let f = mkf(FindingSeverity::High, "x");
        let e1 = Uuid::new_v4();
        let s = EngagementScope::new().in_engagement(e1);
        assert!(s.includes(&f, None, Some(e1)));
        assert!(!s.includes(&f, None, Some(Uuid::new_v4())));
    }

    #[test]
    fn test_scope_uses_finding_field() {
        let mut f = mkf(FindingSeverity::High, "x");
        let t1 = Uuid::new_v4();
        f.test_id = Some(t1);
        let s = EngagementScope::new().in_test(t1);
        assert!(s.includes(&f, None, None));
        let other = EngagementScope::new().in_test(Uuid::new_v4());
        assert!(!other.includes(&f, None, None));
    }

    #[test]
    fn min_severity_gate() {
        let low = mkf(FindingSeverity::Low, "l");
        let high = mkf(FindingSeverity::High, "h");
        let s = EngagementScope::new().min_severity(FindingSeverity::High);
        assert!(!s.includes(&low, None, None));
        assert!(s.includes(&high, None, None));
    }

    #[test]
    fn only_active_drops_inactive() {
        let mut f = mkf(FindingSeverity::High, "x");
        f.transition(StateTransition::RiskAccept, "a").unwrap();
        let s = EngagementScope::new().only_active();
        assert!(!s.includes(&f, None, None));
    }

    #[test]
    fn exclude_false_positive() {
        let mut f = mkf(FindingSeverity::High, "x");
        f.transition(StateTransition::MarkFalsePositive, "a").unwrap();
        let s = EngagementScope::new().exclude_false_positive();
        assert!(!s.includes(&f, None, None));
    }

    #[test]
    fn title_contains_case_insensitive() {
        let f = mkf(FindingSeverity::Low, "XSS in Login Form");
        let s = EngagementScope::new().title_contains("xss");
        assert!(s.includes(&f, None, None));
    }

    #[test]
    fn apply_scope_filters_iterator() {
        let f1 = mkf(FindingSeverity::Low, "a");
        let f2 = mkf(FindingSeverity::High, "b");
        let f3 = mkf(FindingSeverity::Critical, "c");
        let items = vec![
            ScopedFinding {
                finding: &f1,
                product_id: None,
                engagement_id: None,
            },
            ScopedFinding {
                finding: &f2,
                product_id: None,
                engagement_id: None,
            },
            ScopedFinding {
                finding: &f3,
                product_id: None,
                engagement_id: None,
            },
        ];
        let s = EngagementScope::new().min_severity(FindingSeverity::High);
        let got = apply_scope(&s, items);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn scope_serde_roundtrip() {
        let s = EngagementScope::new()
            .min_severity(FindingSeverity::Critical)
            .only_active()
            .exclude_duplicates();
        let j = serde_json::to_string(&s).unwrap();
        let back: EngagementScope = serde_json::from_str(&j).unwrap();
        assert_eq!(back.min_severity, Some(FindingSeverity::Critical));
        assert!(back.only_active);
        assert!(back.exclude_duplicates);
    }
}
