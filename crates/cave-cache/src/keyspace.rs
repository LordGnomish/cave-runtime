// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Keyspace notifications — Redis-compatible event broadcasting.

use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct KeyspaceEvent {
    pub db: usize,
    pub event: String,
    pub key: Vec<u8>,
}

/// Keyspace notification router.
/// Listens to the broadcast channel and re-publishes to the pub/sub registry.
pub async fn keyspace_notification_task(
    mut rx: broadcast::Receiver<KeyspaceEvent>,
    pubsub: std::sync::Arc<tokio::sync::RwLock<crate::db::PubSubRegistry>>,
    config: std::sync::Arc<tokio::sync::RwLock<crate::config::Config>>,
) {
    loop {
        match rx.recv().await {
            Ok(event) => {
                let flags_str = {
                    let cfg = config.read().await;
                    cfg.notify_keyspace_events.clone()
                };
                let flags = crate::config::NotifyFlags::from_str(&flags_str);

                if !flags.any_enabled() {
                    continue;
                }

                let event_type = &event.event;
                let should_notify = flags.all
                    || match event_type.as_str() {
                        "set" | "get" | "del" | "expire" | "rename" | "lpush" | "rpush"
                        | "lpop" | "rpop" | "incr" | "decr" | "append" | "getset" => flags.generic || flags.string || flags.list,
                        "expired" => flags.expired,
                        "evicted" => flags.evicted,
                        _ => flags.generic,
                    };

                if !should_notify {
                    continue;
                }

                let registry = pubsub.read().await;

                // Keyspace channel: __keyspace@{db}__:{key}
                if flags.keyspace || flags.all {
                    let channel = format!("__keyspace@{}__:{}", event.db, String::from_utf8_lossy(&event.key));
                    registry.publish(channel.as_bytes(), event.event.as_bytes());
                }

                // Keyevent channel: __keyevent@{db}__:{event}
                if flags.keyevent || flags.all {
                    let channel = format!("__keyevent@{}__:{}", event.db, event.event);
                    registry.publish(channel.as_bytes(), &event.key);
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Keyspace notification lag: {} events dropped", n);
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
