use std::collections::HashSet;
use crate::engine::CacheEngine;
use crate::types::{CacheEntry, CacheError, CacheResult, CacheValue};

impl CacheEngine {
    pub fn sadd(&self, key: &str, members: &[Vec<u8>]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: CacheValue::Set(HashSet::new()),
            expires_at: None,
            version: 0,
        });
        entry.version += 1;
        match &mut entry.value {
            CacheValue::Set(set) => {
                let mut added = 0;
                for m in members {
                    if set.insert(m.clone()) {
                        added += 1;
                    }
                }
                Ok(added)
            }
            _ => Err(CacheError::WrongType),
        }
    }

    pub fn srem(&self, key: &str, members: &[Vec<u8>]) -> CacheResult<usize> {
        let mut store = self.store.lock().unwrap();
        match store.get_mut(key) {
            None => Ok(0),
            Some(entry) => {
                if Self::is_expired(entry) {
                    store.remove(key);
                    return Ok(0);
                }
                match &mut entry.value {
                    CacheValue::Set(set) => {
                        let mut removed = 0;
                        for m in members {
                            if set.remove(m.as_slice()) {
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

    pub fn smembers(&self, key: &str) -> CacheResult<Vec<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(vec![]),
            Some(entry) => match &entry.value {
                CacheValue::Set(set) => Ok(set.iter().cloned().collect()),
                _ => Err(CacheError::WrongType),
            },
        }
    }

    pub fn sinter(&self, keys: &[&str]) -> CacheResult<Vec<Vec<u8>>> {
        if keys.is_empty() {
            return Ok(vec![]);
        }
        let mut store = self.store.lock().unwrap();
        // Collect all sets
        let mut sets: Vec<HashSet<Vec<u8>>> = Vec::new();
        for key in keys {
            match Self::get_entry(&mut store, key) {
                None => return Ok(vec![]), // intersection with empty is empty
                Some(entry) => match &entry.value {
                    CacheValue::Set(set) => sets.push(set.clone()),
                    _ => return Err(CacheError::WrongType),
                },
            }
        }
        let mut result: HashSet<Vec<u8>> = sets[0].clone();
        for s in &sets[1..] {
            result = result.intersection(s).cloned().collect();
        }
        Ok(result.into_iter().collect())
    }

    pub fn sunion(&self, keys: &[&str]) -> CacheResult<Vec<Vec<u8>>> {
        let mut store = self.store.lock().unwrap();
        let mut result: HashSet<Vec<u8>> = HashSet::new();
        for key in keys {
            match Self::get_entry(&mut store, key) {
                None => {}
                Some(entry) => match &entry.value {
                    CacheValue::Set(set) => {
                        result.extend(set.iter().cloned());
                    }
                    _ => return Err(CacheError::WrongType),
                },
            }
        }
        Ok(result.into_iter().collect())
    }

    pub fn sdiff(&self, keys: &[&str]) -> CacheResult<Vec<Vec<u8>>> {
        if keys.is_empty() {
            return Ok(vec![]);
        }
        let mut store = self.store.lock().unwrap();
        let base: HashSet<Vec<u8>> = match Self::get_entry(&mut store, keys[0]) {
            None => HashSet::new(),
            Some(entry) => match &entry.value {
                CacheValue::Set(set) => set.clone(),
                _ => return Err(CacheError::WrongType),
            },
        };
        let mut result = base;
        for key in &keys[1..] {
            match Self::get_entry(&mut store, key) {
                None => {}
                Some(entry) => match &entry.value {
                    CacheValue::Set(set) => {
                        result = result.difference(set).cloned().collect();
                    }
                    _ => return Err(CacheError::WrongType),
                },
            }
        }
        Ok(result.into_iter().collect())
    }

    pub fn sismember(&self, key: &str, member: &[u8]) -> CacheResult<bool> {
        let mut store = self.store.lock().unwrap();
        match Self::get_entry(&mut store, key) {
            None => Ok(false),
            Some(entry) => match &entry.value {
                CacheValue::Set(set) => Ok(set.contains(member)),
                _ => Err(CacheError::WrongType),
            },
        }
    }
}
