// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Hosts plugin — /etc/hosts style static records.
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
use tracing::{debug, info};

use crate::{
    config::HostsConfig,
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

struct HostsData {
    /// name → addresses
    forward: HashMap<Name, Vec<IpAddr>>,
    /// addr string → name (for PTR)
    reverse: HashMap<String, Name>,
}

impl HostsData {
    fn empty() -> Self {
        Self {
            forward: HashMap::new(),
            reverse: HashMap::new(),
        }
    }

    fn add(&mut self, addr: IpAddr, names: &[Name], ttl: u32) {
        for name in names {
            self.forward.entry(name.clone()).or_default().push(addr);
            self.reverse.insert(addr.to_string(), name.clone());
        }
    }
}

pub struct HostsPlugin {
    config: HostsConfig,
    data: Arc<ArcSwap<HostsData>>,
}

impl HostsPlugin {
    pub fn new(config: HostsConfig) -> Self {
        Self {
            config,
            data: Arc::new(ArcSwap::new(Arc::new(HostsData::empty()))),
        }
    }

    fn parse_content(content: &str, ttl: u32) -> HostsData {
        let mut data = HostsData::empty();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let ip_str = match parts.next() {
                Some(s) => s,
                None => continue,
            };
            let addr: IpAddr = match ip_str.parse() {
                Ok(a) => a,
                Err(_) => continue,
            };
            let names: Vec<Name> = parts
                .filter_map(|h| {
                    let fqdn = if h.ends_with('.') {
                        h.to_owned()
                    } else {
                        format!("{h}.")
                    };
                    fqdn.parse().ok()
                })
                .collect();
            if names.is_empty() {
                continue;
            }
            data.add(addr, &names, ttl);
        }
        data
    }

    async fn reload(&self) -> DnsResult<()> {
        let path = self.config.path.as_deref().unwrap_or("/etc/hosts");
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| DnsError::Plugin(format!("hosts: cannot read {path}: {e}")))?;

        let mut data = Self::parse_content(&content, self.config.ttl);

        // Also parse inline entries
        let inline = self.config.inline.join("\n");
        let inline_data = Self::parse_content(&inline, self.config.ttl);
        for (name, addrs) in inline_data.forward {
            data.forward.entry(name).or_default().extend(addrs);
        }
        for (addr, name) in inline_data.reverse {
            data.reverse.insert(addr, name);
        }

        self.data.store(Arc::new(data));
        Ok(())
    }

    fn build_records(data: &HostsData, name: &Name, qtype: RecordType, ttl: u32) -> Vec<Record> {
        let addrs = match data.forward.get(name) {
            Some(v) => v,
            None => return vec![],
        };

        addrs
            .iter()
            .filter_map(|addr| match (qtype, addr) {
                (RecordType::A, IpAddr::V4(v4)) | (RecordType::ANY, IpAddr::V4(v4)) => {
                    let mut r = Record::new();
                    r.set_name(name.clone());
                    r.set_ttl(ttl);
                    r.set_record_type(RecordType::A);
                    r.set_dns_class(DNSClass::IN);
                    r.set_data(Some(RData::A(hickory_proto::rr::rdata::A(*v4))));
                    Some(r)
                }
                (RecordType::AAAA, IpAddr::V6(v6)) | (RecordType::ANY, IpAddr::V6(v6)) => {
                    let mut r = Record::new();
                    r.set_name(name.clone());
                    r.set_ttl(ttl);
                    r.set_record_type(RecordType::AAAA);
                    r.set_dns_class(DNSClass::IN);
                    r.set_data(Some(RData::AAAA(hickory_proto::rr::rdata::AAAA(*v6))));
                    Some(r)
                }
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl Plugin for HostsPlugin {
    fn name(&self) -> &str {
        "hosts"
    }

    async fn ready(&self) -> DnsResult<()> {
        self.reload().await?;
        info!("hosts plugin loaded");
        Ok(())
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let data = self.data.load();
        let records = Self::build_records(&data, q.name(), q.query_type(), self.config.ttl);

        if !records.is_empty() {
            ctx.response.set_authoritative(false);
            for r in records {
                ctx.response.add_answer(r);
            }
            return Ok(());
        }

        if self.config.fallthrough {
            next.run(ctx).await
        } else {
            Ok(())
        }
    }
}
