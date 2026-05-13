//! `/admin/streams/acls` — Kafka admin "ACLs" tab. cave-streams
//! ACLs live inside the broker (`AclStore`); this view exposes
//! a synthesized per-tenant policy summary from the topic
//! registry — each topic the tenant owns implies an
//! `ALLOW <tenant-principal> READ/WRITE` rule against the
//! Kafka default ACL surface.
//!
//! Upstream: <https://kafka.apache.org/documentation/#security_authz>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use super::StreamsViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclRow {
    pub principal: String,
    pub resource_type: &'static str,
    pub resource_name: String,
    pub operation: &'static str,
    pub permission: &'static str,
}

pub fn synthesize_acls(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<AclRow>, StreamsViewError> {
    let topics = super::topics::list_topics_sorted(state, ctx)?;
    let principal = format!("User:{}", ctx.tenant.as_str());
    let mut rows = Vec::with_capacity(topics.len() * 2);
    for t in &topics {
        rows.push(AclRow {
            principal: principal.clone(),
            resource_type: "Topic",
            resource_name: t.name.clone(),
            operation: "READ",
            permission: "ALLOW",
        });
        rows.push(AclRow {
            principal: principal.clone(),
            resource_type: "Topic",
            resource_name: t.name.clone(),
            operation: "WRITE",
            permission: "ALLOW",
        });
    }
    Ok(rows)
}

pub fn count_by_operation(rows: &[AclRow]) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.operation).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = synthesize_acls(state, ctx)?;
    let by_op = count_by_operation(&rows);
    let chips: String = by_op
        .iter()
        .map(|(op, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-orange-100 text-sm">{op} <strong>×{n}</strong></span>"#,
                op = op,
                n = n,
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.principal),
                r.resource_type.to_string(),
                escape(&r.resource_name),
                r.operation.to_string(),
                r.permission.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">ACLs ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Tenant-scoped allow rules. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#security_authz">Kafka ACLs</a>.
  </p>
  {tbl}
</section>"#,
        chips = chips,
        n = rows.len(),
        tbl = table(
            &["principal", "resource_type", "name", "op", "permission"],
            &table_rows
        ),
    );
    Ok(page_shell(
        &format!("streams/acls · {}", escape(ctx.tenant.as_str())),
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
    fn synthesize_produces_two_acls_per_topic() {
        let topics = super::super::topics::list_topics_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let rows = synthesize_acls(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert_eq!(rows.len(), topics.len() * 2);
    }

    #[test]
    fn synthesize_uses_tenant_principal() {
        let rows = synthesize_acls(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(rows.iter().all(|r| r.principal == "User:acme"));
    }

    #[test]
    fn count_by_operation_splits_read_write_evenly() {
        let rows = synthesize_acls(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let by_op = count_by_operation(&rows);
        assert_eq!(by_op.get("READ"), by_op.get("WRITE"));
    }

    #[test]
    fn synthesize_rejects_without_permission() {
        assert!(synthesize_acls(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_acl_count_and_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("ACLs ("));
        assert!(html.contains("Kafka ACLs"));
    }
}
