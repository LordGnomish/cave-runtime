// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kafka L7 policy — `PortRuleKafka` evaluator.
//!
//! Mirrors `pkg/policy/api/kafka.go` and the Kafka request/response
//! decoder in `pkg/proxy/kafka`.
//!
//! Semantics (faithful to upstream):
//!
//! * A `KafkaRule` AND-combines four optional matchers: `role` (Produce
//!   / Consume — a coarse classifier over API keys), `api_key` (specific
//!   Kafka API like Fetch, Produce, Metadata…), `topic` (regex), and
//!   `client_id` (exact).
//! * An empty rule (no matchers set) allows every Kafka request.
//! * A non-empty rule denies any request that doesn't satisfy *all*
//!   declared matchers.
//! * Multiple rules on the same `PortRule.kafka` list are OR'd: any
//!   matching rule allows the request.
//! * The `Role::Produce` shorthand expands to `{Produce, ApiVersions,
//!   Metadata}`. `Role::Consume` expands to `{Fetch, OffsetCommit,
//!   OffsetFetch, FindCoordinator, JoinGroup, Heartbeat, LeaveGroup,
//!   SyncGroup, DescribeGroups, ListGroups, ApiVersions, Metadata}`.
//! * `topic` is a Cilium-flavoured regex (anchored, supports `.`, `*`,
//!   `[a-z]`, `\d`).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KafkaRole {
    Produce,
    Consume,
}

/// Subset of Kafka API keys Cilium documents (mirrors upstream
/// `pkg/policy/api/kafka.go::KafkaAPIKeyMap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KafkaApiKey {
    Produce,
    Fetch,
    ListOffsets,
    Metadata,
    LeaderAndIsr,
    StopReplica,
    UpdateMetadata,
    ControlledShutdown,
    OffsetCommit,
    OffsetFetch,
    FindCoordinator,
    JoinGroup,
    Heartbeat,
    LeaveGroup,
    SyncGroup,
    DescribeGroups,
    ListGroups,
    SaslHandshake,
    ApiVersions,
    CreateTopics,
    DeleteTopics,
    AlterConfigs,
    DescribeConfigs,
}

impl KafkaApiKey {
    /// Numeric API key per Kafka protocol spec (also what the wire format
    /// carries). Mirrors `pkg/proxy/kafka/protocol.go::APIKey`.
    pub fn numeric(self) -> u16 {
        match self {
            KafkaApiKey::Produce => 0,
            KafkaApiKey::Fetch => 1,
            KafkaApiKey::ListOffsets => 2,
            KafkaApiKey::Metadata => 3,
            KafkaApiKey::LeaderAndIsr => 4,
            KafkaApiKey::StopReplica => 5,
            KafkaApiKey::UpdateMetadata => 6,
            KafkaApiKey::ControlledShutdown => 7,
            KafkaApiKey::OffsetCommit => 8,
            KafkaApiKey::OffsetFetch => 9,
            KafkaApiKey::FindCoordinator => 10,
            KafkaApiKey::JoinGroup => 11,
            KafkaApiKey::Heartbeat => 12,
            KafkaApiKey::LeaveGroup => 13,
            KafkaApiKey::SyncGroup => 14,
            KafkaApiKey::DescribeGroups => 15,
            KafkaApiKey::ListGroups => 16,
            KafkaApiKey::SaslHandshake => 17,
            KafkaApiKey::ApiVersions => 18,
            KafkaApiKey::CreateTopics => 19,
            KafkaApiKey::DeleteTopics => 20,
            KafkaApiKey::AlterConfigs => 33,
            KafkaApiKey::DescribeConfigs => 32,
        }
    }
}

/// Expand a `KafkaRole` to the upstream-defined set of API keys.
pub fn role_api_keys(role: KafkaRole) -> Vec<KafkaApiKey> {
    match role {
        KafkaRole::Produce => vec![
            KafkaApiKey::Produce,
            KafkaApiKey::ApiVersions,
            KafkaApiKey::Metadata,
        ],
        KafkaRole::Consume => vec![
            KafkaApiKey::Fetch,
            KafkaApiKey::OffsetCommit,
            KafkaApiKey::OffsetFetch,
            KafkaApiKey::FindCoordinator,
            KafkaApiKey::JoinGroup,
            KafkaApiKey::Heartbeat,
            KafkaApiKey::LeaveGroup,
            KafkaApiKey::SyncGroup,
            KafkaApiKey::DescribeGroups,
            KafkaApiKey::ListGroups,
            KafkaApiKey::ApiVersions,
            KafkaApiKey::Metadata,
        ],
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KafkaRule {
    pub role: Option<KafkaRole>,
    pub api_key: Option<KafkaApiKey>,
    /// Topic name. Empty matches any topic. Regex with the same subset
    /// supported by HTTP path regex (`.`, `*`, `[a-z]`, `\d`).
    pub topic: Option<String>,
    pub client_id: Option<String>,
}

impl KafkaRule {
    pub fn allow_all() -> Self {
        Self { role: None, api_key: None, topic: None, client_id: None }
    }
    pub fn is_empty(&self) -> bool {
        self.role.is_none() && self.api_key.is_none() && self.topic.is_none() && self.client_id.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KafkaRequest {
    pub api_key: KafkaApiKey,
    pub topic: Option<String>,
    pub client_id: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KafkaError {
    #[error("invalid topic regex `{0}`")]
    BadTopicRegex(String),
    #[error("tenant {tenant} cannot evaluate kafka rule owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KafkaVerdict {
    Allow,
    Deny,
}

/// Evaluate a list of Kafka rules against a request. Mirrors
/// `pkg/proxy/kafka::checkRule`. Empty list → Allow (mirrors upstream
/// "no L7 rules → allow" semantics).
pub fn evaluate(rules: &[KafkaRule], req: &KafkaRequest) -> Result<KafkaVerdict, KafkaError> {
    if rules.is_empty() {
        return Ok(KafkaVerdict::Allow);
    }
    for r in rules {
        if rule_matches(r, req)? {
            return Ok(KafkaVerdict::Allow);
        }
    }
    Ok(KafkaVerdict::Deny)
}

fn rule_matches(rule: &KafkaRule, req: &KafkaRequest) -> Result<bool, KafkaError> {
    if rule.is_empty() {
        return Ok(true);
    }
    if let Some(role) = rule.role {
        if !role_api_keys(role).contains(&req.api_key) {
            return Ok(false);
        }
    }
    if let Some(key) = rule.api_key {
        if key != req.api_key {
            return Ok(false);
        }
    }
    if let Some(t) = &rule.topic {
        let req_topic = req.topic.as_deref().unwrap_or("");
        if !topic_matches(t, req_topic)? {
            return Ok(false);
        }
    }
    if let Some(cid) = &rule.client_id {
        if Some(cid) != req.client_id.as_ref() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn topic_matches(pattern: &str, name: &str) -> Result<bool, KafkaError> {
    // Reuse the simplified regex semantics used by HTTP path regex.
    Ok(simple_regex(pattern, name))
}

fn simple_regex(pattern: &str, input: &str) -> bool {
    // Implement the subset documented for Cilium: literal, `.`, `.*`,
    // `[a-z]`, `\d`, anchored full-string match.
    let pat = pattern.strip_prefix('^').unwrap_or(pattern);
    let pat = pat.strip_suffix('$').unwrap_or(pat);
    apply(pat.as_bytes(), 0, input.as_bytes(), 0)
}

fn apply(pat: &[u8], pi: usize, s: &[u8], si: usize) -> bool {
    if pi >= pat.len() {
        return si == s.len();
    }
    // Parse one element + check for trailing `*`.
    let (elem_end, predicate) = parse_element(pat, pi);
    let star = elem_end < pat.len() && pat[elem_end] == b'*';
    if star {
        // Greedy: try longest match first, back off.
        let mut k = si;
        while k < s.len() && predicate(s[k]) {
            k += 1;
        }
        loop {
            if apply(pat, elem_end + 1, s, k) {
                return true;
            }
            if k == si {
                return false;
            }
            k -= 1;
        }
    }
    if si >= s.len() {
        return false;
    }
    if predicate(s[si]) {
        apply(pat, elem_end, s, si + 1)
    } else {
        false
    }
}

/// Parse a single regex element starting at `pi`. Returns the index after
/// the element and a closure that tests one character.
fn parse_element(pat: &[u8], pi: usize) -> (usize, Box<dyn Fn(u8) -> bool>) {
    match pat[pi] {
        b'.' => (pi + 1, Box::new(|_| true)),
        b'\\' if pi + 1 < pat.len() && pat[pi + 1] == b'd' => {
            (pi + 2, Box::new(|c: u8| c.is_ascii_digit()))
        }
        b'\\' if pi + 1 < pat.len() => {
            let c = pat[pi + 1];
            (pi + 2, Box::new(move |x: u8| x == c))
        }
        b'[' => {
            if let Some(end) = pat[pi..].iter().position(|&c| c == b']') {
                let body: Vec<u8> = pat[pi + 1..pi + end].to_vec();
                let pred = move |x: u8| {
                    let mut j = 0;
                    while j < body.len() {
                        if j + 2 < body.len() && body[j + 1] == b'-' {
                            if x >= body[j] && x <= body[j + 2] {
                                return true;
                            }
                            j += 3;
                        } else {
                            if body[j] == x {
                                return true;
                            }
                            j += 1;
                        }
                    }
                    false
                };
                (pi + end + 1, Box::new(pred))
            } else {
                (pi + 1, Box::new(|_| false))
            }
        }
        c => (pi + 1, Box::new(move |x: u8| x == c)),
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/policy/api/kafka.go", "PortRuleKafka");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn req(api: KafkaApiKey, topic: &str, cid: &str) -> KafkaRequest {
        KafkaRequest {
            api_key: api,
            topic: if topic.is_empty() { None } else { Some(topic.to_string()) },
            client_id: if cid.is_empty() { None } else { Some(cid.to_string()) },
        }
    }

    // ── API-key numerics ─────────────────────────────────────────────────────

    #[test]
    fn kafka_api_key_numeric_matches_protocol_spec() {
        let (_c, _t) = cilium_test_ctx!("pkg/proxy/kafka/protocol.go", "APIKey", "tenant-kf-num");
        assert_eq!(KafkaApiKey::Produce.numeric(), 0);
        assert_eq!(KafkaApiKey::Fetch.numeric(), 1);
        assert_eq!(KafkaApiKey::Metadata.numeric(), 3);
        assert_eq!(KafkaApiKey::OffsetCommit.numeric(), 8);
        assert_eq!(KafkaApiKey::ApiVersions.numeric(), 18);
    }

    #[test]
    fn kafka_role_produce_expands_to_produce_and_metadata() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "Role.Produce", "tenant-kf-rprod");
        let keys = role_api_keys(KafkaRole::Produce);
        assert!(keys.contains(&KafkaApiKey::Produce));
        assert!(keys.contains(&KafkaApiKey::ApiVersions));
        assert!(keys.contains(&KafkaApiKey::Metadata));
        assert!(!keys.contains(&KafkaApiKey::Fetch));
    }

    #[test]
    fn kafka_role_consume_expands_to_consumer_keys() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "Role.Consume", "tenant-kf-rcons");
        let keys = role_api_keys(KafkaRole::Consume);
        assert!(keys.contains(&KafkaApiKey::Fetch));
        assert!(keys.contains(&KafkaApiKey::JoinGroup));
        assert!(keys.contains(&KafkaApiKey::Heartbeat));
        assert!(!keys.contains(&KafkaApiKey::Produce));
    }

    // ── Empty / allow-all ────────────────────────────────────────────────────

    #[test]
    fn kafka_empty_rule_list_allows_everything() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.Empty", "tenant-kf-empty");
        let v = evaluate(&[], &req(KafkaApiKey::Produce, "orders", "client-A")).unwrap();
        assert_eq!(v, KafkaVerdict::Allow);
    }

    #[test]
    fn kafka_empty_rule_in_list_allows_anything() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.AllowAll", "tenant-kf-allowall");
        let v = evaluate(&[KafkaRule::allow_all()], &req(KafkaApiKey::Produce, "orders", "client")).unwrap();
        assert_eq!(v, KafkaVerdict::Allow);
    }

    // ── role + api_key ───────────────────────────────────────────────────────

    #[test]
    fn kafka_role_produce_blocks_fetch_request() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "Role.Produce.BlocksFetch", "tenant-kf-rblock");
        let r = KafkaRule { role: Some(KafkaRole::Produce), ..KafkaRule::allow_all() };
        let v = evaluate(&[r], &req(KafkaApiKey::Fetch, "orders", "client")).unwrap();
        assert_eq!(v, KafkaVerdict::Deny);
    }

    #[test]
    fn kafka_role_produce_allows_produce_request() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "Role.Produce.AllowsProduce", "tenant-kf-rallow");
        let r = KafkaRule { role: Some(KafkaRole::Produce), ..KafkaRule::allow_all() };
        let v = evaluate(&[r], &req(KafkaApiKey::Produce, "orders", "client")).unwrap();
        assert_eq!(v, KafkaVerdict::Allow);
    }

    #[test]
    fn kafka_specific_api_key_matches_request() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.APIKey", "tenant-kf-akey");
        let r = KafkaRule { api_key: Some(KafkaApiKey::Fetch), ..KafkaRule::allow_all() };
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Fetch, "x", "y")).unwrap(), KafkaVerdict::Allow);
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "x", "y")).unwrap(), KafkaVerdict::Deny);
    }

    #[test]
    fn kafka_role_and_api_key_must_both_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.AND", "tenant-kf-and");
        let r = KafkaRule {
            role: Some(KafkaRole::Consume),
            api_key: Some(KafkaApiKey::Fetch),
            ..KafkaRule::allow_all()
        };
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Fetch, "t", "")).unwrap(), KafkaVerdict::Allow);
        // Heartbeat is in Consume role but not the specified api_key.
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Heartbeat, "t", "")).unwrap(), KafkaVerdict::Deny);
    }

    // ── topic ────────────────────────────────────────────────────────────────

    #[test]
    fn kafka_topic_exact_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.Topic", "tenant-kf-topex");
        let r = KafkaRule { topic: Some("orders".into()), ..KafkaRule::allow_all() };
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Produce, "orders", "")).unwrap(), KafkaVerdict::Allow);
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "events", "")).unwrap(), KafkaVerdict::Deny);
    }

    #[test]
    fn kafka_topic_regex_with_dot_star() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.TopicRegex", "tenant-kf-toprx");
        let r = KafkaRule { topic: Some("events\\.[a-z]*".into()), ..KafkaRule::allow_all() };
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Produce, "events.orders", "")).unwrap(), KafkaVerdict::Allow);
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Produce, "events.users", "")).unwrap(), KafkaVerdict::Allow);
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "logs.orders", "")).unwrap(), KafkaVerdict::Deny);
    }

    #[test]
    fn kafka_topic_regex_digit_class() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.TopicDigit", "tenant-kf-topdig");
        let r = KafkaRule { topic: Some("part-\\d".into()), ..KafkaRule::allow_all() };
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Produce, "part-7", "")).unwrap(), KafkaVerdict::Allow);
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "part-x", "")).unwrap(), KafkaVerdict::Deny);
    }

    // ── client_id ────────────────────────────────────────────────────────────

    #[test]
    fn kafka_client_id_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.ClientID", "tenant-kf-cid");
        let r = KafkaRule { client_id: Some("billing-svc".into()), ..KafkaRule::allow_all() };
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Produce, "x", "billing-svc")).unwrap(), KafkaVerdict::Allow);
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "x", "other-svc")).unwrap(), KafkaVerdict::Deny);
    }

    // ── multi-rule OR ────────────────────────────────────────────────────────

    #[test]
    fn kafka_multi_rule_first_match_allows() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.MultiRule", "tenant-kf-multi");
        let rules = vec![
            KafkaRule { topic: Some("orders".into()), ..KafkaRule::allow_all() },
            KafkaRule { topic: Some("payments".into()), ..KafkaRule::allow_all() },
        ];
        assert_eq!(evaluate(&rules, &req(KafkaApiKey::Produce, "payments", "")).unwrap(), KafkaVerdict::Allow);
        assert_eq!(evaluate(&rules, &req(KafkaApiKey::Produce, "logs", "")).unwrap(), KafkaVerdict::Deny);
    }

    // ── Role + topic ─────────────────────────────────────────────────────────

    #[test]
    fn kafka_role_and_topic_combine_with_and() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.RoleTopic", "tenant-kf-rt");
        let r = KafkaRule {
            role: Some(KafkaRole::Consume),
            topic: Some("orders".into()),
            ..KafkaRule::allow_all()
        };
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Fetch, "orders", "")).unwrap(), KafkaVerdict::Allow);
        // Consume role but wrong topic.
        assert_eq!(evaluate(&[r.clone()], &req(KafkaApiKey::Fetch, "logs", "")).unwrap(), KafkaVerdict::Deny);
        // Right topic but Produce role.
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "orders", "")).unwrap(), KafkaVerdict::Deny);
    }

    // ── Verdict default ──────────────────────────────────────────────────────

    #[test]
    fn kafka_no_matching_rule_denies() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.NoMatch", "tenant-kf-nomatch");
        let r = KafkaRule { topic: Some("orders".into()), ..KafkaRule::allow_all() };
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "events", "")).unwrap(), KafkaVerdict::Deny);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn kafka_rule_round_trips_serde() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.Serde", "tenant-kf-serde");
        let r = KafkaRule {
            role: Some(KafkaRole::Consume),
            api_key: Some(KafkaApiKey::Fetch),
            topic: Some("orders".into()),
            client_id: Some("svc".into()),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: KafkaRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn kafka_topic_anchored_full_match_required() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "PortRuleKafka.Anchored", "tenant-kf-anchor");
        let r = KafkaRule { topic: Some("ord".into()), ..KafkaRule::allow_all() };
        // "orders" has extra chars after "ord" → should NOT match (anchored).
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Produce, "orders", "")).unwrap(), KafkaVerdict::Deny);
    }

    #[test]
    fn kafka_role_consume_includes_metadata() {
        let (_c, _t) = cilium_test_ctx!("pkg/policy/api/kafka.go", "Role.Consume.Metadata", "tenant-kf-cmeta");
        let r = KafkaRule { role: Some(KafkaRole::Consume), ..KafkaRule::allow_all() };
        assert_eq!(evaluate(&[r], &req(KafkaApiKey::Metadata, "", "")).unwrap(), KafkaVerdict::Allow);
    }
}
