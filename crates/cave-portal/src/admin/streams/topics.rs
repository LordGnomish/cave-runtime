//! `/admin/streams/topics` — Kafka admin "Topics" tab. Per-topic
//! configuration view with partition count, replication factor,
//! retention, and compaction policy. Read-only here; mutations
//! (create / delete / alter) flow through `cavectl streams`.
//!
//! Upstream: <https://kafka.apache.org/documentation/#basic_ops_topics>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, StreamsTopic};
use super::StreamsViewError;

pub fn list_topics_sorted(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsTopic>, StreamsViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    let mut rows: Vec<StreamsTopic> =
        scope(&state.streams_topics.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub fn total_partitions(rows: &[StreamsTopic]) -> u64 {
    rows.iter().map(|t| t.partitions as u64).sum()
}

pub fn topics_by_compaction(
    rows: &[StreamsTopic],
) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.compaction).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_topics_sorted(state, ctx)?;
    let total_part = total_partitions(&rows);
    let by_compact = topics_by_compaction(&rows);
    let chips: String = by_compact
        .iter()
        .map(|(k, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-purple-100 text-sm">{k} <strong>×{n}</strong></span>"#,
                k = k,
                n = n,
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                r.partitions.to_string(),
                r.replication_factor.to_string(),
                r.retention_ms.to_string(),
                r.compaction.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Topics ({n}) · total partitions {tp}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Topic registry. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#basic_ops_topics">Kafka topics</a>.
  </p>
  {tbl}
</section>"#,
        chips = chips,
        n = rows.len(),
        tp = total_part,
        tbl = table(
            &["topic", "partitions", "rf", "retention_ms", "compaction"],
            &table_rows
        ),
    );
    Ok(page_shell(
        &format!("streams/topics · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_returns_seeded_topics_for_tenant() {
        let rows = list_topics_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(rows.iter().all(|t| t.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_sorted_alphabetically() {
        let rows = list_topics_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].name <= w[1].name);
        }
    }

    #[test]
    fn total_partitions_sums_correctly() {
        let rows = list_topics_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let total = total_partitions(&rows);
        let expected: u64 = rows.iter().map(|t| t.partitions as u64).sum();
        assert_eq!(total, expected);
    }

    #[test]
    fn topics_by_compaction_groups_policy() {
        let rows = list_topics_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let by_c = topics_by_compaction(&rows);
        let total_in_map: usize = by_c.values().sum();
        assert_eq!(total_in_map, rows.len());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_topics_sorted(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_topic_count_and_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Topics ("));
        assert!(html.contains("Kafka topics"));
    }
}
