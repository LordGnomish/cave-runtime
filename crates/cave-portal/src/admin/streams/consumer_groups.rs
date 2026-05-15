//! `/admin/streams/consumer_groups` — Kafka admin "Consumer
//! Groups" tab. Per-group state, lag, and topic binding —
//! the page operators visit first when a downstream consumer
//! is slow.
//!
//! Upstream: <https://kafka.apache.org/documentation/#basic_ops_consumer_lag>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, StreamsConsumerGroup};
use super::StreamsViewError;

pub fn list_groups_sorted(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsConsumerGroup>, StreamsViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    let mut rows: Vec<StreamsConsumerGroup> = scope(
        &state.streams_consumer_groups.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| b.lag().cmp(&a.lag()).then(a.group_id.cmp(&b.group_id)));
    Ok(rows)
}

pub fn total_lag(rows: &[StreamsConsumerGroup]) -> u64 {
    rows.iter().map(|g| g.lag()).sum()
}

pub fn lagging_groups<'a>(
    rows: &'a [StreamsConsumerGroup],
    threshold: u64,
) -> Vec<&'a StreamsConsumerGroup> {
    rows.iter().filter(|g| g.lag() >= threshold).collect()
}

pub fn groups_by_state(
    rows: &[StreamsConsumerGroup],
) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.state).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_groups_sorted(state, ctx)?;
    let total = total_lag(&rows);
    let by_state = groups_by_state(&rows);
    let chips: String = by_state
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-yellow-100 text-sm">{s} <strong>×{n}</strong></span>"#,
                s = s,
                n = n,
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.group_id),
                escape(&r.topic),
                r.members.to_string(),
                r.current_offset.to_string(),
                r.log_end_offset.to_string(),
                r.lag().to_string(),
                r.state.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Consumer Groups ({n}) · total lag {total}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Sorted by lag desc. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#basic_ops_consumer_lag">Kafka consumer lag</a>.
  </p>
  {tbl}
</section>"#,
        chips = chips,
        n = rows.len(),
        total = total,
        tbl = table(
            &["group_id", "topic", "members", "offset", "log_end", "lag", "state"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/consumer_groups",
        &format!("streams/consumer_groups · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_seeded_groups_for_tenant() {
        let rows = list_groups_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(rows.iter().all(|g| g.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_sorted_by_lag_desc() {
        let rows = list_groups_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].lag() >= w[1].lag());
        }
    }

    #[test]
    fn total_lag_sums_lag_field() {
        let rows = list_groups_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let total = total_lag(&rows);
        let expected: u64 = rows.iter().map(|g| g.lag()).sum();
        assert_eq!(total, expected);
    }

    #[test]
    fn lagging_groups_filters_by_threshold() {
        let rows = list_groups_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let zero = lagging_groups(&rows, 0);
        assert_eq!(zero.len(), rows.len());
        let huge = lagging_groups(&rows, u64::MAX);
        assert!(huge.is_empty());
    }

    #[test]
    fn groups_by_state_groups_correctly() {
        let rows = list_groups_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let by_s = groups_by_state(&rows);
        let total_in_map: usize = by_s.values().sum();
        assert_eq!(total_in_map, rows.len());
    }

    #[test]
    fn render_includes_lag_total_in_heading() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Consumer Groups ("));
        assert!(html.contains("total lag"));
    }
}
