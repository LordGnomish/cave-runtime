//! `/admin/search` — OpenSearch Dashboards parity. Index list with
//! doc-count + size summary, status-grouped chips.
//!
//! Upstream UI: <https://opensearch.org/docs/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, SearchIndex};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SearchViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<SearchIndex>, SearchViewError> {
    ctx.authorise(Permission::SearchRead)?;
    let mut rows: Vec<SearchIndex> = scope(&state.search_indexes.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| b.doc_count.cmp(&a.doc_count).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn total_doc_count(rows: &[SearchIndex]) -> u64 {
    rows.iter().map(|r| r.doc_count).sum()
}

pub fn total_size_bytes(rows: &[SearchIndex]) -> u64 {
    rows.iter().map(|r| r.size_bytes).sum()
}

pub fn group_by_status(rows: &[SearchIndex]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows { *acc.entry(r.status.to_string()).or_insert(0) += 1; }
    acc.into_iter().collect()
}

pub fn detail(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<Option<SearchIndex>, SearchViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.name == name))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SearchViewError> {
    let rows = list_records(state, ctx)?;
    let docs = total_doc_count(&rows);
    let size = total_size_bytes(&rows);
    let groups = group_by_status(&rows);
    let chips: String = groups.iter().map(|(s, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
        s = escape(s), n = n)).collect();
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.name), r.doc_count.to_string(), r.size_bytes.to_string(), r.status.into(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">OpenSearch Dashboards (cave-search). Upstream: <a class="text-blue-700 underline" href="https://opensearch.org/docs/">opensearch.org/docs</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> indexes</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{docs}</strong> docs total</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{size}</strong> bytes total</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Indexes ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        docs = docs,
        size = size,
        chips = chips,
        tbl = table(&["name", "doc_count", "size_bytes", "status"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/search", &format!("search · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/search/src/components/IndexesList.tsx", "IndexesList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_sorted_by_docs_desc() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SearchRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) { assert!(w[0].doc_count >= w[1].doc_count); }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn total_doc_count_sums() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SearchRead])).unwrap();
        let expected: u64 = r.iter().map(|x| x.doc_count).sum();
        assert_eq!(total_doc_count(&r), expected);
    }

    #[test]
    fn total_size_bytes_sums() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SearchRead])).unwrap();
        let expected: u64 = r.iter().map(|x| x.size_bytes).sum();
        assert_eq!(total_size_bytes(&r), expected);
    }

    #[test]
    fn group_by_status_counts() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SearchRead])).unwrap();
        let g = group_by_status(&r);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), r.len());
    }

    #[test]
    fn detail_returns_index_by_name() {
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::SearchRead])).unwrap();
        if let Some(f) = r.first() {
            assert!(detail(&s, &ctx(&[Permission::SearchRead]), &f.name).unwrap().is_some());
        }
        assert!(detail(&s, &ctx(&[Permission::SearchRead]), "no-such").unwrap().is_none());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SearchRead])).unwrap();
        assert!(html.contains("docs-index"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SearchRead])).unwrap();
        assert!(!html.contains("evil-index"));
    }

    #[test]
    fn render_includes_totals_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SearchRead])).unwrap();
        assert!(html.contains("docs total"));
        assert!(html.contains("opensearch.org"));
    }
}
