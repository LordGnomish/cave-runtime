// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Broker controller — reconciler state machine for the Knative Broker CRD.
//!
//! upstream: knative/eventing — pkg/reconciler/broker
//!
//! In the upstream the reconciler walks an informer queue and pushes
//! Broker objects through a series of conditions: ConfigReady →
//! TopicReady (per transport) → IngressReady → FilterReady → Addressable.
//! We port the same state-machine without owning the actual k8s
//! informer; the caller hands us the current state and we compute the
//! next status block + the actions to take.

use crate::eventing::{Channel, Trigger};
use crate::meta::ObjectMeta;
use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct Broker {
    pub metadata: ObjectMeta,
    pub spec: BrokerSpec,
    pub status: BrokerStatus,
    pub channel: Channel,
    pub triggers: Vec<Trigger>,
}

#[derive(Default, Debug, Clone)]
pub struct BrokerSpec {
    /// Name of the transport class (kafka / rabbitmq / pulsar / nats / in-memory).
    pub class: String,
    /// Backing config: connection URL or ConfigMap name.
    pub config_ref: Option<String>,
    /// Optional default delivery policy.
    pub delivery: Option<DeliverySpec>,
}

#[derive(Default, Debug, Clone)]
pub struct DeliverySpec {
    pub retry: u32,
    pub backoff_policy: String, // "linear" | "exponential"
    pub backoff_delay_ms: u64,
    pub dead_letter_sink: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct BrokerStatus {
    pub conditions: HashMap<String, ConditionState>,
    pub address_url: Option<String>,
    pub observed_generation: i64,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionState {
    True,
    False(String),
    Unknown,
}

impl Default for ConditionState {
    fn default() -> Self {
        ConditionState::Unknown
    }
}

/// One action a controller would dispatch — typed so the cave-runtime
/// reconciler can route it without re-parsing the broker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileAction {
    EnsureChannel { class: String, name: String },
    EnsureTopic { class: String, name: String },
    EnsureIngress { broker: String, sink: String },
    EnsureFilter { broker: String, trigger: String },
    EnsureDeadLetter { broker: String, sink: String },
}

impl Broker {
    pub fn new(tenant_id: &str, name: &str, class: &str) -> Self {
        let mut b = Broker::default();
        b.metadata = ObjectMeta::with_creator(tenant_id);
        b.metadata.name = name.to_string();
        b.spec.class = class.to_string();
        b
    }

    pub fn set_condition(&mut self, name: &str, state: ConditionState) {
        self.status.conditions.insert(name.to_string(), state);
        self.status.ready = self
            .status
            .conditions
            .values()
            .all(|s| matches!(s, ConditionState::True))
            && !self.status.conditions.is_empty();
    }

    /// Walk the broker through one reconcile pass. Returns the list of
    /// actions a controller would dispatch this tick. Idempotent — calling
    /// twice on the same state returns the same vector.
    pub fn reconcile(&mut self) -> Vec<ReconcileAction> {
        let mut actions = Vec::new();

        // Stage 1 — config validation. ConfigReady gates the rest.
        if self.spec.class.is_empty() {
            self.set_condition(
                "ConfigReady",
                ConditionState::False("missing class".to_string()),
            );
            return actions;
        }
        if self.spec.config_ref.is_none() && self.spec.class != "in-memory" {
            self.set_condition(
                "ConfigReady",
                ConditionState::False("config_ref required for non-IMC class".to_string()),
            );
            return actions;
        }
        self.set_condition("ConfigReady", ConditionState::True);

        // Stage 2 — channel + topic provision (per class).
        actions.push(ReconcileAction::EnsureChannel {
            class: self.spec.class.clone(),
            name: self.metadata.name.clone(),
        });
        if self.spec.class != "in-memory" {
            actions.push(ReconcileAction::EnsureTopic {
                class: self.spec.class.clone(),
                name: format!("knative-broker-{}", self.metadata.name),
            });
            self.set_condition("TopicReady", ConditionState::True);
        }

        // Stage 3 — ingress (HTTP endpoint that accepts events).
        let sink = format!(
            "http://broker-ingress.knative-eventing.svc.cluster.local/{}/{}",
            self.metadata.namespace, self.metadata.name
        );
        actions.push(ReconcileAction::EnsureIngress {
            broker: self.metadata.name.clone(),
            sink: sink.clone(),
        });
        self.status.address_url = Some(sink);
        self.set_condition("IngressReady", ConditionState::True);

        // Stage 4 — filters (one per trigger).
        for t in &self.triggers {
            actions.push(ReconcileAction::EnsureFilter {
                broker: self.metadata.name.clone(),
                trigger: t.metadata.name.clone(),
            });
        }
        self.set_condition("FilterReady", ConditionState::True);

        // Stage 5 — dead-letter sink.
        if let Some(d) = &self.spec.delivery {
            if let Some(dls) = &d.dead_letter_sink {
                actions.push(ReconcileAction::EnsureDeadLetter {
                    broker: self.metadata.name.clone(),
                    sink: dls.clone(),
                });
            }
        }
        self.set_condition("Addressable", ConditionState::True);
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_empty_class_marks_config_false() {
        let mut b = Broker::new("t", "br1", "");
        let actions = b.reconcile();
        assert!(actions.is_empty());
        assert!(matches!(
            b.status.conditions.get("ConfigReady"),
            Some(ConditionState::False(_))
        ));
        assert!(!b.status.ready);
    }

    #[test]
    fn reconcile_inmemory_class_no_config_ref_passes_config() {
        let mut b = Broker::new("t", "br1", "in-memory");
        let actions = b.reconcile();
        assert_eq!(b.status.conditions["ConfigReady"], ConditionState::True);
        assert!(actions.iter().any(
            |a| matches!(a, ReconcileAction::EnsureChannel { class, .. } if class == "in-memory")
        ));
        // in-memory has no separate topic
        assert!(
            actions
                .iter()
                .all(|a| !matches!(a, ReconcileAction::EnsureTopic { .. }))
        );
    }

    #[test]
    fn reconcile_kafka_requires_config_ref() {
        let mut b = Broker::new("t", "br1", "kafka");
        let actions = b.reconcile();
        assert!(actions.is_empty());
        assert!(matches!(
            b.status.conditions["ConfigReady"],
            ConditionState::False(_)
        ));
    }

    #[test]
    fn reconcile_kafka_with_config_ready_emits_topic() {
        let mut b = Broker::new("t", "br1", "kafka");
        b.spec.config_ref = Some("kafka-broker-config".to_string());
        let actions = b.reconcile();
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, ReconcileAction::EnsureTopic { .. }))
        );
        assert_eq!(b.status.conditions["TopicReady"], ConditionState::True);
    }

    #[test]
    fn reconcile_sets_address_url() {
        let mut b = Broker::new("t", "br1", "in-memory");
        b.metadata.namespace = "default".to_string();
        let _ = b.reconcile();
        let url = b.status.address_url.unwrap();
        assert!(url.contains("/default/br1"));
        assert!(url.starts_with("http://broker-ingress"));
    }

    #[test]
    fn reconcile_emits_filter_per_trigger() {
        let mut b = Broker::new("t", "br1", "in-memory");
        b.triggers.push({
            let mut tr = Trigger::new("t", "br1");
            tr.metadata.name = "t1".to_string();
            tr
        });
        b.triggers.push({
            let mut tr = Trigger::new("t", "br1");
            tr.metadata.name = "t2".to_string();
            tr
        });
        let actions = b.reconcile();
        let filters: Vec<&ReconcileAction> = actions
            .iter()
            .filter(|a| matches!(a, ReconcileAction::EnsureFilter { .. }))
            .collect();
        assert_eq!(filters.len(), 2);
    }

    #[test]
    fn reconcile_dead_letter_sink_emitted_when_present() {
        let mut b = Broker::new("t", "br1", "in-memory");
        b.spec.delivery = Some(DeliverySpec {
            retry: 3,
            backoff_policy: "exponential".to_string(),
            backoff_delay_ms: 1000,
            dead_letter_sink: Some("http://dls/in".to_string()),
        });
        let actions = b.reconcile();
        assert!(actions.iter().any(|a| matches!(a, ReconcileAction::EnsureDeadLetter { sink, .. } if sink == "http://dls/in")));
    }

    #[test]
    fn reconcile_idempotent_when_run_twice() {
        let mut b = Broker::new("t", "br1", "in-memory");
        let a1 = b.reconcile();
        let a2 = b.reconcile();
        assert_eq!(a1, a2);
        assert!(b.status.ready);
    }

    #[test]
    fn ready_flips_true_only_when_all_conditions_true() {
        let mut b = Broker::new("t", "br1", "in-memory");
        b.set_condition("ConfigReady", ConditionState::True);
        assert!(b.status.ready);
        b.set_condition("FilterReady", ConditionState::False("x".to_string()));
        assert!(!b.status.ready);
    }
}
