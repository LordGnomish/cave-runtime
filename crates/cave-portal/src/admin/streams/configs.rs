// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/configs` — broker + Connect worker configuration
//! explorer. Surfaces the (synthesised) override chain for the
//! handful of configs the upstream `kafka-configs.sh
//! --describe --all` REST endpoint reports.
//!
//! Persona gate: PlatformAdmin only.
//!
//! Source override chain — same shape as upstream Kafka returns:
//!   * `STATIC_BROKER_CONFIG` — `server.properties`
//!   * `DYNAMIC_BROKER_CONFIG` — `kafka-configs --alter`
//!   * `DEFAULT_CONFIG` — built-in default

use crate::admin::permission::{Permission, Persona, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::StreamsViewError;

/// Override sources the upstream Kafka API uses.
pub const SOURCE_STATIC: &str = "STATIC_BROKER_CONFIG";
pub const SOURCE_DYNAMIC: &str = "DYNAMIC_BROKER_CONFIG";
pub const SOURCE_DEFAULT: &str = "DEFAULT_CONFIG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    pub component: &'static str,
    pub key: &'static str,
    pub value: String,
    pub source: &'static str,
    /// True iff sensitive (e.g. passwords) — value would be redacted
    /// upstream. None of our seeded values are.
    pub sensitive: bool,
}

/// Static set of broker + Connect worker configs cave-streams ships.
pub fn list_configs(_state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ConfigEntry>, StreamsViewError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    ctx.authorise(Permission::StreamsRead)?;
    Ok(vec![
        ConfigEntry {
            component: "broker",
            key: "listeners",
            value: "PLAINTEXT://0.0.0.0:9092".into(),
            source: SOURCE_STATIC,
            sensitive: false,
        },
        ConfigEntry {
            component: "broker",
            key: "log.retention.hours",
            value: "168".into(),
            source: SOURCE_DEFAULT,
            sensitive: false,
        },
        ConfigEntry {
            component: "broker",
            key: "log.segment.bytes",
            value: "1073741824".into(),
            source: SOURCE_DEFAULT,
            sensitive: false,
        },
        ConfigEntry {
            component: "broker",
            key: "num.partitions",
            value: "3".into(),
            source: SOURCE_DYNAMIC,
            sensitive: false,
        },
        ConfigEntry {
            component: "broker",
            key: "default.replication.factor",
            value: "1".into(),
            source: SOURCE_DEFAULT,
            sensitive: false,
        },
        ConfigEntry {
            component: "broker",
            key: "remote.log.storage.system.enable",
            value: "true".into(),
            source: SOURCE_STATIC,
            sensitive: false,
        },
        ConfigEntry {
            component: "connect",
            key: "group.id",
            value: "cave-connect".into(),
            source: SOURCE_STATIC,
            sensitive: false,
        },
        ConfigEntry {
            component: "connect",
            key: "config.storage.topic",
            value: "_cave_connect_configs".into(),
            source: SOURCE_STATIC,
            sensitive: false,
        },
        ConfigEntry {
            component: "connect",
            key: "offset.storage.topic",
            value: "_cave_connect_offsets".into(),
            source: SOURCE_STATIC,
            sensitive: false,
        },
        ConfigEntry {
            component: "connect",
            key: "status.storage.topic",
            value: "_cave_connect_status".into(),
            source: SOURCE_STATIC,
            sensitive: false,
        },
        ConfigEntry {
            component: "connect",
            key: "key.converter",
            value: "org.apache.kafka.connect.json.JsonConverter".into(),
            source: SOURCE_DEFAULT,
            sensitive: false,
        },
        ConfigEntry {
            component: "connect",
            key: "value.converter",
            value: "org.apache.kafka.connect.json.JsonConverter".into(),
            source: SOURCE_DEFAULT,
            sensitive: false,
        },
    ])
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_configs(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|c| {
            let val = if c.sensitive { "[REDACTED]".to_string() } else { escape(&c.value) };
            vec![
                c.component.to_string(),
                c.key.to_string(),
                val,
                c.source.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Configs ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Broker + Connect worker config explorer (source override chain).
    Upstream: <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#dynamicbrokerconfigs">Dynamic broker configs</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["component", "key", "value", "source"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/configs",
        &format!("streams/configs · {}", escape(ctx.tenant.as_str())),
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
    fn list_configs_returns_broker_and_connect_groups() {
        let rows = list_configs(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(rows.iter().any(|c| c.component == "broker"));
        assert!(rows.iter().any(|c| c.component == "connect"));
    }

    #[test]
    fn list_configs_carries_source_override_chain() {
        let rows = list_configs(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let sources: std::collections::BTreeSet<&'static str> = rows.iter().map(|c| c.source).collect();
        // We should surface all three upstream override tiers.
        assert!(sources.contains(SOURCE_STATIC));
        assert!(sources.contains(SOURCE_DYNAMIC));
        assert!(sources.contains(SOURCE_DEFAULT));
    }

    #[test]
    fn list_configs_rejects_tenant_admin_persona() {
        assert!(list_configs(
            &AdminState::seeded(),
            &ctx_as(Persona::TenantAdmin, &[Permission::StreamsRead]),
        )
        .is_err());
    }

    #[test]
    fn list_configs_accepts_platform_admin() {
        assert!(list_configs(
            &AdminState::seeded(),
            &ctx_as(Persona::PlatformAdmin, &[Permission::StreamsRead]),
        )
        .is_ok());
    }

    #[test]
    fn list_configs_rejects_without_permission() {
        assert!(list_configs(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_table_and_listeners_value() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Configs ("));
        assert!(html.contains("PLAINTEXT://0.0.0.0:9092"));
        assert!(html.contains("STATIC_BROKER_CONFIG"));
    }
}
