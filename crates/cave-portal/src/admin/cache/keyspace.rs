// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Keyspace tab — Redis/Valkey-style per-namespace key browser with
//! TTL editor and delete mutator.

use super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{scope, AdminState, CacheEntry};

pub fn list_entries(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<CacheEntry>, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(scope(&state.cache_entries.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect())
}

pub fn entries_in_namespace(
    state: &AdminState,
    ctx: &RequestCtx,
    ns: &str,
) -> Result<Vec<CacheEntry>, CacheViewError> {
    let all = list_entries(state, ctx)?;
    Ok(all.into_iter().filter(|e| e.namespace == ns).collect())
}

pub fn set_ttl(
    state: &AdminState,
    ctx: &RequestCtx,
    ns: &str,
    key: &str,
    ttl: u64,
) -> Result<(), CacheViewError> {
    ctx.authorise(Permission::CacheWrite)?;
    if !(1..=86_400).contains(&ttl) {
        return Err(CacheViewError::InvalidTtl);
    }
    let mut entries = state.cache_entries.write().unwrap();
    let target = entries
        .iter_mut()
        .find(|e| e.tenant == ctx.tenant && e.namespace == ns && e.key == key)
        .ok_or_else(|| CacheViewError::KeyNotFound {
            ns: ns.into(),
            key: key.into(),
        })?;
    target.ttl_seconds = ttl;
    Ok(())
}

pub fn delete_key(
    state: &AdminState,
    ctx: &RequestCtx,
    ns: &str,
    key: &str,
) -> Result<(), CacheViewError> {
    ctx.authorise(Permission::CacheWrite)?;
    let mut entries = state.cache_entries.write().unwrap();
    let before = entries.len();
    entries.retain(|e| !(e.tenant == ctx.tenant && e.namespace == ns && e.key == key));
    if entries.len() == before {
        return Err(CacheViewError::KeyNotFound {
            ns: ns.into(),
            key: key.into(),
        });
    }
    Ok(())
}

/// Per-database (db0..db15) summary mirroring Redis `INFO keyspace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbStat {
    pub db: u8,
    pub keys: u32,
    pub size_bytes: u64,
}

/// Bin namespaces into 16 logical databases by namespace-hash. Mirrors
/// the Redis convention that databases hold disjoint keysets.
pub fn db_stats(entries: &[CacheEntry]) -> Vec<DbStat> {
    let mut acc: std::collections::BTreeMap<u8, (u32, u64)> = std::collections::BTreeMap::new();
    for e in entries {
        let db = (hash_ns(&e.namespace) % 16) as u8;
        let slot = acc.entry(db).or_insert((0, 0));
        slot.0 += 1;
        slot.1 += e.size_bytes;
    }
    acc.into_iter()
        .map(|(db, (keys, size_bytes))| DbStat {
            db,
            keys,
            size_bytes,
        })
        .collect()
}

fn hash_ns(ns: &str) -> u32 {
    let mut h: u32 = 0;
    for b in ns.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    h
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let entries = list_entries(state, ctx)?;
    let stats = db_stats(&entries);
    let key_rows: Vec<Vec<String>> = entries
        .iter()
        .map(|e| {
            vec![
                e.namespace.clone(),
                e.key.clone(),
                format!("{}s", e.ttl_seconds),
                format!("{}B", e.size_bytes),
            ]
        })
        .collect();
    let stat_rows: Vec<Vec<String>> = stats
        .iter()
        .map(|s| {
            vec![
                format!("db{}", s.db),
                s.keys.to_string(),
                format!("{}B", s.size_bytes),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cache-keyspace" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Keyspace (INFO keyspace)</h2>
  {db_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">Entries ({n})</h3>
  {key_tbl}
</section>"#,
        n = entries.len(),
        db_tbl = table(&["db", "keys", "size"], &stat_rows),
        key_tbl = table(&["namespace", "key", "ttl", "size"], &key_rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_entries_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/CacheKeysList.tsx",
            "CacheKeysList",
            "acme"
        );
        let s = AdminState::seeded();
        let e = list_entries(&s, &ctx(&[Permission::CacheRead])).unwrap();
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn entries_in_namespace_filters() {
        let s = AdminState::seeded();
        let e = entries_in_namespace(&s, &ctx(&[Permission::CacheRead]), "session").unwrap();
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn set_ttl_updates_and_validates() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::CacheRead, Permission::CacheWrite]);
        set_ttl(&s, &c, "session", "u-1", 7200).unwrap();
        assert_eq!(
            entries_in_namespace(&s, &c, "session")
                .unwrap()
                .iter()
                .find(|x| x.key == "u-1")
                .unwrap()
                .ttl_seconds,
            7200
        );
        assert!(matches!(
            set_ttl(&s, &c, "session", "u-1", 0).unwrap_err(),
            CacheViewError::InvalidTtl
        ));
        assert!(matches!(
            set_ttl(&s, &c, "session", "u-1", 999_999).unwrap_err(),
            CacheViewError::InvalidTtl
        ));
    }

    #[test]
    fn delete_key_removes_and_refuses_cross_tenant() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::CacheRead, Permission::CacheWrite]);
        delete_key(&s, &c, "session", "u-1").unwrap();
        assert_eq!(entries_in_namespace(&s, &c, "session").unwrap().len(), 1);
        assert!(matches!(
            delete_key(&s, &c, "session", "evil-1").unwrap_err(),
            CacheViewError::KeyNotFound { .. }
        ));
    }

    #[test]
    fn db_stats_partitions_into_16_databases() {
        let entries = list_entries(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        let stats = db_stats(&entries);
        for s in &stats {
            assert!(s.db < 16);
        }
    }

    #[test]
    fn render_section_includes_db_stats_and_entries() {
        let html = render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(html.contains("Keyspace"));
        assert!(html.contains("INFO keyspace"));
        assert!(html.contains("u-1"));
    }

    #[test]
    fn list_entries_requires_permission() {
        assert!(list_entries(&AdminState::seeded(), &ctx(&[])).is_err());
    }
}
