use crate::types::ResourceRecord;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[allow(dead_code)]
pub struct CacheEntry {
    records: Vec<ResourceRecord>,
    expires_at: Instant,
}

pub struct DnsCache {
    entries: Mutex<HashMap<(String, u16), CacheEntry>>,
    max_entries: usize,
}

impl DnsCache {
    pub fn new(max_entries: usize) -> Self {
        DnsCache {
            entries: Mutex::new(HashMap::new()),
            max_entries,
        }
    }

    pub fn get(&self, name: &str, rtype: u16) -> Option<Vec<ResourceRecord>> {
        let entries = self.entries.lock().unwrap();
        let key = (name.to_string(), rtype);
        if let Some(entry) = entries.get(&key) {
            if Instant::now() < entry.expires_at {
                return Some(entry.records.clone());
            }
        }
        None
    }

    pub fn insert(&self, name: &str, rtype: u16, records: Vec<ResourceRecord>, ttl_secs: u32) {
        let mut entries = self.entries.lock().unwrap();

        // Enforce max_entries by evicting one if at capacity
        if entries.len() >= self.max_entries {
            if let Some(key) = entries.keys().next().cloned() {
                entries.remove(&key);
            }
        }

        let expires_at = Instant::now() + Duration::from_secs(u64::from(ttl_secs));
        entries.insert(
            (name.to_string(), rtype),
            CacheEntry { records, expires_at },
        );
    }

    pub fn evict_expired(&self) {
        let mut entries = self.entries.lock().unwrap();
        let now = Instant::now();
        entries.retain(|_, v| now < v.expires_at);
    }

    pub fn size(&self) -> usize {
        let entries = self.entries.lock().unwrap();
        entries.len()
    }
}
