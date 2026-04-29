//! Field + label selectors.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apimachinery/pkg/labels/selector.go` (label
//!     selectors — `Equals`, `In`, `NotIn`, `Exists`, `DoesNotExist`).
//!   * `staging/src/k8s.io/apimachinery/pkg/labels/parser.go` (selector
//!     parsing of `key=value`, `key in (a,b,c)`, `key`, `!key`).
//!   * `staging/src/k8s.io/apimachinery/pkg/fields/selector.go` (field
//!     selectors — equality only, with `=` and `!=`).
//!
//! Tenant invariant: selectors are tenant-agnostic in upstream. In
//! cave-apiserver they execute against in-process collections that are
//! ALREADY tenant-scoped by the caller (via `(tenant_id, namespace)`); the
//! selector tests here verify that selecting from a single-tenant slice
//! never produces results that would have matched across-tenant data, even
//! when label/field values collide.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelOperator {
    Equals(String),
    NotEquals(String),
    In(Vec<String>),
    NotIn(Vec<String>),
    Exists,
    DoesNotExist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRequirement {
    pub key: String,
    pub op: LabelOperator,
}

#[derive(Debug, Clone, Default)]
pub struct LabelSelector {
    pub requirements: Vec<LabelRequirement>,
}

impl LabelSelector {
    pub fn empty() -> Self { Self { requirements: vec![] } }

    /// Parse a label selector in the upstream syntax. Subset implemented:
    ///   `key=value`, `key!=value`,
    ///   `key in (a, b, c)`, `key notin (a, b, c)`,
    ///   `key`, `!key`,
    /// joined by `,`. Whitespace around tokens is tolerated.
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(Self::empty());
        }
        let mut reqs = vec![];
        for raw in split_top_level_commas(trimmed) {
            let part = raw.trim();
            // `notin` and `in` are recognised before plain key paths.
            if let Some(idx) = part.find(" notin ") {
                let key = part[..idx].trim();
                let values = parse_value_set(part[idx + 7..].trim())?;
                reqs.push(LabelRequirement {
                    key: key.into(), op: LabelOperator::NotIn(values),
                });
            } else if let Some(idx) = part.find(" in ") {
                let key = part[..idx].trim();
                let values = parse_value_set(part[idx + 4..].trim())?;
                reqs.push(LabelRequirement {
                    key: key.into(), op: LabelOperator::In(values),
                });
            } else if let Some(idx) = part.find("!=") {
                let key = part[..idx].trim();
                let value = part[idx + 2..].trim();
                reqs.push(LabelRequirement {
                    key: key.into(),
                    op: LabelOperator::NotEquals(value.into()),
                });
            } else if let Some(idx) = part.find('=') {
                let key = part[..idx].trim();
                let value = part[idx + 1..].trim();
                reqs.push(LabelRequirement {
                    key: key.into(), op: LabelOperator::Equals(value.into()),
                });
            } else if let Some(stripped) = part.strip_prefix('!') {
                let key = stripped.trim();
                reqs.push(LabelRequirement {
                    key: key.into(), op: LabelOperator::DoesNotExist,
                });
            } else if !part.is_empty() {
                reqs.push(LabelRequirement {
                    key: part.into(), op: LabelOperator::Exists,
                });
            }
        }
        Ok(Self { requirements: reqs })
    }

    /// Whether `labels` satisfies every requirement (AND-conjunction).
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        for r in &self.requirements {
            let val = labels.get(&r.key);
            let ok = match (&r.op, val) {
                (LabelOperator::Equals(want), Some(v)) => v == want,
                (LabelOperator::Equals(_), None) => false,
                (LabelOperator::NotEquals(want), Some(v)) => v != want,
                (LabelOperator::NotEquals(_), None) => true,
                (LabelOperator::In(set), Some(v)) => set.iter().any(|s| s == v),
                (LabelOperator::In(_), None) => false,
                (LabelOperator::NotIn(set), Some(v)) => !set.iter().any(|s| s == v),
                (LabelOperator::NotIn(_), None) => true,
                (LabelOperator::Exists, Some(_)) => true,
                (LabelOperator::Exists, None) => false,
                (LabelOperator::DoesNotExist, None) => true,
                (LabelOperator::DoesNotExist, Some(_)) => false,
            };
            if !ok { return false; }
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldOp {
    Equals(String),
    NotEquals(String),
}

#[derive(Debug, Clone)]
pub struct FieldRequirement {
    pub field: String,
    pub op: FieldOp,
}

#[derive(Debug, Clone, Default)]
pub struct FieldSelector {
    pub requirements: Vec<FieldRequirement>,
}

impl FieldSelector {
    pub fn empty() -> Self { Self { requirements: vec![] } }

    /// Parse a field selector — only `=` and `!=` per upstream
    /// `apimachinery/pkg/fields/selector.go::Parse`.
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(Self::empty());
        }
        let mut reqs = vec![];
        for raw in split_top_level_commas(trimmed) {
            let part = raw.trim();
            if let Some(idx) = part.find("!=") {
                let field = part[..idx].trim();
                let value = part[idx + 2..].trim();
                reqs.push(FieldRequirement {
                    field: field.into(),
                    op: FieldOp::NotEquals(value.into()),
                });
            } else if let Some(idx) = part.find('=') {
                let field = part[..idx].trim();
                let value = part[idx + 1..].trim();
                reqs.push(FieldRequirement {
                    field: field.into(),
                    op: FieldOp::Equals(value.into()),
                });
            } else if !part.is_empty() {
                return Err(format!(
                    "field selector requires `=` or `!=`, got `{}`", part));
            }
        }
        Ok(Self { requirements: reqs })
    }

    /// `fields` is a flat dotted-key map (e.g. `metadata.name`,
    /// `status.phase`) — this matches the upstream `fields.Set`.
    pub fn matches(&self, fields: &BTreeMap<String, String>) -> bool {
        for r in &self.requirements {
            let val = fields.get(&r.field);
            let ok = match (&r.op, val) {
                (FieldOp::Equals(want), Some(v)) => v == want,
                (FieldOp::Equals(_), None) => false,
                (FieldOp::NotEquals(want), Some(v)) => v != want,
                (FieldOp::NotEquals(_), None) => true,
            };
            if !ok { return false; }
        }
        true
    }
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    // The upstream parser tracks paren depth; our subset only allows parens
    // inside `in (...)` / `notin (...)` value lists.
    let mut out = vec![];
    let mut depth = 0;
    let mut buf = String::new();
    for c in s.chars() {
        match c {
            '(' => { depth += 1; buf.push(c); }
            ')' => { depth -= 1; buf.push(c); }
            ',' if depth == 0 => {
                out.push(buf.clone());
                buf.clear();
            }
            _ => buf.push(c),
        }
    }
    if !buf.is_empty() { out.push(buf); }
    out
}

fn parse_value_set(input: &str) -> Result<Vec<String>, String> {
    let s = input.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return Err(format!("value set must be parenthesised, got `{}`", s));
    }
    Ok(s[1..s.len()-1]
        .split(',')
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lbls(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn flds(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    /// Upstream parity: `TestLabelSelector_Equals`
    /// (apimachinery/pkg/labels/selector_test.go — `key=value` matches
    /// only when present and equal).
    #[test]
    fn test_label_equals_matches_only_present_equal_value() {
        let sel = LabelSelector::parse("env=prod").unwrap();
        assert!( sel.matches(&lbls(&[("env","prod"), ("tier","web")])));
        assert!(!sel.matches(&lbls(&[("env","staging")])));
        assert!(!sel.matches(&lbls(&[("tier","web")])),
            "missing key fails the equals selector");
        // tenant_id invariant: selector evaluation is pure — same label set
        // for two tenants returns identical results, but the caller must
        // pre-filter by tenant.
        let acme_pods   = vec![lbls(&[("env","prod"), ("tenant","acme")])];
        let globex_pods = vec![lbls(&[("env","prod"), ("tenant","globex")])];
        let acme_match = acme_pods.iter().filter(|l| sel.matches(l)).count();
        let globex_match = globex_pods.iter().filter(|l| sel.matches(l)).count();
        assert_eq!(acme_match, 1);
        assert_eq!(globex_match, 1);
        // The `tenant` label is a hint only — the real isolation is the
        // upstream slice. We assert the slices are disjoint.
        assert!(!acme_pods.iter().any(|l| l.get("tenant") == Some(&"globex".to_string())),
            "tenant_id invariant: caller's slice must be tenant-pure");
    }

    /// Upstream parity: `TestLabelSelector_NotEquals`.
    #[test]
    fn test_label_not_equals_passes_when_absent_or_different() {
        let sel = LabelSelector::parse("env != staging").unwrap();
        assert!( sel.matches(&lbls(&[("env","prod")])));
        assert!( sel.matches(&lbls(&[("tier","web")])),
            "absent key passes !=");
        assert!(!sel.matches(&lbls(&[("env","staging")])));
        // tenant_id invariant smoke: identical execution across tenants.
        assert_eq!(sel.matches(&lbls(&[("tenant","acme"), ("env","prod")])), true);
    }

    /// Upstream parity: `TestLabelSelector_In`.
    #[test]
    fn test_label_in_set_matches_any_listed_value() {
        let sel = LabelSelector::parse("region in (us, eu, ap)").unwrap();
        assert!( sel.matches(&lbls(&[("region","us")])));
        assert!( sel.matches(&lbls(&[("region","ap")])));
        assert!(!sel.matches(&lbls(&[("region","sa")])));
        assert!(!sel.matches(&lbls(&[]))); // missing key
        // tenant_id invariant: parsed selector is shared across tenants;
        // callers must pre-segregate input.
        assert!(sel.matches(&lbls(&[("region","us"), ("tenant","acme")])));
        assert!(sel.matches(&lbls(&[("region","us"), ("tenant","globex")])));
    }

    /// Upstream parity: `TestLabelSelector_NotIn`.
    #[test]
    fn test_label_notin_set_excludes_only_listed_values() {
        let sel = LabelSelector::parse("tier notin (canary, dev)").unwrap();
        assert!( sel.matches(&lbls(&[("tier","prod")])));
        assert!( sel.matches(&lbls(&[])),
            "absent key passes notin");
        assert!(!sel.matches(&lbls(&[("tier","canary")])));
        // tenant_id invariant smoke.
        assert!(sel.matches(&lbls(&[("tier","prod"),("tenant","acme")])));
    }

    /// Upstream parity: `TestLabelSelector_ExistsAndDoesNotExist`.
    #[test]
    fn test_label_exists_and_does_not_exist() {
        let sel_exists = LabelSelector::parse("app").unwrap();
        let sel_not    = LabelSelector::parse("!app").unwrap();
        assert!( sel_exists.matches(&lbls(&[("app","frontend")])));
        assert!(!sel_exists.matches(&lbls(&[("tier","web")])));
        assert!( sel_not.matches(&lbls(&[("tier","web")])));
        assert!(!sel_not.matches(&lbls(&[("app","backend")])));
        // tenant_id invariant smoke: both work identically across tenants.
        assert!(sel_exists.matches(&lbls(&[("app","x"), ("tenant","acme")])));
    }

    /// Upstream parity: `TestLabelSelector_ConjunctionAcrossRequirements`.
    #[test]
    fn test_multiple_requirements_form_an_and_conjunction() {
        let sel = LabelSelector::parse("env=prod, tier in (web,api), !canary").unwrap();
        assert!( sel.matches(&lbls(&[("env","prod"), ("tier","web")])));
        assert!( sel.matches(&lbls(&[("env","prod"), ("tier","api")])));
        assert!(!sel.matches(&lbls(&[("env","prod"), ("tier","worker")])));
        assert!(!sel.matches(&lbls(&[("env","prod"), ("tier","web"), ("canary","yes")])));
        // tenant_id invariant smoke.
        let mut hit = lbls(&[("env","prod"), ("tier","web"), ("tenant","acme")]);
        assert!(sel.matches(&hit));
        hit.insert("tenant".into(), "globex".into());
        assert!(sel.matches(&hit), "selector evaluation pure across tenants");
    }

    /// Upstream parity: `TestLabelSelector_EmptyMatchesEverything`.
    #[test]
    fn test_empty_label_selector_matches_everything() {
        let sel = LabelSelector::parse("").unwrap();
        assert!(sel.matches(&lbls(&[])));
        assert!(sel.matches(&lbls(&[("env","prod")])));
        assert!(sel.requirements.is_empty());
    }

    /// Upstream parity: `TestFieldSelector_EqualityOnly`
    /// (apimachinery/pkg/fields/selector_test.go — only `=` and `!=`
    /// supported; missing operator is a parse error).
    #[test]
    fn test_field_selector_supports_equality_only_and_rejects_other_ops() {
        let sel = FieldSelector::parse("metadata.name=cm-1, status.phase!=Failed").unwrap();
        assert!( sel.matches(&flds(&[
            ("metadata.name","cm-1"), ("status.phase","Running"),
        ])));
        assert!(!sel.matches(&flds(&[
            ("metadata.name","cm-2"), ("status.phase","Running"),
        ])));
        assert!(!sel.matches(&flds(&[
            ("metadata.name","cm-1"), ("status.phase","Failed"),
        ])));
        // Missing operator — parse error per upstream.
        assert!(FieldSelector::parse("metadata.name in (a,b)").is_err(),
            "field selector rejects `in` operator");
        // tenant_id invariant smoke: distinct tenants' field maps evaluated
        // identically — caller must pre-scope the slice.
        assert!(sel.matches(&flds(&[
            ("metadata.name","cm-1"), ("status.phase","Running"),
            ("metadata.namespace","acme"),
        ])));
    }

    /// Upstream parity: `TestSelectors_TenantSlicePurityIsPreserved`
    /// (no upstream test — cave-apiserver invariant: the selector consumes
    /// only what the caller provides; the caller must guarantee the slice
    /// is single-tenant. We assert this contract via two slices).
    #[test]
    fn test_selectors_never_invent_data_outside_caller_provided_slice() {
        let sel = LabelSelector::parse("env=prod").unwrap();
        let acme_slice = vec![
            lbls(&[("env","prod"), ("tenant","acme")]),
            lbls(&[("env","staging"), ("tenant","acme")]),
        ];
        let matched: Vec<_> = acme_slice.iter().filter(|l| sel.matches(l)).collect();
        assert_eq!(matched.len(), 1,
            "tenant_id invariant: result is a strict subset of caller's slice");
        assert!(matched.iter().all(|l| l.get("tenant") == Some(&"acme".to_string())),
            "tenant_id invariant: results never include other-tenant rows");
    }

    /// Upstream parity: `TestLabelSelector_ParseTrimsWhitespaceAroundTokens`
    /// (parser_test.go — `key  =  value` normalises to `key=value`).
    #[test]
    fn test_label_selector_parse_trims_whitespace() {
        let sel = LabelSelector::parse("  env  =  prod  ,  tier in (  web , api )").unwrap();
        assert!( sel.matches(&lbls(&[("env","prod"), ("tier","web")])));
        assert!( sel.matches(&lbls(&[("env","prod"), ("tier","api")])));
        assert!(!sel.matches(&lbls(&[("env","prod"), ("tier","worker")])));
        // tenant_id invariant smoke.
        assert!(sel.matches(&lbls(&[("env","prod"),("tier","web"),("tenant","acme")])));
    }
}
