// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use crate::types::*;

#[derive(Clone, Debug)]
pub struct PubSubMessage {
    pub channel: String,
    pub message: Vec<u8>,
}

#[allow(dead_code)]
pub struct CacheEngine {
    pub(crate) store: Arc<Mutex<HashMap<String, CacheEntry>>>,
    pub(crate) pubsub_tx: broadcast::Sender<PubSubMessage>,
    pub(crate) scripts: Arc<Mutex<HashMap<String, String>>>, // sha -> script
    pub(crate) groups: Arc<Mutex<HashMap<String, HashMap<String, String>>>>, // key -> (group -> last_id)
}

impl CacheEngine {
    pub fn new() -> Self {
        let (pubsub_tx, _) = broadcast::channel(1024);
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
            pubsub_tx,
            scripts: Arc::new(Mutex::new(HashMap::new())),
            groups: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn is_expired(entry: &CacheEntry) -> bool {
        entry.expires_at.map(|t| t < std::time::Instant::now()).unwrap_or(false)
    }

    /// Get with expiry check — removes expired keys and returns None for them.
    pub(crate) fn get_entry<'a>(
        store: &'a mut HashMap<String, CacheEntry>,
        key: &str,
    ) -> Option<&'a CacheEntry> {
        let expired = store
            .get(key)
            .map(|e| Self::is_expired(e))
            .unwrap_or(false);
        if expired {
            store.remove(key);
            return None;
        }
        store.get(key)
    }

    /// Mutable version of get_entry.
    #[allow(dead_code)]
    pub(crate) fn get_entry_mut<'a>(
        store: &'a mut HashMap<String, CacheEntry>,
        key: &str,
    ) -> Option<&'a mut CacheEntry> {
        let expired = store
            .get(key)
            .map(|e| Self::is_expired(e))
            .unwrap_or(false);
        if expired {
            store.remove(key);
            return None;
        }
        store.get_mut(key)
    }
}

impl Default for CacheEngine {
    fn default() -> Self {
        Self::new()
    }
}
