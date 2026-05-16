// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/connectors` — Connect connector roster tab.
//! Mirrors `connectors` REST resource from upstream Connect
//! (`GET /connectors`, `GET /connectors/{name}/status`). Per-row
//! detail: name, kind, state, task count, topics, class.
//!
//! Persona gate: PlatformAdmin only — Connect connectors are a
//! platform-wide control surface (no tenant scoping in the
//! upstream Connect model).

use crate::admin::permission::{Permission, Persona, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, StreamsConnector};
use super::StreamsViewError;

pub fn list_connectors(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsConnector>, StreamsViewError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    ctx.authorise(Permission::StreamsRead)?;
    let mut rows: Vec<StreamsConnector> = scope(
        &state.streams_connectors.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub fn count_by_state(
    rows: &[StreamsConnector],
) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.state).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_connectors(state, ctx)?;
    let by_state = count_by_state(&rows);
    let chips: String = by_state
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-indigo-100 text-sm">{s} <strong>×{n}</strong></span>"#,
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|c| {
            vec![
                escape(&c.name),
                c.kind.to_string(),
                c.state.to_string(),
                c.tasks_max.to_string(),
                escape(&c.topics),
                escape(&c.class),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Connectors ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Connect connectors. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#connect">Kafka Connect</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["name", "kind", "state", "tasks_max", "topics", "class"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/connectors",
        &format!("streams/connectors · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }
    fn ctx_as(persona: Persona, perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer_as("acme", perms, persona)
    }

    #[test]
    fn list_returns_seeded_connectors_for_tenant() {
        let rows = list_connectors(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(rows.iter().any(|c| c.name == "jdbc-source"));
        assert!(rows.iter().any(|c| c.name == "hdfs-sink"));
    }

    #[test]
    fn list_sorted_by_name() {
        let rows = list_connectors(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].name <= w[1].name);
        }
    }

    #[test]
    fn list_rejects_tenant_admin_persona() {
        assert!(list_connectors(
            &AdminState::seeded(),
            &ctx_as(Persona::TenantAdmin, &[Permission::StreamsRead]),
        )
        .is_err());
    }

    #[test]
    fn list_accepts_platform_admin_persona() {
        assert!(list_connectors(
            &AdminState::seeded(),
            &ctx_as(Persona::PlatformAdmin, &[Permission::StreamsRead]),
        )
        .is_ok());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_connectors(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn count_by_state_groups_correctly() {
        let rows = list_connectors(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let by_s = count_by_state(&rows);
        let total: usize = by_s.values().sum();
        assert_eq!(total, rows.len());
    }

    #[test]
    fn render_includes_table_and_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Connectors ("));
        assert!(html.contains("jdbc-source"));
        assert!(!html.contains("evil-connector"));
    }
}
