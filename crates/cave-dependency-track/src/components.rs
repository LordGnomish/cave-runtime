// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Component identity + analysis-cache key.
//!
//! Mirrors `org.dependencytrack.model.ComponentIdentity` —
//! the (purl ∪ cpe ∪ swid ∪ hash) tuple Dependency-Track uses to detect
//! whether an inbound component is "the same as" one already on file.

use crate::models::Component;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComponentIdentity {
    pub purl: Option<String>,
    pub cpe: Option<String>,
    pub swid_tag_id: Option<String>,
    pub group: Option<String>,
    pub name: String,
    pub version: Option<String>,
}

impl ComponentIdentity {
    pub fn of(c: &Component) -> Self {
        Self {
            purl: c.purl.clone(),
            cpe: c.cpe.clone(),
            swid_tag_id: c.swid_tag_id.clone(),
            group: c.group.clone(),
            name: c.name.clone(),
            version: c.version.clone(),
        }
    }

    /// Stable cache key — first non-empty in priority order, then
    /// `(group, name, version)`.  Matches `ComponentAnalysisCache#getKey`.
    pub fn cache_key(&self) -> String {
        if let Some(p) = &self.purl {
            return format!("purl:{}", p);
        }
        if let Some(c) = &self.cpe {
            return format!("cpe:{}", c);
        }
        if let Some(s) = &self.swid_tag_id {
            return format!("swid:{}", s);
        }
        format!(
            "gnv:{}/{}@{}",
            self.group.as_deref().unwrap_or(""),
            self.name,
            self.version.as_deref().unwrap_or(""),
        )
    }
}

/// In-memory analysis cache — `ComponentAnalysisCache` parity.
#[derive(Debug, Default)]
pub struct AnalysisCache<V: Clone> {
    inner: HashMap<String, V>,
}

impl<V: Clone> AnalysisCache<V> {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    pub fn put(&mut self, id: &ComponentIdentity, value: V) {
        self.inner.insert(id.cache_key(), value);
    }

    pub fn get(&self, id: &ComponentIdentity) -> Option<V> {
        self.inner.get(&id.cache_key()).cloned()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

/// Internal-component detection — `InternalComponentIdentificationTask` parity.
/// Returns `true` when the component matches `internal_group_prefix` or its
/// PURL namespace is in `internal_namespaces`.
pub fn is_internal(
    c: &Component,
    internal_group_prefix: &[String],
    internal_namespaces: &[String],
) -> bool {
    if let Some(g) = &c.group {
        if internal_group_prefix.iter().any(|p| g.starts_with(p)) {
            return true;
        }
    }
    if let Some(p) = &c.purl {
        for ns in internal_namespaces {
            if p.contains(&format!("/{}/", ns)) || p.contains(&format!(":{}/", ns)) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Classifier;
    use uuid::Uuid;

    fn comp(purl: Option<&str>, cpe: Option<&str>, name: &str) -> Component {
        let mut c = Component::new(Uuid::new_v4(), name);
        c.purl = purl.map(|s| s.into());
        c.cpe = cpe.map(|s| s.into());
        c.classifier = Classifier::Library;
        c
    }

    #[test]
    fn purl_wins_over_cpe() {
        let c = comp(Some("pkg:cargo/serde@1"), Some("cpe:2.3:..."), "serde");
        let id = ComponentIdentity::of(&c);
        assert!(id.cache_key().starts_with("purl:"));
    }

    #[test]
    fn cpe_used_when_no_purl() {
        let c = comp(None, Some("cpe:2.3:a:openssl:openssl:3"), "openssl");
        let id = ComponentIdentity::of(&c);
        assert!(id.cache_key().starts_with("cpe:"));
    }

    #[test]
    fn falls_back_to_gnv_tuple() {
        let mut c = comp(None, None, "lib");
        c.version = Some("1".into());
        c.group = Some("org".into());
        let id = ComponentIdentity::of(&c);
        assert_eq!(id.cache_key(), "gnv:org/lib@1");
    }

    #[test]
    fn cache_get_put_roundtrip() {
        let mut cache: AnalysisCache<u8> = AnalysisCache::new();
        let id = ComponentIdentity::of(&comp(Some("pkg:cargo/x@1"), None, "x"));
        cache.put(&id, 42);
        assert_eq!(cache.get(&id), Some(42));
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn internal_detection_by_group_prefix() {
        let mut c = comp(None, None, "x");
        c.group = Some("io.cave.internal".into());
        assert!(is_internal(
            &c,
            &["io.cave.".to_string()],
            &[],
        ));
    }

    #[test]
    fn internal_detection_by_purl_namespace() {
        let c = comp(Some("pkg:maven/io.cave/private@1.0"), None, "private");
        assert!(is_internal(
            &c,
            &[],
            &["io.cave".to_string()],
        ));
    }

    #[test]
    fn external_component_not_internal() {
        let c = comp(Some("pkg:cargo/serde@1"), None, "serde");
        assert!(!is_internal(
            &c,
            &["io.cave.".to_string()],
            &["io.cave".to_string()],
        ));
    }
}
