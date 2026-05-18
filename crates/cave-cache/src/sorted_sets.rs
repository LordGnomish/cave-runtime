// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::engine::CacheEngine;
use crate::types::{CacheEntry, CacheError, CacheResult, CacheValue};

impl CacheEngine {
    pub fn zadd(&self, key: &str, members: &[(f64, Vec<u8>)]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: CacheValue::ZSet(vec![]),
            expires_at: None,
            version: 0,
        });
        entry.version += 1;
        match &mut entry.value {
            CacheValue::ZSet(zset) => {
                let mut added = 0;
                for (score, member) in members {
                    // Check if member already exists
                    if let Some(pos) = zset.iter().position(|(m, _)| m == member) {
                        zset[pos].1 = *score;
                    } else {
                        zset.push((member.clone(), *score));
                        added += 1;
                    }
                }
                // Keep sorted by score
                zset.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                Ok(added)
            }
            _ => Err(CacheError::WrongType),
        }
    }

    pub fn zrem(&self, key: &str, members: &[Vec<u8>]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        match store.get_mut(key) {
            None => Ok(0),
            Some(entry) => {
                if Self::is_expired(entry) {
                    store.remove(key);
                    return Ok(0);
                }
                match &mut entry.value {
                    CacheValue::ZSet(zset) => {
                        let before = zset.len();
                        zset.retain(|(m, _)| !members.contains(m));
                        Ok(before - zset.len())
                    }
                    _ => Err(CacheError::WrongType),
                }
            }
        }
    }

    fn resolve_index(len: usize, idx: i64) -> usize {
        if idx < 0 {
            let abs = (-idx) as usize;
            if abs > len { 0 } else { len - abs }
        } else {
            idx as usize
        }
    }

    pub fn zrange(&self, key: &str, start: i64, stop: i64, rev: bool) -> CacheResult<Vec<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(vec![]),
            Some(entry) => match &entry.value {
                CacheValue::ZSet(zset) => {
                    let len = zset.len();
                    if len == 0 {
                        return Ok(vec![]);
                    }
                    let start = Self::resolve_index(len, start);
                    let stop = {
                        let s = Self::resolve_index(len, stop);
                        s.min(len - 1)
                    };
                    if start > stop || start >= len {
                        return Ok(vec![]);
                    }
                    let slice: Vec<Vec<u8>> = zset[start..=stop]
                        .iter()
                        .map(|(m, _)| m.clone())
                        .collect();
                    if rev {
                        Ok(slice.into_iter().rev().collect())
                    } else {
                        Ok(slice)
                    }
                }
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn zrangebyscore(&self, key: &str, min: f64, max: f64) -> CacheResult<Vec<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(vec![]),
            Some(entry) => match &entry.value {
                CacheValue::ZSet(zset) => {
                    Ok(zset
                        .iter()
                        .filter(|(_, score)| *score >= min && *score <= max)
                        .map(|(m, _)| m.clone())
                        .collect())
                }
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn zrank(&self, key: &str, member: &[u8]) -> CacheResult<Option<usize>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(None),
            Some(entry) => match &entry.value {
                CacheValue::ZSet(zset) => {
                    Ok(zset.iter().position(|(m, _)| m.as_slice() == member))
                }
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn zscore(&self, key: &str, member: &[u8]) -> CacheResult<Option<f64>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(None),
            Some(entry) => match &entry.value {
                CacheValue::ZSet(zset) => {
                    Ok(zset.iter().find(|(m, _)| m.as_slice() == member).map(|(_, s)| *s))
                }
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn zcard(&self, key: &str) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(0),
            Some(entry) => match &entry.value {
                CacheValue::ZSet(zset) => Ok(zset.len()),
                _ => Err(CacheError::WrongType),
            },
        }
    }
}
