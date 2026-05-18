// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/{Component,Project,ComponentIdentity}.java
//   src/main/java/org/dependencytrack/util/ComponentVersion.java
//
//! Component / Project management — version graph, identity hashing,
//! comparable semver-ish ordering.

use crate::models::ComponentType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Mirror of `org.dependencytrack.model.Project`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub uuid: Uuid,
    pub name: String,
    pub version: Option<String>,
    pub group: Option<String>,
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub swid: Option<String>,
    pub classifier: ComponentType,
    pub created_at: DateTime<Utc>,
    pub last_bom_import: Option<DateTime<Utc>>,
}

impl Project {
    pub fn new(name: impl Into<String>, version: Option<String>) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name: name.into(),
            version,
            group: None,
            purl: None,
            cpe: None,
            swid: None,
            classifier: ComponentType::Application,
            created_at: Utc::now(),
            last_bom_import: None,
        }
    }
}

/// Mirror of `org.dependencytrack.model.Component` (full storage variant).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentRecord {
    pub uuid: Uuid,
    pub project_uuid: Uuid,
    pub name: String,
    pub version: String,
    pub group: Option<String>,
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub swid: Option<String>,
    pub classifier: ComponentType,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub supplier: Option<String>,
    pub hash_md5: Option<String>,
    pub hash_sha1: Option<String>,
    pub hash_sha256: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

impl ComponentRecord {
    pub fn new(
        project_uuid: Uuid,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            project_uuid,
            name: name.into(),
            version: version.into(),
            group: None,
            purl: None,
            cpe: None,
            swid: None,
            classifier: ComponentType::Library,
            license: None,
            license_url: None,
            supplier: None,
            hash_md5: None,
            hash_sha1: None,
            hash_sha256: None,
            published_at: None,
        }
    }
}

/// Identity tuple used by Dependency-Track for de-duplication.
/// Mirrors `org.dependencytrack.model.ComponentIdentity.equals(Object)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComponentIdentity {
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub swid: Option<String>,
    pub group: Option<String>,
    pub name: String,
    pub version: String,
}

impl ComponentIdentity {
    pub fn from_record(r: &ComponentRecord) -> Self {
        Self {
            purl: r.purl.clone(),
            cpe: r.cpe.clone(),
            swid: r.swid.clone(),
            group: r.group.clone(),
            name: r.name.clone(),
            version: r.version.clone(),
        }
    }

    /// Two identities match if either purl matches exactly, or the
    /// (group, name, version) tuple matches. Mirrors upstream behaviour.
    pub fn matches(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (&self.purl, &other.purl) {
            if a == b {
                return true;
            }
        }
        self.group == other.group && self.name == other.name && self.version == other.version
    }
}

/// Best-effort comparable version parsing — handles SemVer + Maven-style.
/// Mirrors `org.dependencytrack.util.ComponentVersion.compareTo`.
///
/// Pre-release semantics (SemVer 2.0.0 §11):
///   "1.0.0-rc1" < "1.0.0" because a pre-release version has lower precedence.
pub fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    // Split off `-suffix` from `+build` and main release.
    let (main_a, pre_a) = split_release_prerelease(a);
    let (main_b, pre_b) = split_release_prerelease(b);
    let pa = parse_version_tokens(main_a);
    let pb = parse_version_tokens(main_b);
    match pa.cmp(&pb) {
        Ordering::Equal => {
            // Pre-release < release.
            match (pre_a.is_empty(), pre_b.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    let ta = parse_version_tokens(pre_a);
                    let tb = parse_version_tokens(pre_b);
                    ta.cmp(&tb)
                }
            }
        }
        other => other,
    }
}

fn split_release_prerelease(v: &str) -> (&str, &str) {
    // Strip build metadata first.
    let v = v.split('+').next().unwrap_or(v);
    match v.split_once('-') {
        Some((main, pre)) => (main, pre),
        None => (v, ""),
    }
}

fn parse_version_tokens(v: &str) -> Vec<Token> {
    // Split on `.`, `-`, `_`, `+`. Numbers are numeric tokens, anything else
    // is a textual token. Numeric > textual (matches Dependency-Track's
    // ComponentVersion which treats non-numeric as pre-release).
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut is_num = None;
    for c in v.chars() {
        if matches!(c, '.' | '-' | '_' | '+') {
            if !cur.is_empty() {
                tokens.push(emit_token(&cur, is_num));
                cur.clear();
                is_num = None;
            }
            continue;
        }
        let this_num = c.is_ascii_digit();
        if is_num.is_some() && is_num != Some(this_num) {
            tokens.push(emit_token(&cur, is_num));
            cur.clear();
        }
        cur.push(c);
        is_num = Some(this_num);
    }
    if !cur.is_empty() {
        tokens.push(emit_token(&cur, is_num));
    }
    tokens
}

fn emit_token(s: &str, is_num: Option<bool>) -> Token {
    if is_num == Some(true) {
        Token::Numeric(s.parse().unwrap_or(0))
    } else {
        Token::Text(s.to_lowercase())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Numeric(u64),
    Text(String),
}

impl PartialOrd for Token {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Token {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (Token::Numeric(a), Token::Numeric(b)) => a.cmp(b),
            (Token::Numeric(_), Token::Text(_)) => Ordering::Greater,
            (Token::Text(_), Token::Numeric(_)) => Ordering::Less,
            (Token::Text(a), Token::Text(b)) => a.cmp(b),
        }
    }
}

/// Build a {name → versions} index across a project's components.
pub fn version_index(records: &[ComponentRecord]) -> HashMap<String, Vec<String>> {
    let mut idx: HashMap<String, Vec<String>> = HashMap::new();
    for r in records {
        idx.entry(r.name.clone()).or_default().push(r.version.clone());
    }
    for v in idx.values_mut() {
        v.sort_by(|a, b| version_compare(a, b));
        v.dedup();
    }
    idx
}

/// Determine which projects contain a given component (by name).
pub fn projects_for_component(
    records: &[ComponentRecord],
    name: &str,
) -> Vec<Uuid> {
    let mut s: std::collections::BTreeSet<Uuid> = std::collections::BTreeSet::new();
    for r in records {
        if r.name == name {
            s.insert(r.project_uuid);
        }
    }
    s.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn project_new_initialises_uuid_and_classifier() {
        let p = Project::new("my-app", Some("1.0.0".into()));
        assert_eq!(p.name, "my-app");
        assert_eq!(p.version.as_deref(), Some("1.0.0"));
        assert_eq!(p.classifier, ComponentType::Application);
    }

    #[test]
    fn component_identity_matches_by_purl() {
        let pu = Uuid::new_v4();
        let mut a = ComponentRecord::new(pu, "lodash", "4.17.21");
        a.purl = Some("pkg:npm/lodash@4.17.21".into());
        let mut b = ComponentRecord::new(pu, "different", "9.9.9");
        b.purl = Some("pkg:npm/lodash@4.17.21".into());
        assert!(ComponentIdentity::from_record(&a).matches(&ComponentIdentity::from_record(&b)));
    }

    #[test]
    fn component_identity_matches_by_gnv_when_no_purl() {
        let pu = Uuid::new_v4();
        let mut a = ComponentRecord::new(pu, "lodash", "4.17.21");
        a.group = Some("npm".into());
        let mut b = ComponentRecord::new(pu, "lodash", "4.17.21");
        b.group = Some("npm".into());
        assert!(ComponentIdentity::from_record(&a).matches(&ComponentIdentity::from_record(&b)));
    }

    #[test]
    fn component_identity_no_match_on_different_gnv() {
        let pu = Uuid::new_v4();
        let a = ComponentRecord::new(pu, "lodash", "4.17.21");
        let b = ComponentRecord::new(pu, "lodash", "4.17.22");
        assert!(!ComponentIdentity::from_record(&a).matches(&ComponentIdentity::from_record(&b)));
    }

    #[test]
    fn version_compare_semver_basic() {
        assert_eq!(version_compare("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(version_compare("1.0.1", "1.0.0"), Ordering::Greater);
        assert_eq!(version_compare("1.2.0", "1.10.0"), Ordering::Less);
    }

    #[test]
    fn version_compare_with_text_qualifier_is_lower() {
        // 1.0.0-rc1 < 1.0.0 because "rc1" is text and text < numeric padding.
        assert_eq!(version_compare("1.0.0-rc1", "1.0.0"), Ordering::Less);
    }

    #[test]
    fn version_compare_lexical_text() {
        assert_eq!(version_compare("1.0.0-alpha", "1.0.0-beta"), Ordering::Less);
    }

    #[test]
    fn version_index_groups_and_sorts() {
        let p = Uuid::new_v4();
        let recs = vec![
            ComponentRecord::new(p, "lodash", "4.17.21"),
            ComponentRecord::new(p, "lodash", "4.17.20"),
            ComponentRecord::new(p, "lodash", "4.17.21"),
            ComponentRecord::new(p, "express", "4.18.0"),
        ];
        let idx = version_index(&recs);
        assert_eq!(idx["lodash"], vec!["4.17.20", "4.17.21"]);
        assert_eq!(idx["express"], vec!["4.18.0"]);
    }

    #[test]
    fn projects_for_component_returns_unique_uuids() {
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        let recs = vec![
            ComponentRecord::new(p1, "lodash", "1"),
            ComponentRecord::new(p2, "lodash", "1"),
            ComponentRecord::new(p1, "lodash", "2"),
        ];
        let pjs = projects_for_component(&recs, "lodash");
        assert_eq!(pjs.len(), 2);
        assert!(pjs.contains(&p1));
        assert!(pjs.contains(&p2));
    }
}
