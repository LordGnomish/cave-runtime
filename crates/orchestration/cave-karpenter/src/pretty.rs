// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Log-formatting helpers — port of `pkg/utils/pretty/pretty.go` from
//! kubernetes-sigs/karpenter v1.12.1 (sha ed490e8). Apache-2.0 upstream.
//!
//! These keep log lines and error messages bounded and readable: JSON-concise
//! marshaling, slice/map truncation with an "and N other(s)" tail, taint
//! pretty-printing, and the camelCase↔snake_case / sentence-case helpers used
//! for metric and condition names. The two upstream `regexp` passes in
//! `ToSnakeCase` are reproduced by hand so the crate stays regex-free.

use std::collections::BTreeMap;
use std::fmt::{Display, Write};

use serde::Serialize;

use crate::scheduling::taints::Taint;

/// `Concise`: compact JSON of `o`. On a marshal error, the error text is
/// returned (mirroring upstream returning `err.Error()`).
pub fn concise<T: Serialize>(o: &T) -> String {
    match serde_json::to_string(o) {
        Ok(s) => s,
        Err(e) => e.to_string(),
    }
}

/// `Slice`: render up to `max_items` elements, then ` and N other(s)`.
pub fn slice<T: Display>(s: &[T], max_items: usize) -> String {
    let mut sb = String::new();
    for (i, elem) in s.iter().enumerate() {
        if i > max_items.saturating_sub(1) {
            let _ = write!(sb, " and {} other(s)", s.len() - i);
            break;
        } else if i > 0 {
            sb.push_str(", ");
        }
        let _ = write!(sb, "{elem}");
    }
    sb
}

/// `Map`: render up to `max_items` sorted `k: v` entries, then ` and N
/// other(s)`. A `BTreeMap` supplies the sorted-key iteration upstream does
/// explicitly.
pub fn map<K: Ord + Display, V: Display>(values: &BTreeMap<K, V>, max_items: usize) -> String {
    let mut buf = String::new();
    let mut count = 0usize;
    for (k, v) in values {
        count += 1;
        if !buf.is_empty() {
            buf.push_str(", ");
        }
        let _ = write!(buf, "{k}: {v}");
        if count >= max_items {
            break;
        }
    }
    if count < values.len() {
        let _ = write!(buf, " and {} other(s)", values.len() - count);
    }
    buf
}

/// `Taint`: `key:effect`, or `key=value:effect` when a non-empty value is set.
pub fn taint(t: &Taint) -> String {
    match &t.value {
        Some(v) if !v.is_empty() => format!("{}={}:{}", t.key, v, t.effect),
        _ => format!("{}:{}", t.key, t.effect),
    }
}

/// First upstream regex pass: `(.)([A-Z][a-z]+)` → `${1}_${2}` — insert `_`
/// before an uppercase-then-lowercase run that follows any character.
fn split_first_cap(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(n + 4);
    let mut i = 0;
    while i < n {
        // group1 = chars[i] (any); group2 = [A-Z][a-z]+ at i+1..
        if i + 2 < n && chars[i + 1].is_ascii_uppercase() && chars[i + 2].is_ascii_lowercase() {
            out.push(chars[i]);
            out.push('_');
            out.push(chars[i + 1]);
            let mut j = i + 2;
            while j < n && chars[j].is_ascii_lowercase() {
                out.push(chars[j]);
                j += 1;
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Second upstream regex pass: `([a-z0-9])([A-Z])` → `${1}_${2}` — insert `_`
/// between a lowercase/digit and a following uppercase.
fn split_all_cap(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(n + 4);
    let mut i = 0;
    while i < n {
        if i + 1 < n
            && (chars[i].is_ascii_lowercase() || chars[i].is_ascii_digit())
            && chars[i + 1].is_ascii_uppercase()
        {
            out.push(chars[i]);
            out.push('_');
            out.push(chars[i + 1]);
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// `ToSnakeCase`: the two camelCase passes, lowercased.
pub fn to_snake_case(s: &str) -> String {
    split_all_cap(&split_first_cap(s)).to_lowercase()
}

/// `Sentence`: capitalize the first character. Empty input yields empty (Go
/// would panic on `str[0]`; failing soft is safer and behaviour-equivalent for
/// every real caller).
pub fn sentence(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
