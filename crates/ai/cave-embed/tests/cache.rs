// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// TDD cycle 6 — embedding result cache (LRU memoization).
//
// Repeated identical inputs (common in RAG re-indexing) should not re-run the
// model. The cache keys on (model, dimensions, input) and evicts least-recently-
// used entries past a capacity bound, tracking hit/miss counts. The service
// transparently memoizes per input so a second identical request is a full hit.

use cave_embed::api::{EmbeddingRequest, Input};
use cave_embed::cache::EmbeddingCache;
use cave_embed::service::EmbeddingService;

#[test]
fn key_is_model_dims_and_input_sensitive() {
    let k1 = EmbeddingCache::key("m", None, "hello");
    let k2 = EmbeddingCache::key("m", None, "hello");
    let k3 = EmbeddingCache::key("m", None, "world");
    let k4 = EmbeddingCache::key("m2", None, "hello");
    let k5 = EmbeddingCache::key("m", Some(64), "hello");
    assert_eq!(k1, k2);
    assert_ne!(k1, k3);
    assert_ne!(k1, k4);
    assert_ne!(k1, k5);
}

#[test]
fn get_miss_then_hit() {
    let cache = EmbeddingCache::new(8);
    let k = EmbeddingCache::key("m", None, "x");
    assert!(cache.get(&k).is_none());
    cache.put(k.clone(), vec![1.0, 2.0]);
    assert_eq!(cache.get(&k), Some(vec![1.0, 2.0]));
    let (hits, misses) = cache.stats();
    assert_eq!((hits, misses), (1, 1));
}

#[test]
fn lru_evicts_least_recently_used() {
    let cache = EmbeddingCache::new(2);
    cache.put("a".into(), vec![1.0]);
    cache.put("b".into(), vec![2.0]);
    // touch "a" so "b" becomes LRU
    assert!(cache.get("a").is_some());
    cache.put("c".into(), vec![3.0]); // evicts "b"
    assert!(cache.get("b").is_none());
    assert!(cache.get("a").is_some());
    assert!(cache.get("c").is_some());
    assert_eq!(cache.len(), 2);
}

#[test]
fn capacity_zero_disables_cache() {
    let cache = EmbeddingCache::new(0);
    cache.put("a".into(), vec![1.0]);
    assert!(cache.get("a").is_none());
    assert_eq!(cache.len(), 0);
}

#[test]
fn service_memoizes_repeated_inputs() {
    let svc = EmbeddingService::with_builtins().with_cache(128);
    let req = EmbeddingRequest {
        model: "minilm".into(),
        input: Input::Single("repeated text".into()),
        encoding_format: None,
        dimensions: None,
        user: None,
    };
    let a = svc.embed(&req).unwrap();
    let b = svc.embed(&req).unwrap();
    // identical output
    assert_eq!(
        format!("{:?}", a.data[0].embedding),
        format!("{:?}", b.data[0].embedding)
    );
    let (hits, _misses) = svc.cache_stats().unwrap();
    assert!(hits >= 1, "second identical request must hit the cache");
}
