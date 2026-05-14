// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Knative Eventing — Source/Sink/Channel/Subscription/Trigger primitives.
//! upstream: knative/eventing v1.18.x — pkg/apis/{sources,messaging,eventing}/v1/

use crate::meta::ObjectMeta;
use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct EventingSource {
    pub metadata: ObjectMeta,
    pub spec: EventingSourceSpec,
    pub status: EventingSourceStatus,
}

#[derive(Default, Debug, Clone)]
pub struct EventingSourceSpec {
    pub sink: Option<String>,
    pub ce_overrides: HashMap<String, String>,
}

#[derive(Default, Debug, Clone)]
pub struct EventingSourceStatus {
    pub sinkURI: Option<String>,
    pub ceAttributes: Vec<String>,
    pub observed_generation: i64,
}

#[derive(Default, Debug, Clone)]
pub struct EventingSink {
    pub metadata: ObjectMeta,
    pub spec: EventingSinkSpec,
    pub status: EventingSinkStatus,
}

#[derive(Default, Debug, Clone)]
pub struct EventingSinkSpec {
    pub destination: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct EventingSinkStatus {
    pub address_url: Option<String>,
    pub observed_generation: i64,
}

impl EventingSource {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: EventingSourceSpec::default(),
            status: EventingSourceStatus::default(),
        }
    }

    /// Drop the source — clears the resolved sink URI but keeps the spec.
    pub fn scale_to_zero(&mut self) {
        self.status.sinkURI = None;
    }

    /// Resolve the sink URI from the spec.sink. Real upstream would walk a Destination CR;
    /// the in-memory port treats the spec.sink string as the URL.
    pub fn resolve_sink(&mut self) -> Option<&str> {
        if let Some(ref s) = self.spec.sink {
            self.status.sinkURI = Some(s.clone());
        }
        self.status.sinkURI.as_deref()
    }

    pub fn add_ce_override(&mut self, key: &str, value: &str) {
        self.spec.ce_overrides.insert(key.to_string(), value.to_string());
    }
}

impl EventingSink {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: EventingSinkSpec::default(),
            status: EventingSinkStatus::default(),
        }
    }

    pub fn resolve_address(&mut self) {
        self.status.address_url = self.spec.destination.clone();
    }
}

// ─────────────────────────────────────────────────────────────
// Channel + Subscription (in-memory channel — like the IMC default)
// ─────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct Channel {
    pub metadata: ObjectMeta,
    pub subscribers: Vec<Subscription>,
}

#[derive(Default, Debug, Clone)]
pub struct Subscription {
    pub uid: String,
    pub subscriber_uri: String,
    pub reply_uri: Option<String>,
}

impl Channel {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            subscribers: Vec::new(),
        }
    }

    pub fn subscribe(&mut self, sub: Subscription) {
        self.subscribers.push(sub);
    }

    pub fn unsubscribe(&mut self, uid: &str) {
        self.subscribers.retain(|s| s.uid != uid);
    }

    /// Distribute an event to every subscriber. Returns the URIs that should receive it.
    pub fn fanout(&self) -> Vec<String> {
        self.subscribers.iter().map(|s| s.subscriber_uri.clone()).collect()
    }
}

// ─────────────────────────────────────────────────────────────
// Trigger — broker-side filter
// ─────────────────────────────────────────────────────────────

#[derive(Default, Debug, Clone)]
pub struct Trigger {
    pub metadata: ObjectMeta,
    pub broker: String,
    pub filter: TriggerFilter,
    pub subscriber_uri: String,
}

#[derive(Default, Debug, Clone)]
pub struct TriggerFilter {
    /// Exact-match attributes on the CloudEvent (e.g. {"type": "com.example.foo"}).
    pub attributes: HashMap<String, String>,
}

impl Trigger {
    pub fn new(tenant_id: &str, broker: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            broker: broker.to_string(),
            filter: TriggerFilter::default(),
            subscriber_uri: String::new(),
        }
    }

    /// Match a CloudEvent against the trigger's filter (exact attribute match).
    pub fn matches(&self, event_attrs: &HashMap<String, String>) -> bool {
        if self.filter.attributes.is_empty() {
            return true;
        }
        self.filter
            .attributes
            .iter()
            .all(|(k, v)| event_attrs.get(k).map(|av| av == v).unwrap_or(false))
    }
}
