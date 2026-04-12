use tokio::sync::broadcast;
use crate::store::StoreEvent;
use crate::types::NotificationConfig;

pub struct NotificationDispatcher {
    sender: broadcast::Sender<StoreEvent>,
}

impl NotificationDispatcher {
    pub fn new(sender: broadcast::Sender<StoreEvent>) -> Self {
        Self { sender }
    }

    pub fn should_notify(config: &NotificationConfig, event_type: &str, key: &str) -> bool {
        for qc in &config.queue_configurations {
            for event_pattern in &qc.events {
                let event_matches = if event_pattern.ends_with('*') {
                    event_type.starts_with(&event_pattern[..event_pattern.len() - 1])
                } else {
                    event_type == event_pattern
                };
                if !event_matches {
                    continue;
                }
                // Check prefix filter
                let prefix_ok = match &qc.prefix_filter {
                    None => true,
                    Some(prefix) => key.starts_with(prefix.as_str()),
                };
                if prefix_ok {
                    return true;
                }
            }
        }
        false
    }

    pub fn dispatch(&self, event: StoreEvent) {
        // Best-effort: ignore if no receivers
        let _ = self.sender.send(event);
    }
}
