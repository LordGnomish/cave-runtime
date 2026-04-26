//! Tenant-scoped topic routing.
//!
//! Cite: debezium `TopicNamingStrategy` (default = `<server>.<schema>.<table>`).
//! cave deviates so every topic is hard-prefixed with the tenant_id —
//! this guarantees a misconfigured connector cannot publish into
//! another tenant's namespace.

use crate::error::{CdcError, CdcResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoutingPolicy {
    /// `<tenant>.<server>.<schema>.<table>`.
    /// Cite: debezium default `<server>.<schema>.<table>` with cave
    /// tenant prefix.
    SchemaTable,
    /// `<tenant>.<server>.<aggregate_type>` — for outbox routing.
    /// Cite: debezium `OutboxEventRouter` `route.topic.replacement`.
    OutboxAggregate,
    /// `<tenant>.<server>` — single-topic-per-server.
    SingleTopic,
}

#[derive(Debug, Clone)]
pub struct TopicRouter {
    pub tenant_id: String,
    pub server: String,
    pub policy: RoutingPolicy,
}

impl TopicRouter {
    pub fn new(
        tenant_id: impl Into<String>,
        server: impl Into<String>,
        policy: RoutingPolicy,
    ) -> Self {
        Self { tenant_id: tenant_id.into(), server: server.into(), policy }
    }

    /// Build the destination topic name. Cite: debezium
    /// `TopicNamingStrategy::dataChangeTopic`.
    pub fn topic_for_change(&self, schema: &str, table: &str) -> CdcResult<String> {
        validate_segment(&self.tenant_id, "tenant_id")?;
        validate_segment(&self.server, "server")?;
        validate_segment(schema, "schema")?;
        validate_segment(table, "table")?;
        Ok(match self.policy {
            RoutingPolicy::SchemaTable     => format!("{}.{}.{}.{}",
                self.tenant_id, self.server, schema, table),
            RoutingPolicy::OutboxAggregate => format!("{}.{}.{}",
                self.tenant_id, self.server, schema), // schema = aggregate_type
            RoutingPolicy::SingleTopic     => format!("{}.{}",
                self.tenant_id, self.server),
        })
    }

    /// Cite: debezium `Partitioner` SPI — the routing layer decides
    /// the partition for a given key. Default = stable hash mod
    /// `partition_count`.
    pub fn partition_for(&self, key: &[u8], partition_count: i32) -> i32 {
        if partition_count <= 0 { return 0; }
        let mut h: u64 = 1469598103934665603;
        for b in self.tenant_id.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(1099511628211); }
        for b in key { h ^= *b as u64; h = h.wrapping_mul(1099511628211); }
        ((h % partition_count as u64) as i64) as i32
    }

    /// Validate that no constructed topic crosses tenants. cave
    /// invariant — every topic emitted by this router MUST start with
    /// `<tenant_id>.`.
    pub fn assert_tenant_prefix(&self, topic: &str) -> CdcResult<()> {
        let prefix = format!("{}.", self.tenant_id);
        if !topic.starts_with(&prefix) {
            return Err(CdcError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: topic.to_string(),
            });
        }
        Ok(())
    }
}

fn validate_segment(s: &str, label: &str) -> CdcResult<()> {
    if s.is_empty() {
        return Err(CdcError::InvalidConfig(format!("{} must be non-empty", label)));
    }
    if s.contains('.') || s.contains(' ') {
        return Err(CdcError::InvalidConfig(format!(
            "{} '{}' may not contain '.' or whitespace (would split topic name)",
            label, s,
        )));
    }
    Ok(())
}
