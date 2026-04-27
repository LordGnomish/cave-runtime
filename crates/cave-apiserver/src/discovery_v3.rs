//! Discovery v3 — content negotiation, schema dedup with refs, paged
//! discovery cache. Layered atop M5 `discovery_v2` and M13 `aggregator_v3`.
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/apiserver/pkg/endpoints/discovery/aggregated/handler.go`
//!     (Accept-Encoding negotiation, hash-keyed caching, ETag).
//!   * `staging/src/k8s.io/kube-openapi/pkg/handler3/handler.go`
//!     (paged spec serving, ETag-based If-None-Match shortcut).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ─────────────────────────────────────────────────────────────────────────────
// Accept-Encoding parser — `gzip;q=1.0, deflate;q=0.5, *;q=0.1`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct EncodingPreference {
    pub coding: String, // gzip / deflate / br / identity / *
    pub q: f32,         // quality 0.0..=1.0
}

pub fn parse_accept_encoding(header: &str) -> Vec<EncodingPreference> {
    let mut out = vec![];
    if header.is_empty() { return out; }
    for token in header.split(',') {
        let mut parts = token.split(';').map(|s| s.trim());
        let Some(coding) = parts.next() else { continue };
        let mut q = 1.0_f32;
        for p in parts {
            if let Some(rest) = p.strip_prefix("q=") {
                if let Ok(v) = rest.parse() { q = v; }
            }
        }
        if !coding.is_empty() {
            out.push(EncodingPreference { coding: coding.to_lowercase(), q });
        }
    }
    out.sort_by(|a, b| b.q.partial_cmp(&a.q).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Pick the best supported encoding given client preferences and what the
/// server actually offers. Returns None when no overlap (or when only `q=0`
/// preferences match).
pub fn negotiate_encoding(
    prefs: &[EncodingPreference], offered: &[&str],
) -> Option<String> {
    let offered_lower: Vec<String> = offered.iter().map(|s| s.to_lowercase()).collect();
    for p in prefs {
        if p.q <= 0.0 { continue; }
        if p.coding == "*" {
            // pick first offered that isn't explicitly q=0 in prefs
            for o in &offered_lower {
                if !prefs.iter().any(|x| x.coding == *o && x.q <= 0.0) {
                    return Some(o.clone());
                }
            }
            continue;
        }
        if offered_lower.contains(&p.coding) { return Some(p.coding.clone()); }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// If-None-Match handling — `handler.go::serveHTTP`. The handler short-circuits
// 304 Not Modified when the client's ETag matches.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheDecision {
    /// Serve full body.
    Send,
    /// Short-circuit with 304 Not Modified.
    NotModified,
}

pub fn etag_match(server_etag: &str, if_none_match: Option<&str>) -> CacheDecision {
    let Some(client) = if_none_match else { return CacheDecision::Send; };
    // RFC 7232: weak ETags marked W/. Equality is "weak comparison".
    let strip_weak = |s: &str| -> String {
        s.strip_prefix("W/").unwrap_or(s).trim().to_string()
    };
    let s = strip_weak(server_etag);
    for tok in client.split(',') {
        let t = strip_weak(tok.trim());
        if t == "*" || t == s { return CacheDecision::NotModified; }
    }
    CacheDecision::Send
}

// ─────────────────────────────────────────────────────────────────────────────
// Schema $ref rewriter — when components are namespaced via collision, all
// `$ref: "#/components/schemas/X"` references must be rewritten to point at
// the namespaced symbol.
// ─────────────────────────────────────────────────────────────────────────────

pub fn rewrite_refs(
    schema: &serde_json::Value, name_map: &BTreeMap<String, String>,
) -> serde_json::Value {
    match schema {
        serde_json::Value::Object(m) => {
            let mut out = serde_json::Map::new();
            for (k, v) in m {
                if k == "$ref" {
                    if let Some(s) = v.as_str() {
                        if let Some(rest) = s.strip_prefix("#/components/schemas/") {
                            if let Some(replacement) = name_map.get(rest) {
                                out.insert(k.clone(), serde_json::Value::String(
                                    format!("#/components/schemas/{replacement}")));
                                continue;
                            }
                        }
                    }
                }
                out.insert(k.clone(), rewrite_refs(v, name_map));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| rewrite_refs(v, name_map)).collect())
        }
        other => other.clone(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PagedDiscoveryCache — handler3 caches each paged result by its hash so
// repeated GETs are O(1). The cache evicts when the upstream spec hash
// changes.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CachedPage {
    pub hash: String,
    pub bytes: Vec<u8>,
}

pub struct PagedDiscoveryCache {
    inner: std::sync::RwLock<BTreeMap<String, CachedPage>>, // gv → page
}

impl PagedDiscoveryCache {
    pub fn new() -> Self { Self { inner: Default::default() } }

    pub fn get(&self, gv: &str) -> Option<CachedPage> {
        self.inner.read().unwrap().get(gv).cloned()
    }

    pub fn put(&self, gv: &str, hash: &str, bytes: Vec<u8>) {
        self.inner.write().unwrap().insert(
            gv.into(), CachedPage { hash: hash.into(), bytes });
    }

    /// Returns (page, hit) — true when the requested hash matched an existing
    /// entry; on miss the cache is left untouched.
    pub fn try_hit(&self, gv: &str, want_hash: &str) -> (Option<CachedPage>, bool) {
        let g = self.inner.read().unwrap();
        match g.get(gv) {
            Some(p) if p.hash == want_hash => (Some(p.clone()), true),
            _ => (None, false),
        }
    }

    pub fn invalidate(&self, gv: &str) {
        self.inner.write().unwrap().remove(gv);
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

impl Default for PagedDiscoveryCache { fn default() -> Self { Self::new() } }

// ─────────────────────────────────────────────────────────────────────────────
// Content type negotiation — `application/json` (default), protobuf,
// yaml, table-mode (kubectl get).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseFormat {
    Json,
    Protobuf,
    Yaml,
    TableV1,
}

pub fn negotiate_content_type(accept: &str) -> ResponseFormat {
    // Walk the Accept list in order; first known match wins.
    for tok in accept.split(',') {
        let main = tok.split(';').next().unwrap_or("").trim().to_lowercase();
        match main.as_str() {
            "application/json" => return ResponseFormat::Json,
            "application/yaml" => return ResponseFormat::Yaml,
            "application/vnd.kubernetes.protobuf" => return ResponseFormat::Protobuf,
            "application/json;as=table;v=v1;g=meta.k8s.io" => return ResponseFormat::TableV1,
            _ => {}
        }
        // Tolerate the table-mode media-type with parameters split across `;`.
        if main == "application/json"
            && tok.contains("as=table") && tok.contains("v=v1")
        {
            return ResponseFormat::TableV1;
        }
    }
    ResponseFormat::Json
}

// ─────────────────────────────────────────────────────────────────────────────
// Hash list — a tiny version-stamp computed from a list of strings, used to
// derive the OpenAPI v3 index entry hash. FNV-1a.
// ─────────────────────────────────────────────────────────────────────────────

pub fn hash_list(items: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for s in items {
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= b'\n' as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests;
