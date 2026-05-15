//! `/admin/store` — MinIO Console parity. Bucket browser with
//! backend grouping + size totals.
//!
//! Upstream UI: <https://min.io/docs/minio/linux/operations/minio-console.html>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, StoreBucket};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<StoreBucket>, StoreViewError> {
    ctx.authorise(Permission::StoreRead)?;
    let mut rows: Vec<StoreBucket> = scope(&state.store_buckets.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn group_by_backend(rows: &[StoreBucket]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows { *acc.entry(r.backend.clone()).or_insert(0) += 1; }
    acc.into_iter().collect()
}

pub fn total_objects(rows: &[StoreBucket]) -> u64 { rows.iter().map(|r| r.object_count).sum() }
pub fn total_size_bytes(rows: &[StoreBucket]) -> u64 { rows.iter().map(|r| r.size_bytes).sum() }

pub fn detail(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<Option<StoreBucket>, StoreViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.name == name))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StoreViewError> {
    let rows = list_records(state, ctx)?;
    let objs = total_objects(&rows);
    let size = total_size_bytes(&rows);
    let backends = group_by_backend(&rows);
    let chips: String = backends.iter().map(|(b, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{b} <strong>×{n}</strong></span>"#,
        b = escape(b), n = n)).collect();
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.name), escape(&r.backend), r.object_count.to_string(), r.size_bytes.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">MinIO Console (cave-store). Upstream: <a class="text-blue-700 underline" href="https://min.io/docs/minio/linux/operations/minio-console.html">min.io/docs</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> buckets</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{objs}</strong> objects</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{size}</strong> bytes total</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Buckets ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        objs = objs,
        size = size,
        chips = chips,
        tbl = table(&["name", "backend", "objects", "size_bytes"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/store", &format!("store · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/store/src/components/BucketsList.tsx", "BucketsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_sorted_by_size_desc() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) { assert!(w[0].size_bytes >= w[1].size_bytes); }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_backend_counts() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        let g = group_by_backend(&r);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), r.len());
    }

    #[test]
    fn totals_sum_correctly() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        let eo: u64 = r.iter().map(|x| x.object_count).sum();
        let es: u64 = r.iter().map(|x| x.size_bytes).sum();
        assert_eq!(total_objects(&r), eo);
        assert_eq!(total_size_bytes(&r), es);
    }

    #[test]
    fn detail_returns_bucket_by_name() {
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::StoreRead])).unwrap();
        if let Some(f) = r.first() {
            assert!(detail(&s, &ctx(&[Permission::StoreRead]), &f.name).unwrap().is_some());
        }
        assert!(detail(&s, &ctx(&[Permission::StoreRead]), "no-such").unwrap().is_none());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        assert!(html.contains("prod-images"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        assert!(!html.contains("evil-bucket"));
    }

    #[test]
    fn render_includes_totals_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StoreRead])).unwrap();
        assert!(html.contains("bytes total"));
        assert!(html.contains("min.io"));
    }
}
