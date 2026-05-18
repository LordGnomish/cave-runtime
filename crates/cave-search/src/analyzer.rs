// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Text analyzer — tokenisation + stop-word filtering.
//!
//! `tokenize` performs the equivalent of Lucene's StandardAnalyzer minimum
//! viable subset: lowercase + split on any non-alphanumeric ASCII char (which
//! is also how Manticore's `charset_table = 0..9,A..Z->a..z,a..z` default
//! behaves).  Unicode word boundaries are *not* applied — by design — so
//! callers keep deterministic, allocation-free hashing keys.
//!
//! `filter_stop_words` runs against a hard-coded English minimal stop list
//! (matches Manticore default `stopwords = en`).  The `_tenant_id` parameter
//! is plumbed in so per-tenant stop-word overrides can attach later without
//! a signature break.

use crate::tenant::TenantId;

const STOP_WORDS_EN: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in",
    "into", "is", "it", "no", "not", "of", "on", "or", "such", "that", "the",
    "their", "then", "there", "these", "they", "this", "to", "was", "will", "with",
];

pub fn tokenize(text: &str, _tenant_id: &TenantId) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            for low in ch.to_lowercase() {
                cur.push(low);
            }
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn filter_stop_words<'a>(tokens: Vec<&'a str>, _tenant_id: &TenantId) -> Vec<&'a str> {
    tokens
        .into_iter()
        .filter(|t| !STOP_WORDS_EN.contains(&t.to_ascii_lowercase().as_str()))
        .collect()
}
