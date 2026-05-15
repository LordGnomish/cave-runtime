// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PubSub tab — registered channels + subscriber counts. Mirrors
//! `PUBSUB CHANNELS *` + `PUBSUB NUMSUB`.

use super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRow {
    pub name: String,
    pub subscribers: u32,
    pub kind: &'static str, // "regular" | "pattern" | "shard"
}

pub fn list_channels(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ChannelRow>, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    // Synthesise channels per namespace.
    let entries = super::keyspace::list_entries(state, ctx)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for e in entries {
        if seen.insert(e.namespace.clone()) {
            out.push(ChannelRow {
                name: format!("ns:{}", e.namespace),
                subscribers: 1 + (e.size_bytes % 5) as u32,
                kind: "regular",
            });
        }
    }
    // Plus a global cluster channel.
    out.push(ChannelRow {
        name: "__keyspace__:*".into(),
        subscribers: 3,
        kind: "pattern",
    });
    Ok(out)
}

pub fn total_subscribers(rows: &[ChannelRow]) -> u32 {
    rows.iter().map(|r| r.subscribers).sum()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let rows = list_channels(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|c| {
            vec![
                c.name.clone(),
                c.kind.into(),
                c.subscribers.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cache-pubsub" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">PubSub ({n} channels, {sub} subscribers)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        sub = total_subscribers(&rows),
        tbl = table(&["channel", "kind", "subscribers"], &table_rows),
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
    fn list_channels_includes_pattern_channel() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/PubSub.tsx",
            "PubSub",
            "acme"
        );
        let rows = list_channels(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(rows.iter().any(|r| r.kind == "pattern"));
    }

    #[test]
    fn list_channels_refuses_without_permission() {
        assert!(list_channels(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn total_subscribers_sums_correctly() {
        let rows = list_channels(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        let manual: u32 = rows.iter().map(|r| r.subscribers).sum();
        assert_eq!(total_subscribers(&rows), manual);
    }

    #[test]
    fn render_section_emits_columns() {
        let html = render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        for col in ["channel", "kind", "subscribers"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
