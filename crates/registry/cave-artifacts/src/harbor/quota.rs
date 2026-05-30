// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Faithful line-port of goharbor/harbor (v2.10.0) per-project resource quota
// enforcement engine:
//   - src/pkg/quota/types/resource.go      — ResourceName / ResourceStorage
//   - src/pkg/quota/types/resourcelist.go  — ResourceList Add / Subtract / Equals
//   - src/pkg/quota/driver/driver.go       — CalculateUsage / enforcement
//   - src/pkg/quota/manager.go             — QuotaExceeded check on used+delta
//   - src/lib/errors/errors.go             — QuotaExceeded error payload
//
//! Per-project quota engine — a pure in-memory resource accounting + limit
//! checker. Harbor models a quota as two `ResourceList`s: a `hard` map of
//! per-resource limits and a `used` map of current consumption. A reservation
//! is admitted only when, for every resource, `used + requested <= hard`. A
//! hard value of `-1` is Harbor's sentinel meaning "unlimited" for that kind,
//! and disables enforcement for that resource (see `IsUnlimited` upstream).
//!
//! This module ports only the runtime arithmetic + admission decision. The
//! persistence of `quotas` rows lives in `src/harbor/store.rs` (the `quotas`
//! table) and is intentionally out of scope here.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Harbor's two quota-able resource kinds.
///
/// Upstream `ResourceName` is an open string ("count" / "storage"); we model
/// the two concrete kinds Harbor actually tracks as a closed enum so the
/// accounting is total and type-checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// Number of artifacts/tags (`ResourceCount`).
    Count,
    /// Bytes of blob storage (`ResourceStorage`).
    Storage,
}

impl ResourceKind {
    /// Stable string name matching Harbor's `ResourceName` values.
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Count => "count",
            ResourceKind::Storage => "storage",
        }
    }
}

/// Harbor sentinel: a hard limit of `-1` means "no limit" for that resource.
pub const UNLIMITED: i64 = -1;

/// Port of Harbor `types.ResourceList` — a map of resource kind to int64.
///
/// Missing keys read as zero (Harbor relies on Go's zero-value map semantics).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceList {
    inner: BTreeMap<ResourceKind, i64>,
}

impl ResourceList {
    pub fn new() -> Self {
        Self { inner: BTreeMap::new() }
    }

    /// Builder-style insert (used by callers + tests).
    pub fn with(mut self, kind: ResourceKind, value: i64) -> Self {
        self.inner.insert(kind, value);
        self
    }

    /// Set a value in place.
    pub fn set(&mut self, kind: ResourceKind, value: i64) {
        self.inner.insert(kind, value);
    }

    /// Read a resource value; absent keys read as zero (Go map zero-value).
    pub fn get(&self, kind: ResourceKind) -> i64 {
        self.inner.get(&kind).copied().unwrap_or(0)
    }

    /// Iterate the explicitly-present kinds.
    pub fn kinds(&self) -> impl Iterator<Item = ResourceKind> + '_ {
        self.inner.keys().copied()
    }

    /// Port of `resourcelist.Add` — element-wise sum over the union of keys.
    pub fn add(&self, other: &ResourceList) -> ResourceList {
        let mut out = self.clone();
        for kind in other.inner.keys().copied() {
            let v = out.get(kind) + other.get(kind);
            out.inner.insert(kind, v);
        }
        out
    }

    /// Port of `resourcelist.Subtract` — element-wise difference (may go
    /// negative; Harbor uses plain int64 subtraction here).
    pub fn subtract(&self, other: &ResourceList) -> ResourceList {
        let mut out = self.clone();
        for kind in other.inner.keys().copied() {
            let v = out.get(kind) - other.get(kind);
            out.inner.insert(kind, v);
        }
        out
    }
}

/// One offending resource in a quota-exceeded outcome (port of the per-resource
/// rows Harbor folds into its `QuotaExceeded` error message).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceededResource {
    pub kind: ResourceKind,
    pub hard: i64,
    pub used: i64,
    pub requested: i64,
}

/// Quota admission errors (port of `errors.QuotaExceeded`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuotaError {
    #[error("quota exceeded for {} resource(s)", resources.len())]
    Exceeded { resources: Vec<ExceededResource> },
}

/// A project's quota: the immutable `hard` limits plus mutable `used` counters.
///
/// Port of Harbor's quota driver state. `check_add` is the read-only admission
/// decision; `commit_add` admits-then-applies; `release` frees usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaUsage {
    hard: ResourceList,
    used: ResourceList,
}

impl QuotaUsage {
    pub fn new(hard: ResourceList, used: ResourceList) -> Self {
        Self { hard, used }
    }

    pub fn hard(&self) -> &ResourceList {
        &self.hard
    }

    pub fn used(&self) -> &ResourceList {
        &self.used
    }

    /// Read-only admission check: would applying `delta` keep every resource
    /// at or below its hard limit?
    ///
    /// Port of the `manager.go` enforcement loop: for each resource present in
    /// the `hard` map, skip if unlimited (`-1`), else fail when
    /// `used + requested > hard`. All offending resources are collected so the
    /// caller can report them together (Harbor folds them into one error).
    pub fn check_add(&self, delta: &ResourceList) -> Result<(), QuotaError> {
        let new_used = self.used.add(delta);
        let mut exceeded = Vec::new();
        for kind in self.hard.kinds() {
            let hard = self.hard.get(kind);
            if hard == UNLIMITED {
                continue;
            }
            if new_used.get(kind) > hard {
                exceeded.push(ExceededResource {
                    kind,
                    hard,
                    used: self.used.get(kind),
                    requested: delta.get(kind),
                });
            }
        }
        if exceeded.is_empty() {
            Ok(())
        } else {
            Err(QuotaError::Exceeded { resources: exceeded })
        }
    }

    /// Admit `delta` and, on success, fold it into `used`. On rejection the
    /// usage is left untouched (Harbor's manager only persists on a passing
    /// check).
    pub fn commit_add(&mut self, delta: &ResourceList) -> Result<(), QuotaError> {
        self.check_add(delta)?;
        self.used = self.used.add(delta);
        Ok(())
    }

    /// Free `delta` from `used`, clamping each resource at zero (port of
    /// Harbor's `free()` which never lets usage go negative).
    pub fn release(&mut self, delta: &ResourceList) {
        let mut next = self.used.subtract(delta);
        for kind in delta.kinds() {
            if next.get(kind) < 0 {
                next.set(kind, 0);
            }
        }
        self.used = next;
    }
}
