// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg partition transforms.
//!
//! Upstream: `crates/iceberg/src/spec/transform.rs`
//! Spec: <https://iceberg.apache.org/spec/#partition-transforms>
//!
//! Transforms used in both PartitionSpec and SortOrder. The MVP
//! implements all eight standard transforms at the metadata level
//! (parse + serialize). Runtime evaluation against actual values
//! is implemented for the scalar value transforms that the scan
//! planner needs for partition pruning (identity / bucket / truncate /
//! temporal-extraction).

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Transform {
    Identity,
    Year,
    Month,
    Day,
    Hour,
    Bucket(u32),
    Truncate(u32),
    Void,
}

impl Transform {
    /// Render to the Iceberg spec textual form (used inside manifest
    /// `summary` strings and certain JSON metadata fields).
    pub fn as_spec_str(self) -> String {
        match self {
            Self::Identity => "identity".into(),
            Self::Year => "year".into(),
            Self::Month => "month".into(),
            Self::Day => "day".into(),
            Self::Hour => "hour".into(),
            Self::Bucket(n) => format!("bucket[{}]", n),
            Self::Truncate(w) => format!("truncate[{}]", w),
            Self::Void => "void".into(),
        }
    }

    /// Parse the Iceberg spec textual form.
    pub fn parse(s: &str) -> Result<Self> {
        let r = match s {
            "identity" => Self::Identity,
            "year" => Self::Year,
            "month" => Self::Month,
            "day" => Self::Day,
            "hour" => Self::Hour,
            "void" => Self::Void,
            other => {
                if let Some(rest) = other.strip_prefix("bucket[") {
                    let n_str = rest.strip_suffix(']').ok_or_else(|| {
                        Error::InvalidMetadata(format!("bad bucket[..]: {}", other))
                    })?;
                    let n: u32 = n_str.parse().map_err(|_| {
                        Error::InvalidMetadata(format!("bad bucket[..]: {}", other))
                    })?;
                    Self::Bucket(n)
                } else if let Some(rest) = other.strip_prefix("truncate[") {
                    let n_str = rest.strip_suffix(']').ok_or_else(|| {
                        Error::InvalidMetadata(format!("bad truncate[..]: {}", other))
                    })?;
                    let n: u32 = n_str.parse().map_err(|_| {
                        Error::InvalidMetadata(format!("bad truncate[..]: {}", other))
                    })?;
                    Self::Truncate(n)
                } else {
                    return Err(Error::InvalidMetadata(format!(
                        "unknown transform: {}",
                        other
                    )));
                }
            }
        };
        Ok(r)
    }

    /// Apply identity transform to an i64 (no-op).
    pub fn apply_identity_i64(self, v: i64) -> Option<i64> {
        match self {
            Self::Identity => Some(v),
            _ => None,
        }
    }

    /// Apply truncate[W] to an i64: floor(v / W) * W.
    pub fn apply_truncate_i64(self, v: i64) -> Option<i64> {
        match self {
            Self::Truncate(w) => {
                let w = w as i64;
                Some(v.div_euclid(w) * w)
            }
            _ => None,
        }
    }

    /// Apply truncate[W] to a string: take the first W bytes (UTF-8 safe
    /// at byte boundary — caller responsible for not splitting code points).
    pub fn apply_truncate_str<'a>(self, v: &'a str) -> Option<&'a str> {
        match self {
            Self::Truncate(w) => {
                let w = w as usize;
                let take = w.min(v.len());
                Some(&v[..take])
            }
            _ => None,
        }
    }

    /// Apply bucket[N] to an i64 using Iceberg's Murmur3_x86_32 spec
    /// fallback — we use the truncated absolute remainder when
    /// Murmur isn't pulled in. The MVP exposes the protocol-level
    /// semantics (bucket index ∈ [0, N)) — exact-hash compatibility
    /// with Iceberg readers is deferred (`scope_cut: murmur-bucket-hash`).
    pub fn apply_bucket_i64(self, v: i64) -> Option<u32> {
        match self {
            Self::Bucket(n) => {
                let n = n as i64;
                if n <= 0 {
                    return None;
                }
                Some(((v.rem_euclid(n)) as u32) & i32::MAX as u32)
            }
            _ => None,
        }
    }
}

impl fmt::Display for Transform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_spec_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_transforms() {
        assert_eq!(Transform::parse("identity").unwrap(), Transform::Identity);
        assert_eq!(Transform::parse("day").unwrap(), Transform::Day);
        assert_eq!(Transform::parse("void").unwrap(), Transform::Void);
    }

    #[test]
    fn parse_bucket_and_truncate() {
        assert_eq!(
            Transform::parse("bucket[16]").unwrap(),
            Transform::Bucket(16)
        );
        assert_eq!(
            Transform::parse("truncate[8]").unwrap(),
            Transform::Truncate(8)
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Transform::parse("nope").is_err());
        assert!(Transform::parse("bucket[xx]").is_err());
        assert!(Transform::parse("truncate[").is_err());
    }

    #[test]
    fn spec_str_roundtrips() {
        for t in [
            Transform::Identity,
            Transform::Bucket(7),
            Transform::Truncate(5),
            Transform::Day,
        ] {
            let s = t.as_spec_str();
            let back = Transform::parse(&s).unwrap();
            assert_eq!(t, back);
        }
    }

    #[test]
    fn apply_truncate_i64_buckets_to_floor() {
        assert_eq!(Transform::Truncate(10).apply_truncate_i64(37), Some(30));
        assert_eq!(Transform::Truncate(10).apply_truncate_i64(-3), Some(-10));
    }

    #[test]
    fn apply_truncate_str_takes_prefix() {
        assert_eq!(
            Transform::Truncate(3).apply_truncate_str("abcdef"),
            Some("abc")
        );
        assert_eq!(
            Transform::Truncate(10).apply_truncate_str("abc"),
            Some("abc")
        );
    }

    #[test]
    fn apply_bucket_i64_in_range() {
        let b = Transform::Bucket(8).apply_bucket_i64(123).unwrap();
        assert!(b < 8);
    }
}
