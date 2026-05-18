// SPDX-License-Identifier: AGPL-3.0-or-later
//! gRPC API surface complete — `RangeRequest` variations, `PutRequest`
//! `prev_kv` semantics, `DeleteRangeRequest` `prev_kv`, and a typed
//! `TxnRequest` builder over the existing flat shape in
//! [`crate::models`].
//!
//! Mirrors etcd v3.6.10
//!   `api/etcdserverpb/rpc.proto` (`RangeRequest.SortOrder`,
//!   `SortTarget`, `count_only`, `keys_only`)
//!   `api/etcdserverpb/rpc.proto` (`PutRequest.prev_kv`,
//!   `DeleteRangeRequest.prev_kv`)
//!   `api/etcdserverpb/rpc.proto` (`TxnRequest.compare/success/failure`).

use crate::error::{EtcdError, EtcdResult};
use crate::models::{
    Compare, CompareResult, CompareTarget, DeleteRangeRequest, KeyValue, PutRequest,
    RangeRequest, RangeResponse, RequestOp, TxnRequest,
};
use crate::store::KvStore;
use serde::{Deserialize, Serialize};

// ── SortOrder + SortTarget ───────────────────────────────────────────────

/// Direction.  Mirrors `RangeRequest.SortOrder`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    /// `NONE` — return in default (key-asc) order.
    None,
    Ascend,
    Descend,
}

/// What to sort on.  Mirrors `RangeRequest.SortTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortTarget {
    Key,
    Version,
    Create,
    Mod,
    Value,
}

/// Sort `kvs` in place per the (`order`, `target`) pair.  No-op when
/// `order == SortOrder::None` so callers can pass through the default
/// without paying the sort cost.
pub fn sort_kvs(kvs: &mut Vec<KeyValue>, order: SortOrder, target: SortTarget) {
    if matches!(order, SortOrder::None) {
        return;
    }
    let cmp: fn(&KeyValue, &KeyValue) -> std::cmp::Ordering = match target {
        SortTarget::Key => |a, b| a.key.cmp(&b.key),
        SortTarget::Version => |a, b| a.version.cmp(&b.version),
        SortTarget::Create => |a, b| a.create_revision.cmp(&b.create_revision),
        SortTarget::Mod => |a, b| a.mod_revision.cmp(&b.mod_revision),
        SortTarget::Value => |a, b| a.value.cmp(&b.value),
    };
    kvs.sort_by(cmp);
    if matches!(order, SortOrder::Descend) {
        kvs.reverse();
    }
}

/// Convenience wrapper around [`KvStore::range`] that also applies an
/// explicit `(order, target)` sort.  Used by route handlers that want
/// sort semantics without monkey-patching the core `RangeRequest` shape.
pub fn range_sorted(
    store: &KvStore,
    req: &RangeRequest,
    order: SortOrder,
    target: SortTarget,
) -> EtcdResult<RangeResponse> {
    let mut resp = store.range(req)?;
    sort_kvs(&mut resp.kvs, order, target);
    Ok(resp)
}

// ── Typed Txn builder ─────────────────────────────────────────────────────

/// Construction-side helper for `TxnRequest`.  Tests / callers can
/// compose a transaction without manually populating the
/// `compare/success/failure` vectors.
#[derive(Debug, Default)]
pub struct TxnBuilder {
    compares: Vec<Compare>,
    success: Vec<RequestOp>,
    failure: Vec<RequestOp>,
}

impl TxnBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compare against the value of `key`.
    pub fn when_value(mut self, key: &str, result: CompareResult, value: &str) -> Self {
        self.compares.push(Compare {
            key: key.into(),
            target: CompareTarget::Value,
            result,
            value: Some(value.into()),
            version: None,
            mod_revision: None,
        });
        self
    }

    /// Compare against the version of `key`.
    pub fn when_version(mut self, key: &str, result: CompareResult, v: u64) -> Self {
        self.compares.push(Compare {
            key: key.into(),
            target: CompareTarget::Version,
            result,
            value: None,
            version: Some(v),
            mod_revision: None,
        });
        self
    }

    /// Compare against the create_revision of `key`.
    pub fn when_create(mut self, key: &str, result: CompareResult, rev: u64) -> Self {
        self.compares.push(Compare {
            key: key.into(),
            target: CompareTarget::Create,
            result,
            value: None,
            version: None,
            mod_revision: Some(rev),
        });
        self
    }

    /// Compare against the mod_revision of `key`.
    pub fn when_mod(mut self, key: &str, result: CompareResult, rev: u64) -> Self {
        self.compares.push(Compare {
            key: key.into(),
            target: CompareTarget::Mod,
            result,
            value: None,
            version: None,
            mod_revision: Some(rev),
        });
        self
    }

    pub fn then_put(mut self, key: &str, value: &str) -> Self {
        self.success.push(RequestOp::Put(PutRequest {
            key: key.into(),
            value: value.into(),
            lease: None,
            prev_kv: false,
        }));
        self
    }

    pub fn then_delete(mut self, key: &str) -> Self {
        self.success.push(RequestOp::DeleteRange(DeleteRangeRequest {
            key: key.into(),
            range_end: None,
            prev_kv: false,
        }));
        self
    }

    pub fn else_put(mut self, key: &str, value: &str) -> Self {
        self.failure.push(RequestOp::Put(PutRequest {
            key: key.into(),
            value: value.into(),
            lease: None,
            prev_kv: false,
        }));
        self
    }

    pub fn else_delete(mut self, key: &str) -> Self {
        self.failure.push(RequestOp::DeleteRange(DeleteRangeRequest {
            key: key.into(),
            range_end: None,
            prev_kv: false,
        }));
        self
    }

    pub fn build(self) -> TxnRequest {
        TxnRequest {
            compare: self.compares,
            success: self.success,
            failure: self.failure,
        }
    }
}

/// Maximum number of operations etcd v3.6.10 allows per transaction.
/// Mirrors `etcdserver.maxTxnOps` (default 128).
pub const MAX_TXN_OPS: usize = 128;

/// Validate a `TxnRequest` against the v3.6.10 invariants:
///   * `compare`, `success`, and `failure` together stay within
///     `MAX_TXN_OPS` ops, and
///   * a transaction must not contain duplicate Put-after-Put on the
///     same key (etcd returns `ErrTxnDuplicateKey`).
pub fn validate_txn(req: &TxnRequest) -> EtcdResult<()> {
    let total = req.compare.len() + req.success.len() + req.failure.len();
    if total > MAX_TXN_OPS {
        return Err(EtcdError::Internal(format!(
            "etcdserver: too many operations in txn request ({total} > {MAX_TXN_OPS})"
        )));
    }
    fn collect_writes(ops: &[RequestOp]) -> Vec<&str> {
        let mut out = Vec::new();
        for op in ops {
            match op {
                RequestOp::Put(p) => out.push(p.key.as_str()),
                RequestOp::DeleteRange(d) => out.push(d.key.as_str()),
                _ => {}
            }
        }
        out
    }
    for branch in [&req.success, &req.failure] {
        let writes = collect_writes(branch);
        let mut sorted = writes.clone();
        sorted.sort();
        let len_before_dedup = sorted.len();
        sorted.dedup();
        if sorted.len() != len_before_dedup {
            return Err(EtcdError::Internal(
                "etcdserver: duplicate key given in txn request".into(),
            ));
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// gRPC API tests — feat/cave-etcd-deeper-003
// Each test embeds an upstream cite + a tenant_id constant.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CompareResult, KeyValue, PutRequest};

    fn dt(tenant_id: &str, suffix: &str) -> String {
        format!("/tenants/{}/{}", tenant_id, suffix)
    }

    fn pk_put(store: &KvStore, key: &str, value: &str) -> u64 {
        store.put(&PutRequest {
            key: key.into(),
            value: value.into(),
            lease: None,
            prev_kv: false,
        }).header.revision
    }

    fn make_kv(key: &str, value: &str, version: u64, create: u64, mod_rev: u64) -> KeyValue {
        KeyValue {
            key: key.as_bytes().to_vec(),
            value: value.as_bytes().to_vec(),
            create_revision: create,
            mod_revision: mod_rev,
            version,
            lease: None,
        }
    }

    // ── Sort ────────────────────────────────────────────────────────

    #[test]
    fn test_sort_kvs_none_is_noop() {
        // cite: etcd v3.6.10 api/etcdserverpb/rpc.proto SortOrder.NONE
        let _tenant_id = "grpc-001";
        let mut kvs = vec![
            make_kv("c", "1", 1, 1, 1),
            make_kv("a", "1", 1, 1, 1),
            make_kv("b", "1", 1, 1, 1),
        ];
        sort_kvs(&mut kvs, SortOrder::None, SortTarget::Key);
        assert_eq!(
            kvs.iter().map(|k| k.key_str()).collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn test_sort_kvs_ascend_by_key() {
        // cite: etcd v3.6.10 SortOrder.ASCEND on SortTarget.KEY
        let _tenant_id = "grpc-002";
        let mut kvs = vec![
            make_kv("c", "1", 1, 1, 1),
            make_kv("a", "1", 1, 1, 1),
            make_kv("b", "1", 1, 1, 1),
        ];
        sort_kvs(&mut kvs, SortOrder::Ascend, SortTarget::Key);
        assert_eq!(
            kvs.iter().map(|k| k.key_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn test_sort_kvs_descend_by_key() {
        // cite: etcd v3.6.10 SortOrder.DESCEND
        let _tenant_id = "grpc-003";
        let mut kvs = vec![
            make_kv("a", "1", 1, 1, 1),
            make_kv("b", "1", 1, 1, 1),
            make_kv("c", "1", 1, 1, 1),
        ];
        sort_kvs(&mut kvs, SortOrder::Descend, SortTarget::Key);
        assert_eq!(
            kvs.iter().map(|k| k.key_str()).collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );
    }

    #[test]
    fn test_sort_kvs_by_version() {
        // cite: etcd v3.6.10 SortTarget.VERSION
        let _tenant_id = "grpc-004";
        let mut kvs = vec![
            make_kv("a", "1", 5, 1, 1),
            make_kv("b", "1", 1, 1, 1),
            make_kv("c", "1", 3, 1, 1),
        ];
        sort_kvs(&mut kvs, SortOrder::Ascend, SortTarget::Version);
        assert_eq!(
            kvs.iter().map(|k| k.key_str()).collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );
    }

    #[test]
    fn test_sort_kvs_by_mod_revision() {
        // cite: etcd v3.6.10 SortTarget.MOD
        let _tenant_id = "grpc-005";
        let mut kvs = vec![
            make_kv("a", "1", 1, 1, 99),
            make_kv("b", "1", 1, 1, 50),
            make_kv("c", "1", 1, 1, 1),
        ];
        sort_kvs(&mut kvs, SortOrder::Descend, SortTarget::Mod);
        assert_eq!(
            kvs.iter().map(|k| k.key_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn test_range_sorted_applies_sort() {
        // cite: etcd v3.6.10 server/.../v3rpc/key.go Range with sort_order
        let tenant_id = "grpc-006";
        let store = KvStore::new();
        for k in ["c", "a", "b"] {
            pk_put(&store, &dt(tenant_id, k), "v");
        }
        let req = RangeRequest {
            key: dt(tenant_id, "").into(),
            range_end: Some(dt(tenant_id, "~").into()),
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        };
        let resp = range_sorted(&store, &req, SortOrder::Descend, SortTarget::Key).unwrap();
        let keys: Vec<String> = resp.kvs.iter().map(|k| k.key_str()).collect();
        // Descending: tenants/grpc-006/c, b, a
        assert!(keys[0].ends_with('c'));
        assert!(keys[2].ends_with('a'));
    }

    // ── TxnBuilder ──────────────────────────────────────────────────

    #[test]
    fn test_txn_builder_builds_compare_when_value() {
        // cite: etcd v3.6.10 api/etcdserverpb/rpc.proto Compare.Value
        let tenant_id = "grpc-007";
        let req = TxnBuilder::new()
            .when_value(&dt(tenant_id, "k"), CompareResult::Equal, "v")
            .then_put(&dt(tenant_id, "k"), "v2")
            .else_put(&dt(tenant_id, "k"), "fallback")
            .build();
        assert_eq!(req.compare.len(), 1);
        assert_eq!(req.success.len(), 1);
        assert_eq!(req.failure.len(), 1);
    }

    #[test]
    fn test_txn_builder_then_delete_else_delete() {
        // cite: etcd v3.6.10 RequestOp.DeleteRange
        let tenant_id = "grpc-008";
        let req = TxnBuilder::new()
            .when_version(&dt(tenant_id, "k"), CompareResult::Greater, 0)
            .then_delete(&dt(tenant_id, "k"))
            .else_delete(&dt(tenant_id, "alt"))
            .build();
        assert!(matches!(
            req.success[0],
            crate::models::RequestOp::DeleteRange(_)
        ));
        assert!(matches!(
            req.failure[0],
            crate::models::RequestOp::DeleteRange(_)
        ));
    }

    #[test]
    fn test_txn_builder_compare_create_and_mod() {
        // cite: etcd v3.6.10 CompareTarget.CREATE and CompareTarget.MOD
        let tenant_id = "grpc-009";
        let req = TxnBuilder::new()
            .when_create(&dt(tenant_id, "k"), CompareResult::Equal, 0)
            .when_mod(&dt(tenant_id, "k"), CompareResult::Less, 100)
            .then_put(&dt(tenant_id, "k"), "ok")
            .build();
        assert_eq!(req.compare.len(), 2);
    }

    #[test]
    fn test_validate_txn_rejects_too_many_ops() {
        // cite: etcd v3.6.10 etcdserver.maxTxnOps (default 128)
        let _tenant_id = "grpc-010";
        let mut req = TxnRequest {
            compare: vec![],
            success: vec![],
            failure: vec![],
        };
        for i in 0..(MAX_TXN_OPS + 1) {
            req.success.push(crate::models::RequestOp::Put(PutRequest {
                key: format!("k{i}"),
                value: "v".into(),
                lease: None,
                prev_kv: false,
            }));
        }
        let err = validate_txn(&req);
        assert!(matches!(err, Err(EtcdError::Internal(_))));
    }

    #[test]
    fn test_validate_txn_rejects_duplicate_key_in_branch() {
        // cite: etcd v3.6.10 ErrTxnDuplicateKey
        let tenant_id = "grpc-011";
        let req = TxnBuilder::new()
            .then_put(&dt(tenant_id, "k"), "v1")
            .then_put(&dt(tenant_id, "k"), "v2") // dup
            .build();
        let err = validate_txn(&req);
        assert!(matches!(err, Err(EtcdError::Internal(_))));
    }

    #[test]
    fn test_validate_txn_allows_distinct_keys_per_branch() {
        // cite: etcd v3.6.10 (distinct keys are fine across branches)
        let tenant_id = "grpc-012";
        let req = TxnBuilder::new()
            .then_put(&dt(tenant_id, "k1"), "v1")
            .else_put(&dt(tenant_id, "k1"), "v2") // ok: different branch
            .build();
        assert!(validate_txn(&req).is_ok());
    }
}
