// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::types::{DnsMessage, Header, RCODE_REFUSED};
use std::collections::HashSet;

/// Plugin trait — synchronous chain (no async in traits).
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    /// Process a DNS message. Return `Some(response)` to short-circuit, `None` to pass to next plugin.
    fn handle(&self, msg: &DnsMessage) -> Option<DnsMessage>;
}

pub struct PluginChain {
    plugins: Vec<Box<dyn Plugin>>,
}

impl Default for PluginChain {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginChain {
    pub fn new() -> Self {
        PluginChain { plugins: vec![] }
    }

    pub fn add(&mut self, plugin: Box<dyn Plugin>) {
        self.plugins.push(plugin);
    }

    pub fn process(&self, msg: &DnsMessage) -> Option<DnsMessage> {
        for p in &self.plugins {
            if let Some(resp) = p.handle(msg) {
                return Some(resp);
            }
        }
        None
    }
}

// ── Built-in plugins ─────────────────────────────────────────────────────────

pub struct LogPlugin;

impl Plugin for LogPlugin {
    fn name(&self) -> &str {
        "log"
    }

    fn handle(&self, msg: &DnsMessage) -> Option<DnsMessage> {
        tracing::debug!(
            "DNS query: {:?}",
            msg.questions.first().map(|q| &q.name)
        );
        None
    }
}

pub struct BlocklistPlugin {
    blocked: HashSet<String>,
}

impl BlocklistPlugin {
    pub fn new(domains: Vec<String>) -> Self {
        let blocked = domains.into_iter().collect();
        BlocklistPlugin { blocked }
    }
}

impl Plugin for BlocklistPlugin {
    fn name(&self) -> &str {
        "blocklist"
    }

    fn handle(&self, msg: &DnsMessage) -> Option<DnsMessage> {
        let is_blocked = msg.questions.first().map_or(false, |q| {
            self.blocked.contains(&q.name)
        });

        if is_blocked {
            Some(refused_response(msg))
        } else {
            None
        }
    }
}

fn refused_response(query: &DnsMessage) -> DnsMessage {
    DnsMessage {
        header: Header {
            id: query.header.id,
            qr: true,
            opcode: query.header.opcode,
            aa: false,
            tc: false,
            rd: query.header.rd,
            ra: false,
            z: 0,
            rcode: RCODE_REFUSED,
        },
        questions: query.questions.clone(),
        answers: vec![],
        authority: vec![],
        additional: vec![],
    }
}
