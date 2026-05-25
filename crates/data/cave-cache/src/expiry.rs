// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::engine::CacheEngine;
use crate::types::{CacheResult, CacheValue};

impl CacheEngine {
    pub fn expire(&self, key: &str, secs: u64) -> CacheResult<bool> {
        let mut store = self.store.lock().unwrap();
        // Check expiry first without holding ref
        let expired = store.get(key).map(|e| Self::is_expired(e)).unwrap_or(false);
        if expired {
            store.remove(key);
        }
        match store.get_mut(key) {
            None => Ok(false),
            Some(entry) => {
                entry.expires_at = Some(Instant::now() + Duration::from_secs(secs));
                entry.version += 1;
                Ok(true)
            }
        }
    }

    pub fn pexpire(&self, key: &str, ms: u64) -> CacheResult<bool> {
        let mut store = self.store.lock().unwrap();
        let expired = store.get(key).map(|e| Self::is_expired(e)).unwrap_or(false);
        if expired {
            store.remove(key);
        }
        match store.get_mut(key) {
            None => Ok(false),
            Some(entry) => {
                entry.expires_at = Some(Instant::now() + Duration::from_millis(ms));
                entry.version += 1;
                Ok(true)
            }
        }
    }

    pub fn ttl(&self, key: &str) -> CacheResult<i64> {
        let mut store = self.store.lock().unwrap();
        let expired = store.get(key).map(|e| Self::is_expired(e)).unwrap_or(false);
        if expired {
            store.remove(key);
            return Ok(-2);
        }
        match store.get(key) {
            None => Ok(-2),
            Some(entry) => {
                match entry.expires_at {
                    None => Ok(-1),
                    Some(t) => {
                        let now = Instant::now();
                        if t <= now {
                            Ok(0)
                        } else {
                            Ok((t - now).as_secs() as i64)
                        }
                    }
                }
            }
        }
    }

    pub fn persist(&self, key: &str) -> CacheResult<bool> {
        let mut store = self.store.lock().unwrap();
        let expired = store.get(key).map(|e| Self::is_expired(e)).unwrap_or(false);
        if expired {
            store.remove(key);
            return Ok(false);
        }
        match store.get_mut(key) {
            None => Ok(false),
            Some(entry) => {
                if entry.expires_at.is_some() {
                    entry.expires_at = None;
                    entry.version += 1;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    pub fn exists(&self, keys: &[&str]) -> usize {
        let store = self.store.lock().unwrap();
        keys.iter()
            .filter(|key| {
                match store.get(**key) {
                    None => false,
                    Some(e) => !Self::is_expired(e),
                }
            })
            .count()
    }

    pub fn del(&self, keys: &[&str]) -> usize {
        let mut store = self.store.lock().unwrap();
        let mut removed = 0;
        for key in keys {
            if store.remove(*key).is_some() {
                removed += 1;
            }
        }
        removed
    }

    pub fn type_of(&self, key: &str) -> Option<&'static str> {
        let mut store = self.store.lock().unwrap();
        let expired = store.get(key).map(|e| Self::is_expired(e)).unwrap_or(false);
        if expired {
            store.remove(key);
            return None;
        }
        store.get(key).map(|entry| match &entry.value {
            CacheValue::String(_) => "string",
            CacheValue::List(_) => "list",
            CacheValue::Set(_) => "set",
            CacheValue::ZSet(_) => "zset",
            CacheValue::Hash(_) => "hash",
            CacheValue::Stream(_) => "stream",
        })
    }
}

pub fn start_expiry_task(engine: Arc<CacheEngine>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let expired_keys: Vec<String> = {
                let store = engine.store.lock().unwrap();
                store
                    .iter()
                    .filter(|(_, e)| CacheEngine::is_expired(e))
                    .map(|(k, _)| k.clone())
                    .collect()
            };
            if !expired_keys.is_empty() {
                let mut store = engine.store.lock().unwrap();
                for key in expired_keys {
                    store.remove(&key);
                }
            }
        }
    })
}
