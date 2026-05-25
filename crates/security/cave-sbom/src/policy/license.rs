// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/LicensePolicyEvaluator.java
//
//! License policy evaluator — allow-list and deny-list semantics.
//!
//! Allow-list: component license MUST be in `allow`. Missing license = violation.
//! Deny-list: component license MUST NOT be in `deny`. Missing license = no violation.

use crate::components::ComponentRecord;

pub fn violates_allow(c: &ComponentRecord, allow: &[String]) -> Option<String> {
    match c.license.as_deref() {
        Some(lic) if allow.iter().any(|a| eq_ignore_ascii(a, lic)) => None,
        Some(lic) => Some(format!("license `{}` not in allow-list", lic)),
        None => Some("component has no declared license".to_string()),
    }
}

pub fn violates_deny(c: &ComponentRecord, deny: &[String]) -> Option<String> {
    match c.license.as_deref() {
        Some(lic) if deny.iter().any(|d| eq_ignore_ascii(d, lic)) => {
            Some(format!("license `{}` is in deny-list", lic))
        }
        _ => None,
    }
}

fn eq_ignore_ascii(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn comp(license: Option<&str>) -> ComponentRecord {
        let mut c = ComponentRecord::new(Uuid::new_v4(), "x", "1");
        c.license = license.map(|s| s.into());
        c
    }

    #[test]
    fn allow_pass_when_license_in_list() {
        assert!(violates_allow(&comp(Some("MIT")), &["MIT".into(), "Apache-2.0".into()]).is_none());
    }

    #[test]
    fn allow_violates_when_license_not_in_list() {
        let v = violates_allow(&comp(Some("GPL-3.0")), &["MIT".into()]);
        assert!(v.is_some());
        assert!(v.unwrap().contains("GPL-3.0"));
    }

    #[test]
    fn allow_violates_when_no_license() {
        assert!(violates_allow(&comp(None), &["MIT".into()]).is_some());
    }

    #[test]
    fn allow_is_case_insensitive() {
        assert!(violates_allow(&comp(Some("mit")), &["MIT".into()]).is_none());
    }

    #[test]
    fn deny_violates_when_license_in_list() {
        let v = violates_deny(&comp(Some("GPL-3.0")), &["GPL-3.0".into()]);
        assert!(v.is_some());
    }

    #[test]
    fn deny_passes_when_license_not_in_list() {
        assert!(violates_deny(&comp(Some("MIT")), &["GPL-3.0".into()]).is_none());
    }

    #[test]
    fn deny_passes_when_no_license() {
        assert!(violates_deny(&comp(None), &["GPL-3.0".into()]).is_none());
    }
}
