// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Etcd plugin — DNS records from etcd key-value store.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    config::EtcdConfig,
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

/// Schema for DNS records stored in etcd as JSON.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EtcdRecord {
    pub host: Option<String>,
    pub ip: Option<String>,
    pub ttl: Option<u32>,
    pub text: Option<String>,
    #[serde(rename = "type")]
    pub record_type: Option<String>,
}

type RecordStore = HashMap<(Name, RecordType), Vec<Record>>;

pub struct EtcdPlugin {
    config: EtcdConfig,
    store: Arc<ArcSwap<RecordStore>>,
}

impl EtcdPlugin {
    pub fn new(config: EtcdConfig) -> Self {
        Self {
            config,
            store: Arc::new(ArcSwap::new(Arc::new(HashMap::new()))),
        }
    }

    async fn sync(&self) -> DnsResult<RecordStore> {
        use etcd_client::{Client, GetOptions};

        let client = Client::connect(&self.config.endpoints, None)
            .await
            .map_err(|e| DnsError::Etcd(format!("connect: {e}")))?;

        let prefix = self.config.prefix.trim_end_matches('/').to_owned() + "/";
        let opts = GetOptions::new().with_prefix();
        let resp = client
            .kv_client()
            .get(prefix.as_bytes(), Some(opts))
            .await
            .map_err(|e| DnsError::Etcd(format!("get: {e}")))?;

        let mut store: RecordStore = HashMap::new();

        for kv in resp.kvs() {
            let key = String::from_utf8_lossy(kv.key());
            let value = String::from_utf8_lossy(kv.value());

            let etcd_rec: EtcdRecord = match serde_json::from_str(&value) {
                Ok(r) => r,
                Err(e) => {
                    warn!(key = %key, error = %e, "etcd: bad record JSON");
                    continue;
                }
            };

            // Convert etcd key path to DNS name:
            // /skydns/local/cluster/svc → svc.cluster.local.
            let relative = key
                .strip_prefix(&self.config.prefix)
                .unwrap_or(&key);
            let labels: Vec<&str> = relative
                .trim_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            let dns_name: String = labels
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(".")
                + ".";

            let name: Name = match dns_name.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };

            let ttl = etcd_rec.ttl.unwrap_or(300);

            if let Some(ip_str) = &etcd_rec.ip {
                let ip: std::net::IpAddr = match ip_str.parse() {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                let (rdata, rtype) = match ip {
                    std::net::IpAddr::V4(v4) => (
                        RData::A(hickory_proto::rr::rdata::A(v4)),
                        RecordType::A,
                    ),
                    std::net::IpAddr::V6(v6) => (
                        RData::AAAA(hickory_proto::rr::rdata::AAAA(v6)),
                        RecordType::AAAA,
                    ),
                };
                let mut r = Record::new();
                r.set_name(name.clone());
                r.set_ttl(ttl);
                r.set_record_type(rtype);
                r.set_dns_class(DNSClass::IN);
                r.set_data(Some(rdata));
                store.entry((name, rtype)).or_default().push(r);
            } else if let Some(host) = &etcd_rec.host {
                let target: Name = match format!("{host}.").parse() {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                let mut r = Record::new();
                r.set_name(name.clone());
                r.set_ttl(ttl);
                r.set_record_type(RecordType::CNAME);
                r.set_dns_class(DNSClass::IN);
                r.set_data(Some(RData::CNAME(hickory_proto::rr::rdata::CNAME(target))));
                store
                    .entry((name, RecordType::CNAME))
                    .or_default()
                    .push(r);
            }
        }
        Ok(store)
    }
}

#[async_trait]
impl Plugin for EtcdPlugin {
    fn name(&self) -> &str {
        "etcd"
    }

    async fn ready(&self) -> DnsResult<()> {
        match self.sync().await {
            Ok(store) => {
                info!(records = store.len(), "etcd plugin synced");
                self.store.store(Arc::new(store));
            }
            Err(e) => {
                warn!(error = %e, "etcd: initial sync failed (continuing)");
            }
        }
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let store = self.store.load();
        if let Some(records) = store.get(&(q.name().clone(), q.query_type())) {
            for r in records {
                ctx.response.add_answer(r.clone());
            }
            return Ok(());
        }

        next.run(ctx).await
    }
}
