// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Type-safe in-process pub/sub bus for fan-out events.
//!
//! Backed by `tokio::sync::broadcast`. Topic separation is provided by
//! constructing a separate `EventBus<T>` per event type — no string-keyed
//! dispatch, no dynamic typing.
//!
//! Used by cave-apiserver watch endpoints, cave-portal SSE streams, and the
//! Reflex Engine: each picks the same shared primitive instead of rolling
//! its own broadcast wrapper.
//!
//! Slow / disconnected subscribers receive `RecvError::Lagged` and may
//! resync. Capacity is fixed at construction; the producer never blocks.

use std::sync::Arc;
use thiserror::Error;
use tokio::sync::broadcast::{self, Sender};

#[derive(Debug, Error)]
pub enum EventBusError {
    #[error("no active subscribers")]
    NoSubscribers,
    #[error("subscriber lagged behind by {0} events")]
    Lagged(u64),
    #[error("bus closed")]
    Closed,
}

pub type EventBusResult<T> = Result<T, EventBusError>;

/// Cloneable, multi-producer / multi-consumer broadcast bus.
pub struct EventBus<T: Clone + Send + 'static> {
    tx: Sender<T>,
}

impl<T: Clone + Send + 'static> EventBus<T> {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// Subscribe; receivers see only events published *after* subscription.
    pub fn subscribe(&self) -> Subscription<T> {
        Subscription { rx: self.tx.subscribe() }
    }

    /// Publish an event. Returns the number of subscribers that received it.
    /// Returns `NoSubscribers` if no one is currently listening — the caller
    /// can choose to drop or buffer.
    pub fn publish(&self, event: T) -> EventBusResult<usize> {
        match self.tx.send(event) {
            Ok(n) => Ok(n),
            Err(_) => Err(EventBusError::NoSubscribers),
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Convert into a shareable Arc for cross-task / cross-router cloning.
    pub fn into_shared(self) -> Arc<Self> {
        Arc::new(self)
    }
}

impl<T: Clone + Send + 'static> Clone for EventBus<T> {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone() }
    }
}

pub struct Subscription<T: Clone + Send + 'static> {
    rx: broadcast::Receiver<T>,
}

impl<T: Clone + Send + 'static> Subscription<T> {
    pub async fn recv(&mut self) -> EventBusResult<T> {
        match self.rx.recv().await {
            Ok(v) => Ok(v),
            Err(broadcast::error::RecvError::Closed) => Err(EventBusError::Closed),
            Err(broadcast::error::RecvError::Lagged(n)) => Err(EventBusError::Lagged(n)),
        }
    }

    /// Non-blocking peek; returns `Ok(None)` if no event is currently
    /// queued, `Lagged` if the receiver fell behind, `Closed` if the bus is
    /// gone.
    pub fn try_recv(&mut self) -> EventBusResult<Option<T>> {
        match self.rx.try_recv() {
            Ok(v) => Ok(Some(v)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Closed) => Err(EventBusError::Closed),
            Err(broadcast::error::TryRecvError::Lagged(n)) => Err(EventBusError::Lagged(n)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fan_out_to_two_subscribers() {
        let bus: EventBus<u32> = EventBus::new(8);
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish(7).unwrap();
        assert_eq!(a.recv().await.unwrap(), 7);
        assert_eq!(b.recv().await.unwrap(), 7);
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_errors() {
        let bus: EventBus<&'static str> = EventBus::new(4);
        let err = bus.publish("hi").unwrap_err();
        assert!(matches!(err, EventBusError::NoSubscribers));
    }

    #[tokio::test]
    async fn lagged_subscriber_gets_lag_signal() {
        let bus: EventBus<u32> = EventBus::new(2);
        let mut sub = bus.subscribe();
        for i in 0..5 {
            let _ = bus.publish(i);
        }
        let err = sub.recv().await.unwrap_err();
        assert!(matches!(err, EventBusError::Lagged(_)));
    }

    #[tokio::test]
    async fn try_recv_empty_returns_none() {
        let bus: EventBus<()> = EventBus::new(4);
        let mut sub = bus.subscribe();
        assert!(matches!(sub.try_recv(), Ok(None)));
    }

    #[tokio::test]
    async fn subscriber_count_tracks_drops() {
        let bus: EventBus<u8> = EventBus::new(4);
        let s1 = bus.subscribe();
        let _s2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
        drop(s1);
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[tokio::test]
    async fn clone_shares_underlying_channel() {
        let bus: EventBus<u8> = EventBus::new(4);
        let bus2 = bus.clone();
        let mut sub = bus.subscribe();
        bus2.publish(99).unwrap();
        assert_eq!(sub.recv().await.unwrap(), 99);
    }
}
