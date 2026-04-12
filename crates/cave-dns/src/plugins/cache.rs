/// Cache plugin — positive and negative DNS caching with TTL countdown.
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use hickory_proto::{
    op::ResponseCode,
    rr::{DNSClass, Name, Record, RecordType},
};
use tracing::debug;

use crate::{
    config::CacheConfig,
    error::DnsResult,
    plugins::{Next, Plugin, QueryContext},
};

type CacheKey = (Name, RecordType, DNSClass);

#[derive(Clone)]
struct CacheEntry {
    records: Vec<Record>,
    cached_at: Instant,
    /// Original TTL of the record(s) at cache time.
    original_ttl: u32,
    negative: bool,
}

impl CacheEntry {
    fn remaining_ttl(&self, now: Instant) -> u32 {
        let elapsed = now.duration_since(self.cached_at).as_secs() as u32;
        self.original_ttl.saturating_sub(elapsed)
    }

    fn is_expired(&self, now: Instant) -> bool {
        self.remaining_ttl(now) == 0
    }
}

pub struct CachePlugin {
    config: CacheConfig,
    store: Arc<DashMap<CacheKey, CacheEntry>>,
}

impl CachePlugin {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            store: Arc::new(DashMap::new()),
        }
    }

    fn cache_key(ctx: &QueryContext) -> Option<CacheKey> {
        let q = ctx.request.queries().first()?;
        Some((q.name().clone(), q.query_type(), q.query_class()))
    }

    fn get(&self, key: &CacheKey) -> Option<CacheEntry> {
        let now = Instant::now();
        let entry = self.store.get(key)?.clone();

        if entry.is_expired(now) {
            if self.config.serve_stale {
                // Return stale entry but mark for refresh
                let stale_age = now.duration_since(entry.cached_at).as_secs() as u32;
                if stale_age > self.config.original_ttl_plus_stale(&entry) {
                    return None;
                }
                return Some(entry);
            }
            self.store.remove(key);
            return None;
        }

        Some(entry)
    }

    fn put(&self, key: CacheKey, records: Vec<Record>, ttl: u32, negative: bool) {
        let ttl = ttl
            .min(self.config.max_ttl)
            .max(self.config.min_ttl);
        if ttl == 0 && !negative {
            return; // do not cache zero-TTL records
        }
        let entry = CacheEntry {
            records,
            cached_at: Instant::now(),
            original_ttl: ttl,
            negative,
        };
        if self.store.len() >= self.config.capacity {
            // Evict one expired entry (simple strategy)
            let now = Instant::now();
            if let Some(key_to_remove) = self
                .store
                .iter()
                .find(|e| e.is_expired(now))
                .map(|e| e.key().clone())
            {
                self.store.remove(&key_to_remove);
            }
        }
        self.store.insert(key, entry);
    }
}

trait CacheConfigExt {
    fn original_ttl_plus_stale(&self, entry: &CacheEntry) -> u32;
}

impl CacheConfigExt for CacheConfig {
    fn original_ttl_plus_stale(&self, entry: &CacheEntry) -> u32 {
        entry.original_ttl.saturating_add(self.stale_ttl)
    }
}

#[async_trait]
impl Plugin for CachePlugin {
    fn name(&self) -> &str {
        "cache"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let key = match Self::cache_key(ctx) {
            Some(k) => k,
            None => return next.run(ctx).await,
        };

        // Cache hit path
        if let Some(entry) = self.get(&key) {
            let now = Instant::now();
            let remaining = entry.remaining_ttl(now);
            debug!(name = %key.0, rtype = %key.1, ttl = remaining, "cache hit");

            // Adjust TTLs downward
            let records: Vec<Record> = entry
                .records
                .into_iter()
                .map(|mut r| {
                    r.set_ttl(remaining);
                    r
                })
                .collect();

            if entry.negative {
                ctx.response.set_response_code(ResponseCode::NXDomain);
                for r in records {
                    ctx.response.add_name_server(r);
                }
            } else {
                for r in records {
                    ctx.response.add_answer(r);
                }
            }
            return Ok(());
        }

        // Cache miss — call next plugin then store response
        next.run(ctx).await?;

        let rcode = ctx.response.response_code();
        let answers = ctx.response.answers().to_vec();

        if rcode == ResponseCode::NXDomain {
            let ttl = ctx
                .response
                .name_servers()
                .iter()
                .find(|r| r.record_type() == RecordType::SOA)
                .map(|r| r.ttl())
                .unwrap_or(self.config.neg_ttl);
            self.put(key, ctx.response.name_servers().to_vec(), ttl, true);
        } else if rcode == ResponseCode::NoError && !answers.is_empty() {
            let min_ttl = answers.iter().map(|r| r.ttl()).min().unwrap_or(0);
            self.put(key, answers, min_ttl, false);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CacheConfig;
    use hickory_proto::rr::rdata::A;
    use std::net::Ipv4Addr;

    fn make_a_record(name: &str, ttl: u32, addr: Ipv4Addr) -> Record {
        let mut r = Record::new();
        r.set_name(name.parse().unwrap());
        r.set_ttl(ttl);
        r.set_record_type(RecordType::A);
        r.set_dns_class(DNSClass::IN);
        r.set_data(Some(hickory_proto::rr::RData::A(A(addr))));
        r
    }

    #[test]
    fn put_and_get_entry() {
        let plugin = CachePlugin::new(CacheConfig::default());
        let key: CacheKey = (
            "www.example.com.".parse().unwrap(),
            RecordType::A,
            DNSClass::IN,
        );
        plugin.put(
            key.clone(),
            vec![make_a_record("www.example.com.", 300, Ipv4Addr::new(1, 2, 3, 4))],
            300,
            false,
        );
        let entry = plugin.get(&key).unwrap();
        assert!(!entry.negative);
        assert_eq!(entry.records.len(), 1);
    }

    #[test]
    fn expired_entry_is_removed() {
        let mut config = CacheConfig::default();
        config.serve_stale = false;
        let plugin = CachePlugin::new(config);
        let key: CacheKey = (
            "expired.example.com.".parse().unwrap(),
            RecordType::A,
            DNSClass::IN,
        );
        // Insert with TTL=0 → should not cache
        plugin.put(
            key.clone(),
            vec![make_a_record("expired.example.com.", 0, Ipv4Addr::new(1, 2, 3, 4))],
            0,
            false,
        );
        assert!(plugin.get(&key).is_none());
    }
}
