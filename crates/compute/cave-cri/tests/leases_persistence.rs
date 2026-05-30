// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lease persistence — the boltdb scope cut closed.
//!
//! Containerd persists leases in its metadata bolt store under the
//! `leases` bucket (`core/metadata/leases.go`): a lease id maps to its
//! created-at, labels, and resource set, surviving daemon restart so
//! the GC interlock is durable. cave-cri's single-process analog is a
//! JSON-backed lease table under `<root>/leases.json`. These tests pin
//! the durability contract: every mutation that the in-memory table
//! accepts must still be visible after the manager is dropped and
//! re-opened from the same root.

use cave_cri::leases::{LeaseManager, Resource};
use std::collections::HashMap;
use tempfile::TempDir;

#[test]
fn created_lease_survives_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let m = LeaseManager::open(dir.path()).unwrap();
        m.create("lease-a", None, HashMap::new()).unwrap();
    }
    let m2 = LeaseManager::open(dir.path()).unwrap();
    assert!(m2.get("lease-a").is_ok(), "lease must survive reopen");
}

#[test]
fn added_resources_survive_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let m = LeaseManager::open(dir.path()).unwrap();
        m.create("L", None, HashMap::new()).unwrap();
        m.add_resource("L", Resource::snapshot("snap-1")).unwrap();
        m.add_resource("L", Resource::snapshot("snap-2")).unwrap();
    }
    let m2 = LeaseManager::open(dir.path()).unwrap();
    let lease = m2.get("L").unwrap();
    assert_eq!(lease.resources.len(), 2);
    assert!(lease.resources.contains(&Resource::snapshot("snap-1")));
}

#[test]
fn removed_resource_does_not_reappear_after_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let m = LeaseManager::open(dir.path()).unwrap();
        m.create("L", None, HashMap::new()).unwrap();
        let r = Resource::snapshot("gone");
        m.add_resource("L", r.clone()).unwrap();
        m.remove_resource("L", &r).unwrap();
    }
    let m2 = LeaseManager::open(dir.path()).unwrap();
    assert_eq!(m2.get("L").unwrap().resources.len(), 0);
}

#[test]
fn deleted_lease_stays_deleted_after_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let m = LeaseManager::open(dir.path()).unwrap();
        m.create("temp", None, HashMap::new()).unwrap();
        m.delete("temp").unwrap();
    }
    let m2 = LeaseManager::open(dir.path()).unwrap();
    assert!(m2.get("temp").is_err());
}

#[test]
fn labels_and_ttl_survive_reopen() {
    let dir = TempDir::new().unwrap();
    let mut labels = HashMap::new();
    labels.insert("kind".to_string(), "pull".to_string());
    {
        let m = LeaseManager::open(dir.path()).unwrap();
        m.create("kept", Some(3600), labels.clone()).unwrap();
    }
    let m2 = LeaseManager::open(dir.path()).unwrap();
    let lease = m2.get("kept").unwrap();
    assert_eq!(lease.labels, labels);
    assert_eq!(lease.ttl_seconds, Some(3600));
}

#[test]
fn reap_expired_removal_is_durable() {
    let dir = TempDir::new().unwrap();
    {
        let m = LeaseManager::open(dir.path()).unwrap();
        m.create("short", Some(0), HashMap::new()).unwrap();
        m.create("forever", None, HashMap::new()).unwrap();
        let reaped = m.reap_expired();
        assert_eq!(reaped, vec!["short".to_string()]);
    }
    let m2 = LeaseManager::open(dir.path()).unwrap();
    assert!(m2.get("short").is_err(), "reaped lease must not return on reopen");
    assert!(m2.get("forever").is_ok());
}

#[test]
fn many_leases_survive_reopen() {
    let dir = TempDir::new().unwrap();
    {
        let m = LeaseManager::open(dir.path()).unwrap();
        for i in 0..10 {
            m.create(format!("l-{i}"), None, HashMap::new()).unwrap();
        }
    }
    let m2 = LeaseManager::open(dir.path()).unwrap();
    assert_eq!(m2.list().len(), 10);
}

#[test]
fn open_with_store_rehydrates_in_use_interlock() {
    use cave_cri::content::digest::{Digest, DigestAlgorithm};
    use cave_cri::content::store::{ContentStore, LocalStore, StoreError};
    use std::io::Write;
    use std::sync::Arc;

    let dir = TempDir::new().unwrap();
    let blob = b"protected-across-restart";
    let digest = Digest::compute(DigestAlgorithm::Sha256, blob);

    // First boot: store the blob and hold it under a persisted lease.
    {
        let store = Arc::new(LocalStore::open(dir.path()).unwrap());
        let mut w = store.writer("r-x".to_string(), digest.clone()).unwrap();
        w.write_all(blob).unwrap();
        w.commit().unwrap();
        let m = LeaseManager::open_with_store(dir.path(), store.clone()).unwrap();
        m.create("hold", None, HashMap::new()).unwrap();
        m.add_resource("hold", Resource::content(&digest)).unwrap();
    }

    // Second boot: re-open the store + leases. The persisted lease must
    // re-establish the in-use interlock so the blob still can't be reaped.
    let store2 = Arc::new(LocalStore::open(dir.path()).unwrap());
    let m2 = LeaseManager::open_with_store(dir.path(), store2.clone()).unwrap();
    assert!(m2.get("hold").is_ok());
    let err = store2.delete(&digest).unwrap_err();
    assert!(matches!(err, StoreError::InUse(_)), "rehydrated lease must block delete");

    // Drop the lease → blob becomes reapable.
    m2.delete("hold").unwrap();
    store2.delete(&digest).unwrap();
}
