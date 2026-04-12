#[cfg(test)]
mod wal_tests {
    use tempfile::TempDir;
    use crate::engine::wal::{WalEntry, WalFile, WalOp};

    #[test]
    fn write_and_replay() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");
        let mut wal = WalFile::open(&path, false).unwrap();

        let e1 = WalEntry::new(1, WalOp::Put { key: b"foo".to_vec(), value: b"bar".to_vec(), lease_id: 0 });
        let e2 = WalEntry::new(2, WalOp::Delete { key: b"foo".to_vec() });
        wal.append(&e1).unwrap();
        wal.append(&e2).unwrap();
        drop(wal);

        let entries = WalFile::replay(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].revision, 1);
        assert_eq!(entries[1].revision, 2);
        assert!(matches!(entries[0].op, WalOp::Put { .. }));
        assert!(matches!(entries[1].op, WalOp::Delete { .. }));
    }

    #[test]
    fn replay_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");
        let entries = WalFile::replay(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn rotate() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wal.log");
        let mut wal = WalFile::open(&path, false).unwrap();
        wal.append(&WalEntry::new(1, WalOp::Put { key: b"k".to_vec(), value: b"v".to_vec(), lease_id: 0 })).unwrap();

        let archive = dir.path().join("wal.log.1");
        wal.rotate(&archive).unwrap();

        let new_entries = WalFile::replay(&path).unwrap();
        assert!(new_entries.is_empty());
        let old_entries = WalFile::replay(&archive).unwrap();
        assert_eq!(old_entries.len(), 1);
    }
}

#[cfg(test)]
mod mvcc_tests {
    use crate::engine::mvcc::{Compare, CompareResult, CompareTarget, MvccStore, TxnOp};

    fn store() -> MvccStore {
        MvccStore::new()
    }

    #[test]
    fn put_and_get() {
        let mut s = store();
        s.put(b"foo".to_vec(), b"bar".to_vec(), 0, false);
        let kv = s.get(b"foo", None).unwrap();
        assert_eq!(kv.value, b"bar");
        assert_eq!(kv.version, 1);
        assert_eq!(kv.create_revision, 1);
        assert_eq!(kv.mod_revision, 1);
    }

    #[test]
    fn put_updates_version() {
        let mut s = store();
        s.put(b"k".to_vec(), b"v1".to_vec(), 0, false);
        s.put(b"k".to_vec(), b"v2".to_vec(), 0, false);
        let kv = s.get(b"k", None).unwrap();
        assert_eq!(kv.value, b"v2");
        assert_eq!(kv.version, 2);
        assert_eq!(kv.create_revision, 1);
        assert_eq!(kv.mod_revision, 2);
    }

    #[test]
    fn delete_returns_tombstone() {
        let mut s = store();
        s.put(b"k".to_vec(), b"v".to_vec(), 0, false);
        let (_, prev) = s.delete(b"k", true);
        assert!(prev.is_some());
        assert_eq!(prev.unwrap().value, b"v");
        assert!(s.get(b"k", None).is_none());
    }

    #[test]
    fn range_query() {
        let mut s = store();
        s.put(b"a".to_vec(), b"1".to_vec(), 0, false);
        s.put(b"b".to_vec(), b"2".to_vec(), 0, false);
        s.put(b"c".to_vec(), b"3".to_vec(), 0, false);

        // Range [a, c)
        let (kvs, count) = s.range(b"a", b"c", None, 0, false, false);
        assert_eq!(count, 2);
        assert_eq!(kvs.len(), 2);
        assert_eq!(kvs[0].key, b"a");
        assert_eq!(kvs[1].key, b"b");
    }

    #[test]
    fn range_all_keys() {
        let mut s = store();
        s.put(b"a".to_vec(), b"1".to_vec(), 0, false);
        s.put(b"z".to_vec(), b"2".to_vec(), 0, false);
        // range_end = \0 means "all keys >= key"
        let (kvs, _) = s.range(b"a", &[0u8], None, 0, false, false);
        assert_eq!(kvs.len(), 2);
    }

    #[test]
    fn range_with_limit() {
        let mut s = store();
        for c in b'a'..=b'e' {
            s.put(vec![c], vec![c], 0, false);
        }
        let (kvs, count) = s.range(b"a", &[0u8], None, 2, false, false);
        assert_eq!(kvs.len(), 2);
        assert_eq!(count, 5);
    }

    #[test]
    fn historical_revision() {
        let mut s = store();
        s.put(b"k".to_vec(), b"v1".to_vec(), 0, false); // rev 1
        s.put(b"k".to_vec(), b"v2".to_vec(), 0, false); // rev 2

        let old = s.get(b"k", Some(1)).unwrap();
        assert_eq!(old.value, b"v1");

        let new = s.get(b"k", Some(2)).unwrap();
        assert_eq!(new.value, b"v2");
    }

    #[test]
    fn compact() {
        let mut s = store();
        s.put(b"k".to_vec(), b"v1".to_vec(), 0, false); // rev 1
        s.put(b"k".to_vec(), b"v2".to_vec(), 0, false); // rev 2
        s.compact(1).unwrap();
        assert_eq!(s.compacted_revision(), 1);
        // Old revision no longer accessible
        assert!(s.compact(1).is_err()); // can't compact already-compacted
        // Current is still accessible
        assert!(s.get(b"k", None).is_some());
    }

    #[test]
    fn txn_success_path() {
        let mut s = store();
        s.put(b"k".to_vec(), b"v1".to_vec(), 0, false);

        let cmps = vec![Compare {
            key: b"k".to_vec(),
            range_end: vec![],
            result: CompareResult::Equal,
            target: CompareTarget::Value(b"v1".to_vec()),
        }];
        let success = vec![TxnOp::Put {
            key: b"k".to_vec(),
            value: b"v2".to_vec(),
            lease_id: 0,
            prev_kv: false,
        }];
        let (succeeded, _) = s.txn(cmps, success, vec![]);
        assert!(succeeded);
        assert_eq!(s.get(b"k", None).unwrap().value, b"v2");
    }

    #[test]
    fn txn_failure_path() {
        let mut s = store();
        s.put(b"k".to_vec(), b"v1".to_vec(), 0, false);

        let cmps = vec![Compare {
            key: b"k".to_vec(),
            range_end: vec![],
            result: CompareResult::Equal,
            target: CompareTarget::Value(b"wrong".to_vec()),
        }];
        let failure = vec![TxnOp::Put {
            key: b"k".to_vec(),
            value: b"fallback".to_vec(),
            lease_id: 0,
            prev_kv: false,
        }];
        let (succeeded, _) = s.txn(cmps, vec![], failure);
        assert!(!succeeded);
        assert_eq!(s.get(b"k", None).unwrap().value, b"fallback");
    }

    #[test]
    fn lease_grant_revoke() {
        let mut s = store();
        let id = s.lease_grant(0, 60);
        s.put(b"k".to_vec(), b"v".to_vec(), id, false);
        let keys = s.lease_revoke(id).unwrap();
        assert!(keys.contains(&b"k".to_vec()));
        assert!(s.get(b"k", None).is_none());
    }

    #[test]
    fn lease_ttl() {
        let mut s = store();
        let id = s.lease_grant(0, 30);
        let (granted, remaining, _) = s.lease_ttl(id).unwrap();
        assert_eq!(granted, 30);
        assert!(remaining <= 30 && remaining >= 29);
    }

    #[test]
    fn lease_not_found() {
        let s = store();
        assert!(s.lease_ttl(9999).is_err());
    }

    #[test]
    fn watch_create_and_cancel() {
        let mut s = store();
        let wid = s.watch_create(b"k".to_vec(), vec![], 0, false, false, false, false, 0);
        assert!(wid > 0);
        s.watch_cancel(wid);
    }

    #[test]
    fn watch_receives_events() {
        let mut s = store();
        let mut rx = s.subscribe();
        s.watch_create(b"foo".to_vec(), vec![], 0, false, false, false, false, 1);
        s.put(b"foo".to_vec(), b"bar".to_vec(), 0, false);

        let event = rx.try_recv().unwrap();
        assert_eq!(event.watch_id, 1);
        assert_eq!(event.kv.key, b"foo");
    }

    #[test]
    fn delete_range() {
        let mut s = store();
        s.put(b"a/1".to_vec(), b"v".to_vec(), 0, false);
        s.put(b"a/2".to_vec(), b"v".to_vec(), 0, false);
        s.put(b"b/1".to_vec(), b"v".to_vec(), 0, false);

        let (_, deleted) = s.delete_range(b"a/", b"b/", true);
        assert_eq!(deleted.len(), 2);
        assert!(s.get(b"b/1", None).is_some());
    }
}

#[cfg(test)]
mod engine_integration {
    use tempfile::TempDir;
    use crate::engine::StorageEngine;

    #[test]
    fn wal_replay_restores_state() {
        let dir = TempDir::new().unwrap();
        {
            let engine = StorageEngine::open(dir.path(), false).unwrap();
            engine.put(b"hello".to_vec(), b"world".to_vec(), 0, false).unwrap();
            engine.put(b"foo".to_vec(), b"bar".to_vec(), 0, false).unwrap();
        }
        // Reopen and check state is restored
        let engine2 = StorageEngine::open(dir.path(), false).unwrap();
        let kv = engine2.mvcc.read().get(b"hello", None).unwrap();
        assert_eq!(kv.value, b"world");
        let kv2 = engine2.mvcc.read().get(b"foo", None).unwrap();
        assert_eq!(kv2.value, b"bar");
    }

    #[test]
    fn lease_wal_roundtrip() {
        let dir = TempDir::new().unwrap();
        let lease_id;
        {
            let engine = StorageEngine::open(dir.path(), false).unwrap();
            lease_id = engine.lease_grant(0, 60).unwrap();
            engine.put(b"leased".to_vec(), b"val".to_vec(), lease_id, false).unwrap();
        }
        let engine2 = StorageEngine::open(dir.path(), false).unwrap();
        let kv = engine2.mvcc.read().get(b"leased", None).unwrap();
        assert_eq!(kv.value, b"val");
        assert_eq!(kv.lease, lease_id);
    }
}
