//! DNS observation cache.

use crate::models::DnsRecord;
use chrono::Utc;
use dashmap::DashMap;

pub struct DnsCache {
    records: DashMap<String, DnsRecord>,
    history: std::sync::Mutex<Vec<DnsRecord>>,
}

impl DnsCache {
    pub fn new() -> Self {
        Self {
            records: DashMap::new(),
            history: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn observe(&self, name: &str, record_type: &str, values: Vec<String>, ttl: u64, pod: Option<String>, ns: Option<String>) {
        let key = format!("{name}/{record_type}");
        let rec = DnsRecord {
            name: name.to_owned(),
            record_type: record_type.to_owned(),
            values,
            ttl_secs: ttl,
            observed_at: Utc::now(),
            source_pod: pod,
            source_namespace: ns,
        };
        self.records.insert(key, rec.clone());
        let mut hist = self.history.lock().unwrap();
        hist.push(rec);
        let len = hist.len();
        if len > 1000 {
            let excess = len - 1000;
            hist.drain(0..excess);
        }
    }

    pub fn lookup(&self, name: &str) -> Vec<DnsRecord> {
        self.records.iter()
            .filter(|r| r.value().name == name)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn list(&self) -> Vec<DnsRecord> {
        self.records.iter().map(|r| r.value().clone()).collect()
    }

    pub fn search(&self, query: &str) -> Vec<DnsRecord> {
        self.records.iter()
            .filter(|r| r.value().name.contains(query))
            .map(|r| r.value().clone())
            .collect()
    }
}

impl Default for DnsCache {
    fn default() -> Self { Self::new() }
}
