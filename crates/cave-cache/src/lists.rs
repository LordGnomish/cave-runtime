// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::VecDeque;
use crate::engine::CacheEngine;
use crate::types::{CacheEntry, CacheError, CacheResult, CacheValue};

impl CacheEngine {
    pub fn lpush(&self, key: &str, values: &[Vec<u8>]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: CacheValue::List(VecDeque::new()),
            expires_at: None,
            version: 0,
        });
        entry.version += 1;
        match &mut entry.value {
            CacheValue::List(list) => {
                for v in values {
                    list.push_front(v.clone());
                }
                Ok(list.len())
            }
            _ => Err(CacheError::WrongType),
        }
    }

    pub fn rpush(&self, key: &str, values: &[Vec<u8>]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: CacheValue::List(VecDeque::new()),
            expires_at: None,
            version: 0,
        });
        entry.version += 1;
        match &mut entry.value {
            CacheValue::List(list) => {
                for v in values {
                    list.push_back(v.clone());
                }
                Ok(list.len())
            }
            _ => Err(CacheError::WrongType),
        }
    }

    pub fn lpop(&self, key: &str, count: usize) -> CacheResult<Vec<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match store.get_mut(key) {
            None => Ok(vec![]),
            Some(entry) => {
                if Self::is_expired(entry) {
                    store.remove(key);
                    return Ok(vec![]);
                }
                match &mut entry.value {
                    CacheValue::List(list) => {
                        let mut result = Vec::new();
                        for _ in 0..count {
                            match list.pop_front() {
                                Some(v) => result.push(v),
                                None => break,
                            }
                        }
                        Ok(result)
                    }
                    _ => Err(CacheError::WrongType),
                }
            }
        }
    }

    pub fn rpop(&self, key: &str, count: usize) -> CacheResult<Vec<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match store.get_mut(key) {
            None => Ok(vec![]),
            Some(entry) => {
                if Self::is_expired(entry) {
                    store.remove(key);
                    return Ok(vec![]);
                }
                match &mut entry.value {
                    CacheValue::List(list) => {
                        let mut result = Vec::new();
                        for _ in 0..count {
                            match list.pop_back() {
                                Some(v) => result.push(v),
                                None => break,
                            }
                        }
                        Ok(result)
                    }
                    _ => Err(CacheError::WrongType),
                }
            }
        }
    }

    pub fn lrange(&self, key: &str, start: i64, stop: i64) -> CacheResult<Vec<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(vec![]),
            Some(entry) => match &entry.value {
                CacheValue::List(list) => {
                    let len = list.len() as i64;
                    let start = if start < 0 {
                        (len + start).max(0) as usize
                    } else {
                        start as usize
                    };
                    let stop = if stop < 0 {
                        (len + stop).max(-1) as usize
                    } else {
                        stop.min(len - 1) as usize
                    };
                    if start > stop || start >= len as usize {
                        return Ok(vec![]);
                    }
                    Ok(list.iter().skip(start).take(stop - start + 1).cloned().collect())
                }
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn llen(&self, key: &str) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(0),
            Some(entry) => match &entry.value {
                CacheValue::List(list) => Ok(list.len()),
                _ => Err(CacheError::WrongType),
            },
        }
    }
}
