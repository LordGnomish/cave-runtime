// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::collections::HashMap;
use crate::engine::CacheEngine;
use crate::types::{CacheEntry, CacheError, CacheResult, CacheValue};

impl CacheEngine {
    pub fn hget(&self, key: &str, field: &[u8]) -> CacheResult<Option<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(None),
            Some(entry) => match &entry.value {
                CacheValue::Hash(map) => Ok(map.get(field).cloned()),
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn hset(&self, key: &str, fields: &[(&[u8], Vec<u8>)]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: CacheValue::Hash(HashMap::new()),
            expires_at: None,
            version: 0,
        });
        entry.version += 1;
        match &mut entry.value {
            CacheValue::Hash(map) => {
                let mut added = 0;
                for (field, value) in fields {
                    if !map.contains_key(*field) {
                        added += 1;
                    }
                    map.insert(field.to_vec(), value.clone());
                }
                Ok(added)
            }
            _ => Err(CacheError::WrongType),
        }
    }

    pub fn hdel(&self, key: &str, fields: &[&[u8]]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        match store.get_mut(key) {
            None => Ok(0),
            Some(entry) => {
                if Self::is_expired(entry) {
                    store.remove(key);
                    return Ok(0);
                }
                match &mut entry.value {
                    CacheValue::Hash(map) => {
                        let mut removed = 0;
                        for field in fields {
                            if map.remove(*field).is_some() {
                                removed += 1;
                            }
                        }
                        Ok(removed)
                    }
                    _ => Err(CacheError::WrongType),
                }
            }
        }
    }

    pub fn hgetall(&self, key: &str) -> CacheResult<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(vec![]),
            Some(entry) => match &entry.value {
                CacheValue::Hash(map) => Ok(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn hincrby(&self, key: &str, field: &[u8], delta: i64) -> CacheResult<i64> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: CacheValue::Hash(HashMap::new()),
            expires_at: None,
            version: 0,
        });
        entry.version += 1;
        match &mut entry.value {
            CacheValue::Hash(map) => {
                let current: i64 = match map.get(field) {
                    None => 0,
                    Some(v) => {
                        let s = std::str::from_utf8(v)
                            .map_err(|e| CacheError::Parse(e.to_string()))?;
                        s.trim().parse::<i64>().map_err(|e| CacheError::Parse(e.to_string()))?
                    }
                };
                let new_val = current + delta;
                map.insert(field.to_vec(), new_val.to_string().into_bytes());
                Ok(new_val)
            }
            _ => Err(CacheError::WrongType),
        }
    }

    pub fn hexists(&self, key: &str, field: &[u8]) -> CacheResult<bool> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(false),
            Some(entry) => match &entry.value {
                CacheValue::Hash(map) => Ok(map.contains_key(field)),
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn hlen(&self, key: &str) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(0),
            Some(entry) => match &entry.value {
                CacheValue::Hash(map) => Ok(map.len()),
                _ => Err(CacheError::WrongType),
            },
        }
    }
}
