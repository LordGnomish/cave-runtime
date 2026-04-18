//! Knative Eventing — brokers, triggers, sources, channels, subscriptions, CloudEvents.

use crate::error::{KnativeError, KnativeResult};
use crate::models::{
    Addressable, Broker, BrokerClass, BrokerConfig, BrokerStatus, Channel, ChannelStatus,
    ChannelType, CloudEvent, CreateBrokerRequest, CreateChannelRequest, CreateSourceRequest,
    CreateSubscriptionRequest, CreateTriggerRequest, DeliverySpec, EventSource, EventSourceSpec,
    SendEventRequest, SourceStatus, SourceType, Subscription, SubscriptionStatus, Trigger,
    TriggerFilter, TriggerStatus, BrokerAddress,
};
use chrono::Utc;
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

const MAX_BROKER_EVENTS: usize = 10_000;
const MAX_TRIGGER_DELIVERIES: usize = 1_000;

pub struct EventingStore {
    brokers: DashMap<String, Broker>,
    triggers: DashMap<String, Trigger>,
    sources: DashMap<String, EventSource>,
    channels: DashMap<String, Channel>,
    subscriptions: DashMap<String, Subscription>,
    /// Recent events per broker (namespace/broker → ring buffer)
    broker_events: DashMap<String, Arc<RwLock<VecDeque<CloudEvent>>>>,
    total_events: Arc<AtomicU64>,
}

impl EventingStore {
    pub fn new() -> Self {
        Self {
            brokers: DashMap::new(),
            triggers: DashMap::new(),
            sources: DashMap::new(),
            channels: DashMap::new(),
            subscriptions: DashMap::new(),
            broker_events: DashMap::new(),
            total_events: Arc::new(AtomicU64::new(0)),
        }
    }

    fn ns_key(namespace: &str, name: &str) -> String {
        format!("{namespace}/{name}")
    }

    // ── Brokers ───────────────────────────────────────────────────────────────

    pub fn create_broker(&self, req: CreateBrokerRequest) -> KnativeResult<Broker> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.brokers.contains_key(&key) {
            return Err(KnativeError::Validation(format!("Broker {key} already exists")));
        }
        if req.name.is_empty() {
            return Err(KnativeError::Validation("broker name is required".into()));
        }
        let broker = Broker {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            broker_class: req.broker_class.unwrap_or(BrokerClass::MTChannelBasedBroker),
            config: BrokerConfig { delivery: req.delivery },
            status: BrokerStatus::Ready,
            address: Some(BrokerAddress {
                url: format!(
                    "http://broker-ingress.knative-eventing.svc.cluster.local/{}/{}",
                    req.namespace, req.name
                ),
                audience: None,
            }),
            event_count: 0,
            created_at: Utc::now(),
        };
        self.broker_events
            .insert(key.clone(), Arc::new(RwLock::new(VecDeque::new())));
        self.brokers.insert(key, broker.clone());
        info!(broker = %req.name, namespace = %req.namespace, "knative broker created");
        Ok(broker)
    }

    pub fn get_broker(&self, namespace: &str, name: &str) -> KnativeResult<Broker> {
        let key = Self::ns_key(namespace, name);
        self.brokers
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| KnativeError::BrokerNotFound(key))
    }

    pub fn list_brokers(&self, namespace: &str) -> Vec<Broker> {
        self.brokers
            .iter()
            .filter(|r| r.value().namespace == namespace)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_broker(&self, namespace: &str, name: &str) -> KnativeResult<()> {
        let key = Self::ns_key(namespace, name);
        self.brokers
            .remove(&key)
            .ok_or_else(|| KnativeError::BrokerNotFound(key.clone()))?;
        self.broker_events.remove(&key);
        // Remove triggers pointing to this broker
        self.triggers.retain(|_, t| !(t.broker == name && t.namespace == namespace));
        Ok(())
    }

    /// Ingest a CloudEvent into a broker, fan out to matching triggers.
    pub fn send_event(
        &self,
        namespace: &str,
        broker_name: &str,
        req: SendEventRequest,
    ) -> KnativeResult<CloudEvent> {
        let key = Self::ns_key(namespace, broker_name);
        if !self.brokers.contains_key(&key) {
            return Err(KnativeError::BrokerNotFound(key));
        }

        let mut event = CloudEvent::new(req.event_type, req.source, req.data);
        if let Some(ext) = req.extensions {
            event.extensions = ext;
        }

        // Store event in broker ring buffer
        if let Some(buf) = self.broker_events.get(&key) {
            let mut q = buf.write();
            q.push_back(event.clone());
            if q.len() > MAX_BROKER_EVENTS {
                q.pop_front();
            }
        }

        // Update broker event count
        if let Some(mut broker) = self.brokers.get_mut(&key) {
            broker.event_count += 1;
        }
        self.total_events.fetch_add(1, Ordering::Relaxed);

        // Fan out to matching triggers
        let matching_triggers: Vec<_> = self
            .triggers
            .iter()
            .filter(|t| t.value().broker == broker_name && t.value().namespace == namespace)
            .filter(|t| self.event_matches_filter(&event, &t.value().filter))
            .map(|t| t.value().clone())
            .collect();

        for trigger in matching_triggers {
            if let Some(mut t) = self.triggers.get_mut(&Self::ns_key(namespace, &trigger.name)) {
                t.event_count += 1;
            }
        }

        Ok(event)
    }

    fn event_matches_filter(&self, event: &CloudEvent, filter: &TriggerFilter) -> bool {
        // Check attribute filters (exact match)
        for (attr, value) in &filter.attributes {
            let event_val = match attr.as_str() {
                "type" => event.event_type.clone(),
                "source" => event.source.clone(),
                "subject" => event.subject.clone().unwrap_or_default(),
                _ => event
                    .extensions
                    .get(attr)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            };
            if &event_val != value {
                return false;
            }
        }
        true
    }

    pub fn get_broker_events(&self, namespace: &str, broker_name: &str, limit: usize) -> Vec<CloudEvent> {
        let key = Self::ns_key(namespace, broker_name);
        self.broker_events
            .get(&key)
            .map(|buf| {
                let q = buf.read();
                q.iter().rev().take(limit).cloned().collect()
            })
            .unwrap_or_default()
    }

    // ── Triggers ──────────────────────────────────────────────────────────────

    pub fn create_trigger(&self, req: CreateTriggerRequest) -> KnativeResult<Trigger> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.triggers.contains_key(&key) {
            return Err(KnativeError::Validation(format!("Trigger {key} already exists")));
        }
        // Verify broker exists
        let broker_key = Self::ns_key(&req.namespace, &req.broker);
        if !self.brokers.contains_key(&broker_key) {
            return Err(KnativeError::BrokerNotFound(broker_key));
        }
        let trigger = Trigger {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            broker: req.broker,
            filter: req.filter.unwrap_or_default(),
            subscriber: Addressable { uri: req.subscriber_uri },
            delivery: req.delivery,
            status: TriggerStatus::Ready,
            event_count: 0,
            created_at: Utc::now(),
        };
        self.triggers.insert(key, trigger.clone());
        Ok(trigger)
    }

    pub fn get_trigger(&self, namespace: &str, name: &str) -> KnativeResult<Trigger> {
        let key = Self::ns_key(namespace, name);
        self.triggers
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| KnativeError::TriggerNotFound(key))
    }

    pub fn list_triggers(&self, namespace: &str) -> Vec<Trigger> {
        self.triggers
            .iter()
            .filter(|r| r.value().namespace == namespace)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_trigger(&self, namespace: &str, name: &str) -> KnativeResult<()> {
        let key = Self::ns_key(namespace, name);
        self.triggers
            .remove(&key)
            .ok_or_else(|| KnativeError::TriggerNotFound(key))?;
        Ok(())
    }

    // ── Event Sources ─────────────────────────────────────────────────────────

    pub fn create_source(&self, req: CreateSourceRequest) -> KnativeResult<EventSource> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.sources.contains_key(&key) {
            return Err(KnativeError::Validation(format!("Source {key} already exists")));
        }
        let source = EventSource {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            source_type: req.source_type,
            spec: req.spec,
            sink: Addressable { uri: req.sink_uri },
            status: SourceStatus::Ready,
            event_count: 0,
            created_at: Utc::now(),
        };
        self.sources.insert(key, source.clone());
        Ok(source)
    }

    pub fn get_source(&self, namespace: &str, name: &str) -> KnativeResult<EventSource> {
        let key = Self::ns_key(namespace, name);
        self.sources
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| KnativeError::SourceNotFound(key))
    }

    pub fn list_sources(&self, namespace: &str) -> Vec<EventSource> {
        self.sources
            .iter()
            .filter(|r| r.value().namespace == namespace)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_source(&self, namespace: &str, name: &str) -> KnativeResult<()> {
        let key = Self::ns_key(namespace, name);
        self.sources
            .remove(&key)
            .ok_or_else(|| KnativeError::SourceNotFound(key))?;
        Ok(())
    }

    // ── Channels ──────────────────────────────────────────────────────────────

    pub fn create_channel(&self, req: CreateChannelRequest) -> KnativeResult<Channel> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.channels.contains_key(&key) {
            return Err(KnativeError::Validation(format!("Channel {key} already exists")));
        }
        let channel = Channel {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            channel_type: req.channel_type.unwrap_or(ChannelType::InMemoryChannel),
            status: ChannelStatus::Ready,
            address: Some(format!(
                "http://{}.{}.svc.cluster.local",
                req.name, req.namespace
            )),
            event_count: 0,
            subscriber_count: 0,
            created_at: Utc::now(),
        };
        self.channels.insert(key, channel.clone());
        Ok(channel)
    }

    pub fn get_channel(&self, namespace: &str, name: &str) -> KnativeResult<Channel> {
        let key = Self::ns_key(namespace, name);
        self.channels
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| KnativeError::ChannelNotFound(key))
    }

    pub fn list_channels(&self, namespace: &str) -> Vec<Channel> {
        self.channels
            .iter()
            .filter(|r| r.value().namespace == namespace)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_channel(&self, namespace: &str, name: &str) -> KnativeResult<()> {
        let key = Self::ns_key(namespace, name);
        self.channels
            .remove(&key)
            .ok_or_else(|| KnativeError::ChannelNotFound(key))?;
        // Remove subscriptions for this channel
        self.subscriptions.retain(|_, s| !(s.channel == name && s.namespace == namespace));
        Ok(())
    }

    // ── Subscriptions ─────────────────────────────────────────────────────────

    pub fn create_subscription(&self, req: CreateSubscriptionRequest) -> KnativeResult<Subscription> {
        let key = Self::ns_key(&req.namespace, &req.name);
        if self.subscriptions.contains_key(&key) {
            return Err(KnativeError::Validation(format!("Subscription {key} already exists")));
        }
        let channel_key = Self::ns_key(&req.namespace, &req.channel);
        if !self.channels.contains_key(&channel_key) {
            return Err(KnativeError::ChannelNotFound(channel_key));
        }
        let sub = Subscription {
            id: Uuid::new_v4(),
            name: req.name.clone(),
            namespace: req.namespace.clone(),
            channel: req.channel.clone(),
            subscriber: req.subscriber_uri.map(|uri| Addressable { uri }),
            reply: req.reply_uri.map(|uri| Addressable { uri }),
            delivery: req.delivery,
            status: SubscriptionStatus::Ready,
            created_at: Utc::now(),
        };
        // Update channel subscriber count
        if let Some(mut ch) = self.channels.get_mut(&channel_key) {
            ch.subscriber_count += 1;
        }
        self.subscriptions.insert(key, sub.clone());
        Ok(sub)
    }

    pub fn get_subscription(&self, namespace: &str, name: &str) -> KnativeResult<Subscription> {
        let key = Self::ns_key(namespace, name);
        self.subscriptions
            .get(&key)
            .map(|r| r.clone())
            .ok_or_else(|| KnativeError::SubscriptionNotFound(key))
    }

    pub fn list_subscriptions(&self, namespace: &str) -> Vec<Subscription> {
        self.subscriptions
            .iter()
            .filter(|r| r.value().namespace == namespace)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn delete_subscription(&self, namespace: &str, name: &str) -> KnativeResult<()> {
        let key = Self::ns_key(namespace, name);
        let sub = self.subscriptions
            .remove(&key)
            .ok_or_else(|| KnativeError::SubscriptionNotFound(key))?
            .1;
        // Decrement channel subscriber count
        let channel_key = Self::ns_key(namespace, &sub.channel);
        if let Some(mut ch) = self.channels.get_mut(&channel_key) {
            if ch.subscriber_count > 0 {
                ch.subscriber_count -= 1;
            }
        }
        Ok(())
    }

    // ── Stats ─────────────────────────────────────────────────────────────────

    pub fn total_events(&self) -> u64 {
        self.total_events.load(Ordering::Relaxed)
    }
}

impl Default for EventingStore {
    fn default() -> Self {
        Self::new()
    }
}
