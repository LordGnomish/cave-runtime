// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/policy/CoordinatesPolicyEvaluator.java
//
//! Coordinates policy — match by (group, name, version) tuple with wildcards.

use crate::components::ComponentRecord;

/// Returns `Some(reason)` when the component matches the supplied coordinates.
/// Each coordinate may be `None` (wildcard).
pub fn violates(
    c: &ComponentRecord,
    group: Option<&str>,
    name: &str,
    version: Option<&str>,
) -> Option<String> {
    let g_ok = match group {
        None => true,
        Some(g) => c.group.as_deref() == Some(g),
    };
    let n_ok = name == "*" || c.name == name;
    let v_ok = match version {
        None | Some("*") => true,
        Some(v) => c.version == v,
    };
    if g_ok && n_ok && v_ok {
        Some(format!(
            "matches coordinates {}/{}@{}",
            group.unwrap_or("*"),
            name,
            version.unwrap_or("*")
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn comp(group: Option<&str>, name: &str, version: &str) -> ComponentRecord {
        let mut c = ComponentRecord::new(Uuid::new_v4(), name, version);
        c.group = group.map(|s| s.into());
        c
    }

    #[test]
    fn match_exact_triple() {
        let c = comp(Some("npm"), "lodash", "4.17.21");
        assert!(violates(&c, Some("npm"), "lodash", Some("4.17.21")).is_some());
    }

    #[test]
    fn wildcards_pass() {
        let c = comp(Some("npm"), "lodash", "4.17.21");
        assert!(violates(&c, None, "*", None).is_some());
    }

    #[test]
    fn version_mismatch_no_violation() {
        let c = comp(Some("npm"), "lodash", "4.17.21");
        assert!(violates(&c, Some("npm"), "lodash", Some("9.9.9")).is_none());
    }

    #[test]
    fn group_mismatch_no_violation() {
        let c = comp(Some("npm"), "lodash", "4.17.21");
        assert!(violates(&c, Some("maven"), "lodash", None).is_none());
    }
}
