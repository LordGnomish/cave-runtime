//! `/admin/cache` view — cache namespace + key browser with TTL editor.
//!
//! Mirrors Backstage's `redis-cache` plugin pane — keys are listed by
//! namespace, and a tenant-admin can extend the TTL of a single key.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, CacheEntry};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CacheViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("key {ns}/{key} not found")]
    KeyNotFound { ns: String, key: String },
    #[error("ttl_seconds must be between 1 and 86400")]
    InvalidTtl,
}

pub fn list_entries(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<CacheEntry>, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(scope(&state.cache_entries.read().unwrap(), &ctx.tenant, |r| &r.tenant)
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

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CacheViewError> {
    let entries = list_entries(state, ctx)?;
    let rows: Vec<Vec<String>> = entries
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
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Cache entries ({n})</h2>{tbl}</section>"#,
        n = entries.len(),
        tbl = table(&["namespace", "key", "ttl", "size"], &rows),
    );
    Ok(page_shell(
        &format!("cache · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/redis-cache/src/components/CacheKeysList.tsx",
    "CacheKeysList",
);

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
        assert!(e.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn entries_in_namespace_filters() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/NamespaceFilter.tsx",
            "NamespaceFilter",
            "acme"
        );
        let s = AdminState::seeded();
        let e = entries_in_namespace(&s, &ctx(&[Permission::CacheRead]), "session").unwrap();
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn set_ttl_updates_and_validates() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/TtlEditor.tsx",
            "TtlEditor",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::CacheRead, Permission::CacheWrite]);
        set_ttl(&s, &c, "session", "u-1", 7200).unwrap();
        let e = entries_in_namespace(&s, &c, "session").unwrap();
        assert_eq!(e.iter().find(|x| x.key == "u-1").unwrap().ttl_seconds, 7200);
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
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/DeleteKeyButton.tsx",
            "deleteKey",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::CacheRead, Permission::CacheWrite]);
        delete_key(&s, &c, "session", "u-1").unwrap();
        assert_eq!(entries_in_namespace(&s, &c, "session").unwrap().len(), 1);
        // Foreign key from evil tenant must look not-found from acme.
        assert!(matches!(
            delete_key(&s, &c, "session", "evil-1").unwrap_err(),
            CacheViewError::KeyNotFound { .. }
        ));
    }

    #[test]
    fn render_excludes_evil_entries() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/CachePage.tsx",
            "CachePage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::CacheRead])).unwrap();
        assert!(html.contains("u-1"));
        assert!(!html.contains("evil-1"));
    }
}
