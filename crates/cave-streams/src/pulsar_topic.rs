//! Pulsar-style topic addressing: `persistent://tenant/ns/topic`.
//!
//! Cave Streams uses Pulsar's hierarchical naming scheme as the canonical
//! address; the Kafka wire layer translates a flat Kafka topic into this
//! form by treating the first two slash-separated segments as
//! `tenant/namespace` and the remainder as the local topic.  When no
//! slash is present, both default to `public/default`
//! ([`crate::tenant::DEFAULT_TENANT`] / [`crate::tenant::DEFAULT_NAMESPACE`]).
//!
//! Upstream reference: Apache Pulsar 4.2.0
//! `pulsar-common/src/main/java/org/apache/pulsar/common/naming/TopicName.java`.

use crate::error::{StreamsError, StreamsResult};
use crate::tenant::{DEFAULT_NAMESPACE, DEFAULT_TENANT};
use std::fmt;

/// Persistence flavour of a topic.  Pulsar 4.x ships only `persistent://`
/// and `non-persistent://`; cave-streams accepts the same set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopicDomain {
    Persistent,
    NonPersistent,
}

impl TopicDomain {
    pub fn as_scheme(self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::NonPersistent => "non-persistent",
        }
    }

    pub fn parse(s: &str) -> StreamsResult<Self> {
        match s {
            "persistent" => Ok(Self::Persistent),
            "non-persistent" => Ok(Self::NonPersistent),
            other => Err(StreamsError::InvalidTopicName(format!(
                "unknown topic domain {other:?}"
            ))),
        }
    }
}

/// A fully qualified topic identifier: `<scheme>://<tenant>/<namespace>/<local>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TopicName {
    pub domain: TopicDomain,
    pub tenant: String,
    pub namespace: String,
    pub local: String,
    /// Partition index for partitioned topics
    /// (Pulsar appends `-partition-N` to the local name).  `None` for
    /// non-partitioned and for the umbrella partitioned-topic root.
    pub partition: Option<i32>,
}

impl TopicName {
    /// Build a non-partitioned `persistent://t/ns/l` from parts (validated).
    pub fn persistent(
        tenant: impl Into<String>,
        namespace: impl Into<String>,
        local: impl Into<String>,
    ) -> StreamsResult<Self> {
        let t = tenant.into();
        let n = namespace.into();
        let l = local.into();
        validate_segment("tenant", &t)?;
        validate_segment("namespace", &n)?;
        validate_segment("topic", &l)?;
        Ok(Self {
            domain: TopicDomain::Persistent,
            tenant: t,
            namespace: n,
            local: l,
            partition: None,
        })
    }

    /// Parse a fully-qualified Pulsar topic string.
    /// Accepts `persistent://t/ns/l` and `persistent://t/ns/l-partition-N`.
    pub fn parse(s: &str) -> StreamsResult<Self> {
        let (scheme, rest) = s
            .split_once("://")
            .ok_or_else(|| StreamsError::InvalidTopicName(s.into()))?;
        let domain = TopicDomain::parse(scheme)?;
        let parts: Vec<&str> = rest.splitn(3, '/').collect();
        if parts.len() != 3 {
            return Err(StreamsError::InvalidTopicName(format!(
                "expected tenant/namespace/topic, got {s:?}"
            )));
        }
        let (tenant, namespace, local_full) = (parts[0], parts[1], parts[2]);
        validate_segment("tenant", tenant)?;
        validate_segment("namespace", namespace)?;

        let (local, partition) = split_partition(local_full)?;
        validate_segment("topic", &local)?;
        Ok(Self {
            domain,
            tenant: tenant.into(),
            namespace: namespace.into(),
            local,
            partition,
        })
    }

    /// Translate a flat Kafka-style topic (`my.topic` or `tenant/ns/topic`)
    /// into the Pulsar canonical form.  Defaults to `public/default` when no
    /// explicit prefix is present.
    pub fn from_kafka_topic(s: &str) -> StreamsResult<Self> {
        if s.is_empty() {
            return Err(StreamsError::InvalidTopicName(s.into()));
        }
        if s.contains("://") {
            return Self::parse(s);
        }
        let parts: Vec<&str> = s.splitn(3, '/').collect();
        let (tenant, namespace, local) = match parts.as_slice() {
            [t, ns, l] => (t.to_string(), ns.to_string(), l.to_string()),
            _ => (
                DEFAULT_TENANT.to_string(),
                DEFAULT_NAMESPACE.to_string(),
                s.to_string(),
            ),
        };
        Self::persistent(tenant, namespace, local)
    }

    /// Fully-qualified string form (`persistent://t/ns/l[-partition-N]`).
    pub fn to_string_full(&self) -> String {
        match self.partition {
            None => format!(
                "{}://{}/{}/{}",
                self.domain.as_scheme(),
                self.tenant,
                self.namespace,
                self.local
            ),
            Some(p) => format!(
                "{}://{}/{}/{}-partition-{}",
                self.domain.as_scheme(),
                self.tenant,
                self.namespace,
                self.local,
                p
            ),
        }
    }

    /// `tenant/namespace` (Pulsar admin namespace identifier).
    pub fn namespace_fqn(&self) -> String {
        format!("{}/{}", self.tenant, self.namespace)
    }

    /// Kafka-style flat topic name used on the Kafka wire (`tenant/ns/local`).
    /// Round-trips through [`Self::from_kafka_topic`].
    pub fn to_kafka_topic(&self) -> String {
        let local = match self.partition {
            None => self.local.clone(),
            Some(p) => format!("{}-{}", self.local, p),
        };
        if self.tenant == DEFAULT_TENANT && self.namespace == DEFAULT_NAMESPACE {
            local
        } else {
            format!("{}/{}/{}", self.tenant, self.namespace, local)
        }
    }

    /// Construct the partition-N child of a partitioned topic.
    pub fn partition_of(&self, idx: i32) -> StreamsResult<Self> {
        if idx < 0 {
            return Err(StreamsError::InvalidTopicName(format!(
                "partition index must be ≥ 0, got {idx}"
            )));
        }
        Ok(Self {
            partition: Some(idx),
            ..self.clone()
        })
    }
}

impl fmt::Display for TopicName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_full())
    }
}

fn split_partition(s: &str) -> StreamsResult<(String, Option<i32>)> {
    if let Some(idx) = s.rfind("-partition-") {
        let (left, right) = s.split_at(idx);
        let suffix = &right["-partition-".len()..];
        let n: i32 = suffix
            .parse()
            .map_err(|_| StreamsError::InvalidTopicName(format!("bad partition suffix: {right}")))?;
        if n < 0 {
            return Err(StreamsError::InvalidTopicName(
                "partition index must be ≥ 0".into(),
            ));
        }
        Ok((left.to_string(), Some(n)))
    } else {
        Ok((s.to_string(), None))
    }
}

fn validate_segment(label: &str, s: &str) -> StreamsResult<()> {
    if s.is_empty() {
        return Err(StreamsError::InvalidTopicName(format!(
            "empty {label} segment"
        )));
    }
    if s.contains('/') {
        return Err(StreamsError::InvalidTopicName(format!(
            "{label} must not contain '/'"
        )));
    }
    if !s.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == ':'
    }) {
        return Err(StreamsError::InvalidTopicName(format!(
            "{label} contains illegal character: {s:?}"
        )));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// pulsar_topic tests
// feat/cave-streams-kafka-pulsar-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_name_parse_persistent() {
        // cite: pulsar 4.2.0 .../naming/TopicName.java#get(String)
        let tenant_id = "topic-001";
        let s = format!("persistent://{}/ns/orders", tenant_id);
        let t = TopicName::parse(&s).unwrap();
        assert_eq!(t.domain, TopicDomain::Persistent);
        assert_eq!(t.tenant, tenant_id);
        assert_eq!(t.namespace, "ns");
        assert_eq!(t.local, "orders");
        assert_eq!(t.partition, None);
    }

    #[test]
    fn test_topic_name_parse_non_persistent() {
        // cite: pulsar 4.2.0 .../TopicName.java domain matrix
        let tenant_id = "topic-002";
        let s = format!("non-persistent://{}/ns/ephemeral", tenant_id);
        let t = TopicName::parse(&s).unwrap();
        assert_eq!(t.domain, TopicDomain::NonPersistent);
        assert_eq!(t.local, "ephemeral");
    }

    #[test]
    fn test_topic_name_parse_partition_suffix() {
        // cite: pulsar 4.2.0 .../TopicName.java getPartitionIndex
        let tenant_id = "topic-003";
        let s = format!("persistent://{}/ns/orders-partition-3", tenant_id);
        let t = TopicName::parse(&s).unwrap();
        assert_eq!(t.local, "orders");
        assert_eq!(t.partition, Some(3));
    }

    #[test]
    fn test_topic_name_round_trip() {
        // cite: pulsar 4.2.0 .../TopicName.java#toString
        let tenant_id = "topic-004";
        let original = format!("persistent://{}/ns/events", tenant_id);
        let parsed = TopicName::parse(&original).unwrap();
        assert_eq!(parsed.to_string_full(), original);
    }

    #[test]
    fn test_topic_name_partition_round_trip() {
        // cite: pulsar 4.2.0 .../TopicName.java getPartition(idx)
        let tenant_id = "topic-005";
        let root = TopicName::persistent(tenant_id, "ns", "events").unwrap();
        let p = root.partition_of(7).unwrap();
        let s = p.to_string_full();
        let back = TopicName::parse(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn test_topic_name_rejects_missing_segments() {
        // cite: pulsar 4.2.0 TopicName.validateNamespace
        let _tenant_id = "topic-006";
        let err = TopicName::parse("persistent://only/two");
        assert!(err.is_err());
    }

    #[test]
    fn test_topic_name_rejects_unknown_domain() {
        // cite: pulsar 4.2.0 TopicDomain.getEnum
        let _tenant_id = "topic-007";
        let err = TopicName::parse("memory://t/ns/foo");
        assert!(err.is_err());
    }

    #[test]
    fn test_topic_name_rejects_illegal_char() {
        // cite: pulsar 4.2.0 TopicName.validateTopic#segment validation
        let _tenant_id = "topic-008";
        // space is not in the allowed alphabet
        let err = TopicName::persistent("public", "default", "bad name");
        assert!(err.is_err());
    }

    #[test]
    fn test_topic_name_rejects_negative_partition() {
        // cite: pulsar 4.2.0 TopicName.getPartition (idx >= 0)
        let _tenant_id = "topic-009";
        let root = TopicName::persistent("public", "default", "x").unwrap();
        assert!(root.partition_of(-1).is_err());
    }

    #[test]
    fn test_topic_name_from_kafka_flat_topic_uses_defaults() {
        // cite: cave ADR-RUNTIME-STREAMING-CONSOLIDATION-001 §addressing
        let tenant_id = "topic-010";
        let kafka = format!("kafka-{}", tenant_id);
        let t = TopicName::from_kafka_topic(&kafka).unwrap();
        assert_eq!(t.tenant, DEFAULT_TENANT);
        assert_eq!(t.namespace, DEFAULT_NAMESPACE);
        assert_eq!(t.local, kafka);
    }

    #[test]
    fn test_topic_name_from_kafka_full_path() {
        // cite: cave ADR-RUNTIME-STREAMING-CONSOLIDATION-001 §addressing.kafka
        let tenant_id = "topic-011";
        let kafka = format!("{}/ns/orders", tenant_id);
        let t = TopicName::from_kafka_topic(&kafka).unwrap();
        assert_eq!(t.tenant, tenant_id);
        assert_eq!(t.namespace, "ns");
        assert_eq!(t.local, "orders");
    }

    #[test]
    fn test_topic_name_to_kafka_topic_round_trip() {
        // cite: cave ADR-RUNTIME-STREAMING-CONSOLIDATION-001 §addressing.kafka
        let tenant_id = "topic-012";
        let original = format!("{}/ns/orders", tenant_id);
        let t = TopicName::from_kafka_topic(&original).unwrap();
        assert_eq!(t.to_kafka_topic(), original);
    }
}
