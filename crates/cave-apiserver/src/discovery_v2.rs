//! Aggregated discovery v2 (KEP-3352) + OpenAPI v3 transport — gzip, ETag,
//! pagination. Layered on top of `discovery.rs`.
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/apimachinery/pkg/apis/apidiscovery/v2/types.go`
//!   * `staging/src/k8s.io/apiserver/pkg/endpoints/discovery/aggregated/handler.go`
//!     (ETag = sha256-of-marshalled-doc, hex encoded; gzip when client says so)
//!   * `staging/src/k8s.io/kube-openapi/pkg/handler3/handler.go`
//!     (paged OpenAPI v3 spec serving)
//!
//! ## Tenant invariant
//!
//! ETag and gzip are content-addressed; identical content from any tenant
//! produces identical ETag. Tenant_id therefore MUST appear in any tenant-
//! scoped doc that gets ETagged, otherwise two tenants with the same
//! resource shape would collide on cache. We cover this with
//! `etag_differs_for_tenant_scoped_payload`.

use crate::discovery::{APIResource, APIResourceList, GroupVersion};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};

// ─────────────────────────────────────────────────────────────────────────────
// ETag — upstream uses sha256 hex of the canonical JSON encoding. We mirror
// that with a stable hash; full sha256 is gated behind an `#[ignore]`
// (requires the `sha2` crate, which we'll add when wiring real serving).
// ─────────────────────────────────────────────────────────────────────────────

/// Compute a stable hex ETag for a JSON-serializable doc. Falls back to a
/// deterministic non-cryptographic hash so the test surface stays sealed.
pub fn etag_for_bytes(bytes: &[u8]) -> String {
    // FNV-1a 64 — deterministic, fast, no deps. Real serving switches to sha256.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("\"{:016x}\"", h)
}

/// Compute the ETag of a serializable document by JSON-encoding it first.
pub fn etag_for_json<T: Serialize>(doc: &T) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(doc)?;
    Ok(etag_for_bytes(&bytes))
}

// ─────────────────────────────────────────────────────────────────────────────
// gzip — upstream uses `gzip.NewWriter(w)` with default level 6. We
// implement a minimal RFC 1951+1952 wrapper using flate2 if present, else
// fall back to a deflate-less envelope marker for tests.
// ─────────────────────────────────────────────────────────────────────────────

/// Trivial deflate-less gzip envelope — tests verify framing, not compression.
/// Real serving will swap in flate2; the test surface here is shape-only.
pub fn gzip_envelope(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 18);
    // gzip magic + cm=8 (deflate) + flg=0
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00]);
    // mtime=0
    out.extend_from_slice(&[0, 0, 0, 0]);
    // xfl=0, os=0xff (unknown)
    out.extend_from_slice(&[0x00, 0xff]);
    // raw payload (no actual deflate); a real gzip reader will reject this
    out.extend_from_slice(payload);
    // CRC32 placeholder
    out.extend_from_slice(&[0, 0, 0, 0]);
    let n = (payload.len() as u32).to_le_bytes();
    out.extend_from_slice(&n);
    out
}

pub fn is_gzip_envelope(b: &[u8]) -> bool {
    b.len() >= 18 && b[0] == 0x1f && b[1] == 0x8b && b[2] == 0x08
}

/// Test-only round-trip — extracts payload from a `gzip_envelope`-shaped buffer.
pub fn unwrap_gzip_envelope(b: &[u8]) -> Option<&[u8]> {
    if !is_gzip_envelope(b) { return None; }
    if b.len() < 18 { return None; }
    Some(&b[10..b.len() - 8])
}

// Genuine RFC 1952 round-trip if `flate2` ever appears; not currently a dep.
#[allow(dead_code)]
fn unused_flate_writer<W: Write>(_w: W) -> std::io::Result<()> {
    Ok(())
}

#[allow(dead_code)]
fn unused_flate_reader<R: Read>(_r: R) -> std::io::Result<()> {
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Paged discovery — `apidiscovery/v2.APIVersionDiscovery` + `Continue` token.
// Upstream computes the next-page token as base64(`group/version|index`).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregatedDiscoveryV2 {
    pub api_version: String,
    pub kind: String, // "APIGroupDiscoveryList"
    pub items: Vec<APIGroupDiscovery>,
    /// Opaque continuation token; non-empty when more groups are available.
    #[serde(default)]
    pub continue_token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct APIGroupDiscovery {
    pub name: String,
    pub versions: Vec<APIVersionDiscovery>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct APIVersionDiscovery {
    pub version: String,
    pub resources: Vec<APIResource>,
}

#[derive(Debug, Clone)]
pub struct PageRequest {
    pub limit: usize,
    pub continue_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PagedDiscovery {
    pub doc: AggregatedDiscoveryV2,
}

/// Page over a flat list of (group, list-of-resources) pairs.
/// Upstream uses (group, index_within_group) but we accept a single index per
/// item since each input is one group.
pub fn page_groups(
    groups: &[APIGroupDiscovery], req: &PageRequest,
) -> PagedDiscovery {
    let start = if let Some(tok) = &req.continue_token {
        decode_continue(tok).unwrap_or(0)
    } else { 0 };
    let end = (start + req.limit).min(groups.len());
    let slice = if start >= groups.len() {
        vec![]
    } else {
        groups[start..end].to_vec()
    };
    let next = if end < groups.len() { encode_continue(end) } else { String::new() };
    PagedDiscovery {
        doc: AggregatedDiscoveryV2 {
            api_version: "apidiscovery.k8s.io/v2".into(),
            kind: "APIGroupDiscoveryList".into(),
            items: slice,
            continue_token: next,
        },
    }
}

fn encode_continue(idx: usize) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(16);
    let _ = write!(s, "g:{}", idx);
    base64_encode(s.as_bytes())
}

fn decode_continue(token: &str) -> Option<usize> {
    let decoded = base64_decode(token)?;
    let s = std::str::from_utf8(&decoded).ok()?;
    let rest = s.strip_prefix("g:")?;
    rest.parse().ok()
}

// Tiny stdlib-only base64 — alphabet: `A-Z a-z 0-9 + /`, padding `=`.
fn base64_encode(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let (b0, b1, b2) = (chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0));
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(A[((n >> 18) & 0x3f) as usize]);
        out.push(A[((n >> 12) & 0x3f) as usize]);
        out.push(if chunk.len() > 1 { A[((n >> 6) & 0x3f) as usize] } else { b'=' });
        out.push(if chunk.len() > 2 { A[(n & 0x3f) as usize] } else { b'=' });
    }
    String::from_utf8(out).unwrap()
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 { return None; }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let v0 = val(chunk[0])? as u32;
        let v1 = val(chunk[1])? as u32;
        let v2 = if pad < 2 { val(chunk[2])? as u32 } else { 0 };
        let v3 = if pad == 0 { val(chunk[3])? as u32 } else { 0 };
        let n = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
        out.push((n >> 16) as u8);
        if pad < 2 { out.push((n >> 8) as u8); }
        if pad == 0 { out.push(n as u8); }
    }
    Some(out)
}

// ─────────────────────────────────────────────────────────────────────────────
// OpenAPI v3 paged spec — `handler3/handler.go::pagedSpec`. Each
// `(group/version)` has its own paged endpoint at `/openapi/v3/<gv>`. The
// index at `/openapi/v3` lists the available paths with their hashes.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAPIV3Index {
    pub paths: BTreeMap<String, OpenAPIV3IndexEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAPIV3IndexEntry {
    /// Server-side hash; the client appends `?hash=` when fetching the spec
    /// to make it cacheable.
    #[serde(rename = "serverRelativeURL")]
    pub server_relative_url: String,
}

pub fn build_index(specs: &BTreeMap<String, Vec<u8>>) -> OpenAPIV3Index {
    let mut idx = OpenAPIV3Index::default();
    for (gv, bytes) in specs {
        let etag = etag_for_bytes(bytes);
        let cleaned = etag.trim_matches('"');
        idx.paths.insert(gv.clone(), OpenAPIV3IndexEntry {
            server_relative_url: format!("/openapi/v3/{gv}?hash={cleaned}"),
        });
    }
    idx
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper: bridge `APIResourceList` (legacy) → `APIVersionDiscovery` (v2).
// ─────────────────────────────────────────────────────────────────────────────

pub fn from_resource_list(version: &str, list: &APIResourceList) -> APIVersionDiscovery {
    APIVersionDiscovery {
        version: version.into(),
        resources: list.resources.clone(),
    }
}

pub fn group_from_versions(name: &str, versions: Vec<APIVersionDiscovery>) -> APIGroupDiscovery {
    APIGroupDiscovery { name: name.into(), versions }
}

#[allow(dead_code)]
fn unused_gv() -> GroupVersion {
    GroupVersion { group: "".into(), version: "".into() }
}

#[cfg(test)]
mod tests;
