// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD RED (2026-05-30): port of goharbor/harbor pkg/quota — the
// per-project resource-quota enforcement engine. Drives a NOT-YET-EXISTING
// module `cave_artifacts::harbor::quota`.
//
// Upstream references (Harbor v2.10.0):
//   - src/pkg/quota/types/resource.go   (ResourceList, Resource{Count,Storage})
//   - src/pkg/quota/types/resourcelist.go (Add / Subtract / Equals)
//   - src/pkg/quota/driver/driver.go + src/pkg/quota/manager.go (limit check)
//   - src/lib/errors/quota.go (QuotaExceeded payload listing offending res.)

use cave_artifacts::harbor::quota::{QuotaError, ResourceKind, ResourceList, QuotaUsage};

// ── ResourceList arithmetic ──────────────────────────────────────────────────

#[test]
fn resource_list_add_sums_per_kind() {
    let a = ResourceList::new()
        .with(ResourceKind::Count, 3)
        .with(ResourceKind::Storage, 100);
    let b = ResourceList::new()
        .with(ResourceKind::Count, 2)
        .with(ResourceKind::Storage, 50);
    let sum = a.add(&b);
    assert_eq!(sum.get(ResourceKind::Count), 5);
    assert_eq!(sum.get(ResourceKind::Storage), 150);
}

#[test]
fn resource_list_subtract_can_go_negative() {
    // Harbor's Subtract is plain int64 subtraction (a delta may be negative
    // when content is removed), so the result may be < 0.
    let a = ResourceList::new().with(ResourceKind::Storage, 50);
    let b = ResourceList::new().with(ResourceKind::Storage, 80);
    let diff = a.subtract(&b);
    assert_eq!(diff.get(ResourceKind::Storage), -30);
}

#[test]
fn resource_list_missing_kind_reads_zero() {
    let a = ResourceList::new().with(ResourceKind::Count, 7);
    assert_eq!(a.get(ResourceKind::Storage), 0);
}

// ── Limit enforcement ────────────────────────────────────────────────────────

#[test]
fn within_limit_is_allowed() {
    // hard = {count: 10, storage: 1000}, used = {count: 4, storage: 400}
    let hard = ResourceList::new()
        .with(ResourceKind::Count, 10)
        .with(ResourceKind::Storage, 1000);
    let usage = QuotaUsage::new(hard, ResourceList::new()
        .with(ResourceKind::Count, 4)
        .with(ResourceKind::Storage, 400));
    // adding 3 count + 500 storage stays under hard.
    let delta = ResourceList::new()
        .with(ResourceKind::Count, 3)
        .with(ResourceKind::Storage, 500);
    assert!(usage.check_add(&delta).is_ok());
}

#[test]
fn exceeding_storage_is_rejected_and_names_resource() {
    let hard = ResourceList::new()
        .with(ResourceKind::Count, 10)
        .with(ResourceKind::Storage, 1000);
    let usage = QuotaUsage::new(hard, ResourceList::new()
        .with(ResourceKind::Count, 4)
        .with(ResourceKind::Storage, 900));
    // +200 storage -> 1100 > 1000.
    let delta = ResourceList::new().with(ResourceKind::Storage, 200);
    match usage.check_add(&delta) {
        Err(QuotaError::Exceeded { resources }) => {
            assert!(resources.iter().any(|r| r.kind == ResourceKind::Storage));
            // the offending entry reports requested/used/hard.
            let r = resources.iter().find(|r| r.kind == ResourceKind::Storage).unwrap();
            assert_eq!(r.hard, 1000);
            assert_eq!(r.used, 900);
            assert_eq!(r.requested, 200);
        }
        other => panic!("expected QuotaExceeded, got {other:?}"),
    }
}

#[test]
fn negative_one_hard_means_unlimited() {
    // Harbor sentinel: a hard limit of -1 disables enforcement for that kind.
    let hard = ResourceList::new()
        .with(ResourceKind::Count, -1)
        .with(ResourceKind::Storage, -1);
    let usage = QuotaUsage::new(hard, ResourceList::new()
        .with(ResourceKind::Count, 999_999)
        .with(ResourceKind::Storage, 1 << 50));
    let delta = ResourceList::new()
        .with(ResourceKind::Count, 1_000_000)
        .with(ResourceKind::Storage, 1 << 40);
    assert!(usage.check_add(&delta).is_ok());
}

#[test]
fn check_add_then_commit_updates_used() {
    let hard = ResourceList::new().with(ResourceKind::Count, 5);
    let mut usage = QuotaUsage::new(hard, ResourceList::new().with(ResourceKind::Count, 1));
    let delta = ResourceList::new().with(ResourceKind::Count, 2);
    usage.commit_add(&delta).expect("within limit");
    assert_eq!(usage.used().get(ResourceKind::Count), 3);
    // committing past the hard limit fails and leaves used unchanged.
    let big = ResourceList::new().with(ResourceKind::Count, 10);
    assert!(usage.commit_add(&big).is_err());
    assert_eq!(usage.used().get(ResourceKind::Count), 3);
}

#[test]
fn release_subtracts_used_clamping_at_zero() {
    // Harbor's free() never lets used go below zero.
    let hard = ResourceList::new().with(ResourceKind::Storage, 1000);
    let mut usage = QuotaUsage::new(hard, ResourceList::new().with(ResourceKind::Storage, 300));
    usage.release(&ResourceList::new().with(ResourceKind::Storage, 500));
    assert_eq!(usage.used().get(ResourceKind::Storage), 0);
}
