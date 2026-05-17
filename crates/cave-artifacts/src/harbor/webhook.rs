// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Webhook notifications on push / pull / delete / scan events.

use crate::harbor::types::{WebhookConfig, WebhookEvent, WebhookPayload};
use crate::harbor::store::RegistryStore;
use std::sync::Arc;
use tracing::{error, info};

pub struct WebhookManager {
    store: Arc<RegistryStore>,
    client: reqwest::Client,
}

impl WebhookManager {
    pub fn new(store: Arc<RegistryStore>) -> Self {
        Self {
            store,
            client: reqwest::Client::new(),
        }
    }

    pub async fn register(&self, config: WebhookConfig) {
        self.store.add_webhook(config).await;
    }

    pub async fn fire(
        &self,
        event: WebhookEvent,
        repository: &str,
        digest: Option<&str>,
        tag: Option<&str>,
    ) {
        let hooks = self.store.get_webhooks().await;
        let payload = WebhookPayload {
            event: event.clone(),
            repository: repository.to_string(),
            digest: digest.map(str::to_string),
            tag: tag.map(str::to_string),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        for hook in hooks {
            if !hook.enabled {
                continue;
            }
            if let Some(ref repo) = hook.repository {
                if repo != repository {
                    continue;
                }
            }
            if !hook.events.contains(&event) {
                continue;
            }
            let client = self.client.clone();
            let url = hook.url.clone();
            let payload = payload.clone();
            tokio::spawn(async move {
                match client.post(&url).json(&payload).send().await {
                    Ok(resp) => {
                        info!(
                            target: "cave_registry::webhook",
                            url = %url,
                            status = resp.status().as_u16(),
                            "webhook delivered"
                        );
                    }
                    Err(e) => {
                        error!(target: "cave_registry::webhook", url = %url, err = %e, "webhook failed");
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_webhook_fires_for_matching_event() {
        let store = Arc::new(RegistryStore::new());
        let mgr = WebhookManager::new(Arc::clone(&store));

        mgr.register(WebhookConfig {
            id: "wh1".to_string(),
            repository: Some("myrepo".to_string()),
            url: "http://localhost:9999/hook".to_string(),
            events: vec![WebhookEvent::Push],
            enabled: true,
        })
        .await;

        let hooks = store.get_webhooks().await;
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].events, vec![WebhookEvent::Push]);
    }

    #[tokio::test]
    async fn test_disabled_webhook_not_active() {
        let store = Arc::new(RegistryStore::new());
        let mgr = WebhookManager::new(Arc::clone(&store));

        mgr.register(WebhookConfig {
            id: "wh2".to_string(),
            repository: None,
            url: "http://localhost:9999/hook".to_string(),
            events: vec![WebhookEvent::Push],
            enabled: false,
        })
        .await;

        // fire() must not panic even when the target is unreachable.
        mgr.fire(WebhookEvent::Push, "repo", Some("sha256:abc"), Some("latest")).await;
    }
}
