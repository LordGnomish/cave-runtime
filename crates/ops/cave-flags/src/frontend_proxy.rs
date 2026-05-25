// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Frontend-proxy mode — parity with `src/lib/proxy/*` (Unleash v5.0.0).
//!
//! Cached feature snapshots are served to browser SDKs via the frontend
//! proxy endpoints. Evaluation is performed server-side using the
//! provided `UnleashContext`; the proxy returns only `enabled` features
//! and never leaks strategy definitions to the client.

use crate::engine::evaluate_flag;
use crate::models::{
    EvaluatedVariant, FeatureFlag, FrontendFeaturesResponse, FrontendToggle, Segment, UnleashContext,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Snapshot cache — refreshed on a fixed interval (the runtime calls
/// `refresh` before each request batch and `is_stale` for eager refresh).
pub struct ProxyCache {
    inner: Mutex<ProxyCacheInner>,
    ttl: Duration,
}

struct ProxyCacheInner {
    features: Vec<FeatureFlag>,
    segments: Vec<Segment>,
    last_refresh: Option<Instant>,
}

impl ProxyCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: Mutex::new(ProxyCacheInner {
                features: Vec::new(),
                segments: Vec::new(),
                last_refresh: None,
            }),
        }
    }

    pub fn refresh(&self, features: Vec<FeatureFlag>, segments: Vec<Segment>) {
        let mut g = self.inner.lock().unwrap();
        g.features = features;
        g.segments = segments;
        g.last_refresh = Some(Instant::now());
    }

    pub fn is_stale(&self) -> bool {
        let g = self.inner.lock().unwrap();
        match g.last_refresh {
            None => true,
            Some(t) => t.elapsed() >= self.ttl,
        }
    }

    pub fn feature_count(&self) -> usize {
        self.inner.lock().unwrap().features.len()
    }

    /// Build the proxy response for the supplied context. Returns only
    /// the toggles that evaluated to `enabled = true` (matches the
    /// frontend-proxy contract in upstream Unleash).
    pub fn evaluate(&self, ctx: &UnleashContext, environment: &str) -> FrontendFeaturesResponse {
        let g = self.inner.lock().unwrap();
        let mut segments: HashMap<i64, &Segment> = HashMap::with_capacity(g.segments.len());
        for s in g.segments.iter() {
            segments.insert(s.id, s);
        }
        let mut toggles = Vec::new();
        for f in g.features.iter() {
            let r = evaluate_flag(f, environment, ctx, &segments);
            if !r.enabled {
                continue;
            }
            toggles.push(FrontendToggle {
                name: f.name.clone(),
                enabled: true,
                variant: r.variant,
                impression_data: f.impression_data,
            });
        }
        FrontendFeaturesResponse { toggles }
    }

    /// Same as `evaluate` but also returns a stable list of disabled
    /// flags (used by SDK polling). Disabled toggles are serialized
    /// with `enabled=false` and a "disabled" variant.
    pub fn evaluate_all(
        &self,
        ctx: &UnleashContext,
        environment: &str,
    ) -> FrontendFeaturesResponse {
        let g = self.inner.lock().unwrap();
        let mut segments: HashMap<i64, &Segment> = HashMap::with_capacity(g.segments.len());
        for s in g.segments.iter() {
            segments.insert(s.id, s);
        }
        let mut toggles = Vec::new();
        for f in g.features.iter() {
            let r = evaluate_flag(f, environment, ctx, &segments);
            let variant = if r.enabled {
                r.variant
            } else {
                EvaluatedVariant {
                    name: "disabled".into(),
                    enabled: false,
                    payload: None,
                    feature_enabled: false,
                }
            };
            toggles.push(FrontendToggle {
                name: f.name.clone(),
                enabled: r.enabled,
                variant,
                impression_data: f.impression_data,
            });
        }
        FrontendFeaturesResponse { toggles }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Constraint, FeatureEnvironment, FeatureStrategy, FeatureType, Variant, WeightType,
    };
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn default_strategy() -> FeatureStrategy {
        FeatureStrategy {
            id: Uuid::nil(),
            name: "default".into(),
            parameters: HashMap::new(),
            constraints: Vec::<Constraint>::new(),
            segments: vec![],
            sort_order: 0,
            variants: vec![],
            disabled: false,
        }
    }

    fn flag(name: &str, enabled: bool) -> FeatureFlag {
        FeatureFlag {
            name: name.into(),
            feature_type: FeatureType::Release,
            description: String::new(),
            enabled,
            stale: false,
            impression_data: false,
            project: "default".into(),
            created_at: Utc::now(),
            last_seen_at: None,
            strategies: vec![default_strategy()],
            variants: vec![],
            environments: vec![FeatureEnvironment {
                name: "default".into(),
                enabled,
                strategies: vec![default_strategy()],
                variants: vec![],
            }],
            tags: vec![],
        }
    }

    #[test]
    fn cache_is_stale_when_never_refreshed() {
        let c = ProxyCache::new(Duration::from_secs(1));
        assert!(c.is_stale());
    }

    #[test]
    fn cache_not_stale_after_refresh() {
        let c = ProxyCache::new(Duration::from_secs(60));
        c.refresh(vec![flag("a", true)], vec![]);
        assert!(!c.is_stale());
        assert_eq!(c.feature_count(), 1);
    }

    #[test]
    fn evaluate_returns_only_enabled() {
        let c = ProxyCache::new(Duration::from_secs(60));
        c.refresh(vec![flag("a", true), flag("b", false)], vec![]);
        let resp = c.evaluate(&UnleashContext::default(), "default");
        assert_eq!(resp.toggles.len(), 1);
        assert_eq!(resp.toggles[0].name, "a");
    }

    #[test]
    fn evaluate_response_has_disabled_variant_when_no_variants() {
        let c = ProxyCache::new(Duration::from_secs(60));
        c.refresh(vec![flag("a", true)], vec![]);
        let resp = c.evaluate(&UnleashContext::default(), "default");
        assert_eq!(resp.toggles[0].variant.name, "disabled");
    }

    #[test]
    fn evaluate_picks_variant_deterministically() {
        let mut f = flag("a", true);
        let variants = vec![
            Variant {
                name: "blue".into(),
                weight: 500,
                weight_type: WeightType::Variable,
                stickiness: "default".into(),
                overrides: vec![],
                payload: None,
            },
            Variant {
                name: "green".into(),
                weight: 500,
                weight_type: WeightType::Variable,
                stickiness: "default".into(),
                overrides: vec![],
                payload: None,
            },
        ];
        f.variants = variants.clone();
        f.environments[0].variants = variants;
        let c = ProxyCache::new(Duration::from_secs(60));
        c.refresh(vec![f], vec![]);
        let ctx = UnleashContext {
            user_id: Some("u1".into()),
            ..Default::default()
        };
        let r1 = c.evaluate(&ctx, "default");
        let r2 = c.evaluate(&ctx, "default");
        assert_eq!(r1.toggles[0].variant.name, r2.toggles[0].variant.name);
    }

    #[test]
    fn evaluate_empty_cache_returns_empty() {
        let c = ProxyCache::new(Duration::from_secs(60));
        let resp = c.evaluate(&UnleashContext::default(), "default");
        assert!(resp.toggles.is_empty());
    }

    #[test]
    fn refresh_updates_feature_count() {
        let c = ProxyCache::new(Duration::from_secs(60));
        c.refresh(vec![flag("a", true)], vec![]);
        assert_eq!(c.feature_count(), 1);
        c.refresh(vec![flag("a", true), flag("b", true), flag("c", true)], vec![]);
        assert_eq!(c.feature_count(), 3);
    }

    #[test]
    fn evaluate_all_returns_disabled_too() {
        let c = ProxyCache::new(Duration::from_secs(60));
        c.refresh(vec![flag("a", true), flag("b", false)], vec![]);
        let resp = c.evaluate_all(&UnleashContext::default(), "default");
        assert_eq!(resp.toggles.len(), 2);
        let b = resp.toggles.iter().find(|t| t.name == "b").unwrap();
        assert!(!b.enabled);
        assert_eq!(b.variant.name, "disabled");
    }
}
