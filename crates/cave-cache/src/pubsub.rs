// SPDX-License-Identifier: AGPL-3.0-or-later
use tokio::sync::broadcast;
use crate::engine::{CacheEngine, PubSubMessage};

pub struct PubSubHandle {
    rx: broadcast::Receiver<PubSubMessage>,
    pub patterns: Vec<String>,
}

impl CacheEngine {
    pub fn subscribe(&self, channels: &[&str]) -> PubSubHandle {
        PubSubHandle {
            rx: self.pubsub_tx.subscribe(),
            patterns: channels.iter().map(|c| c.to_string()).collect(),
        }
    }

    pub fn psubscribe(&self, patterns: &[&str]) -> PubSubHandle {
        PubSubHandle {
            rx: self.pubsub_tx.subscribe(),
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
        }
    }

    pub fn publish(&self, channel: &str, message: Vec<u8>) -> usize {
        let msg = PubSubMessage {
            channel: channel.to_string(),
            message,
        };
        self.pubsub_tx.send(msg).unwrap_or(0)
    }
}

impl PubSubHandle {
    pub async fn recv(&mut self) -> Option<PubSubMessage> {
        loop {
            match self.rx.recv().await {
                Ok(msg) => {
                    // Check if channel matches any pattern/channel in our list
                    if self.patterns.iter().any(|p| {
                        // Simple glob matching: '*' matches everything, else exact match
                        if p.ends_with('*') {
                            msg.channel.starts_with(&p[..p.len() - 1])
                        } else {
                            msg.channel == *p
                        }
                    }) {
                        return Some(msg);
                    }
                    // No match — keep looping
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
