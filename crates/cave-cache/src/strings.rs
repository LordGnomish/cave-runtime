// SPDX-License-Identifier: AGPL-3.0-or-later
use std::time::{Duration, Instant};
use crate::engine::CacheEngine;
use crate::types::{CacheEntry, CacheError, CacheResult, CacheValue};

impl CacheEngine {
    pub fn get(&self, key: &str) -> CacheResult<Option<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(None),
            Some(entry) => match &entry.value {
                CacheValue::String(v) => Ok(Some(v.clone())),
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn set(&self, key: &str, value: Vec<u8>, ex: Option<Duration>) -> CacheResult<()> {
        let expires_at = ex.map(|d| Instant::now() + d);
        let mut store = self.store.lock().unwrap();
        let version = store.get(key).map(|e| e.version + 1).unwrap_or(1);
        store.insert(
            key.to_string(),
            CacheEntry {
                value: CacheValue::String(value),
                expires_at,
                version,
            },
        );
        Ok(())
    }

    pub fn mget(&self, keys: &[&str]) -> Vec<Option<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        keys.iter()
            .map(|key| {
                match Self::get_entry(&mut store, key) {
                    None => None,
                    Some(entry) => match &entry.value {
                        CacheValue::String(v) => Some(v.clone()),
                        _ => None,
                    },
                }
            })
            .collect()
    }

    pub fn mset(&self, pairs: &[(&str, Vec<u8>)]) -> CacheResult<()> {
        let mut store = self.store.lock().unwrap();
        for (key, value) in pairs {
            let version = store.get(*key).map(|e| e.version + 1).unwrap_or(1);
            store.insert(
                key.to_string(),
                CacheEntry {
                    value: CacheValue::String(value.clone()),
                    expires_at: None,
                    version,
                },
            );
        }
        Ok(())
    }

    pub fn incr(&self, key: &str) -> CacheResult<i64> {
        self.incrby(key, 1)
    }

    pub fn decr(&self, key: &str) -> CacheResult<i64> {
        self.incrby(key, -1)
    }

    pub fn incrby(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let mut store = self.store.lock().unwrap();
        let current: i64 = match Self::get_entry(&mut store, key) {
            None => 0,
            Some(entry) => match &entry.value {
                CacheValue::String(v) => {
                    let s = std::str::from_utf8(v)
                        .map_err(|e| CacheError::Parse(e.to_string()))?;
                    s.trim()
                        .parse::<i64>()
                        .map_err(|e| CacheError::Parse(e.to_string()))?
                }
                _ => return Err(CacheError::WrongType),
            },
        };
        let new_val = current + delta;
        let version = store.get(key).map(|e| e.version + 1).unwrap_or(1);
        store.insert(
            key.to_string(),
            CacheEntry {
                value: CacheValue::String(new_val.to_string().into_bytes()),
                expires_at: None,
                version,
            },
        );
        Ok(new_val)
    }

    pub fn append(&self, key: &str, value: Vec<u8>) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        let existing: Vec<u8> = match Self::get_entry(&mut store, key) {
            None => vec![],
            Some(entry) => match &entry.value {
                CacheValue::String(v) => v.clone(),
                _ => return Err(CacheError::WrongType),
            },
        };
        let mut new_val = existing;
        new_val.extend_from_slice(&value);
        let len = new_val.len();
        let version = store.get(key).map(|e| e.version + 1).unwrap_or(1);
        store.insert(
            key.to_string(),
            CacheEntry {
                value: CacheValue::String(new_val),
                expires_at: None,
                version,
            },
        );
        Ok(len)
    }
}
