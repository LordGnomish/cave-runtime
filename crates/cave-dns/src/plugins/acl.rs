// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// ACL plugin — allow or deny queries by source IP/CIDR.
use std::str::FromStr;

use async_trait::async_trait;
use hickory_proto::op::ResponseCode;
use ipnet::IpNet;
use tracing::debug;

use crate::{
    config::{AclAction, AclConfig, AclRule},
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
    protocol::message::make_error_response,
};

struct CompiledRule {
    action: AclAction,
    networks: Vec<IpNet>,
    zones: Vec<String>,
    types: Vec<String>,
}

pub struct AclPlugin {
    rules: Vec<CompiledRule>,
    default_action: AclAction,
}

impl AclPlugin {
    pub fn new(config: AclConfig) -> DnsResult<Self> {
        let rules = config
            .rules
            .into_iter()
            .map(|rule| {
                let networks = rule
                    .source
                    .iter()
                    .map(|s| {
                        // Allow bare IPs like "1.2.3.4" as /32
                        if s.contains('/') {
                            s.parse::<IpNet>()
                                .map_err(|e| DnsError::Config(format!("ACL CIDR parse {s}: {e}")))
                        } else {
                            let host: std::net::IpAddr = s
                                .parse()
                                .map_err(|e| DnsError::Config(format!("ACL IP parse {s}: {e}")))?;
                            Ok(IpNet::from(host))
                        }
                    })
                    .collect::<DnsResult<Vec<_>>>()?;
                Ok(CompiledRule {
                    action: rule.action,
                    networks,
                    zones: rule.zones,
                    types: rule.types,
                })
            })
            .collect::<DnsResult<Vec<_>>>()?;

        Ok(Self {
            rules,
            default_action: config.default_action,
        })
    }

    fn evaluate(&self, ctx: &QueryContext) -> AclAction {
        let client_ip = ctx.client_addr.ip();
        let q = ctx.request.queries().first();
        let qname = q.map(|q| q.name().to_string()).unwrap_or_default();
        let qtype = q.map(|q| q.query_type().to_string()).unwrap_or_default();

        for rule in &self.rules {
            // IP match (empty = match all)
            let ip_match = rule.networks.is_empty()
                || rule.networks.iter().any(|net| net.contains(&client_ip));

            // Zone match (empty = match all)
            let zone_match = rule.zones.is_empty()
                || rule.zones.iter().any(|z| qname.ends_with(z.as_str()));

            // Type match (empty = match all)
            let type_match = rule.types.is_empty()
                || rule.types.iter().any(|t| t.eq_ignore_ascii_case(&qtype));

            if ip_match && zone_match && type_match {
                debug!(
                    client = %client_ip,
                    action = ?rule.action,
                    "ACL match"
                );
                return rule.action.clone();
            }
        }

        self.default_action.clone()
    }
}

#[async_trait]
impl Plugin for AclPlugin {
    fn name(&self) -> &str {
        "acl"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        match self.evaluate(ctx) {
            AclAction::Allow => next.run(ctx).await,
            AclAction::Deny => {
                ctx.response = make_error_response(&ctx.request, ResponseCode::Refused);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AclAction, AclConfig, AclRule};
    use crate::plugins::{Protocol, QueryContext};
    use hickory_proto::op::{Message, MessageType, OpCode, Query};
    use hickory_proto::rr::{DNSClass, Name, RecordType};
    use std::net::SocketAddr;

    fn make_ctx(client: &str) -> QueryContext {
        let addr: SocketAddr = client.parse().unwrap();
        let mut msg = Message::new();
        msg.set_id(1);
        msg.set_message_type(MessageType::Query);
        msg.set_op_code(OpCode::Query);
        let mut q = Query::new();
        q.set_name("example.com.".parse::<Name>().unwrap());
        q.set_query_type(RecordType::A);
        q.set_query_class(DNSClass::IN);
        msg.add_query(q);
        QueryContext::new(msg, addr, Protocol::Udp)
    }

    #[tokio::test]
    async fn deny_specific_ip() {
        let plugin = AclPlugin::new(AclConfig {
            rules: vec![AclRule {
                action: AclAction::Deny,
                source: vec!["10.0.0.1".into()],
                zones: vec![],
                types: vec![],
            }],
            default_action: AclAction::Allow,
        })
        .unwrap();

        let mut ctx = make_ctx("10.0.0.1:1234");
        struct NoOp;
        // Just check evaluate returns Deny
        assert!(matches!(plugin.evaluate(&ctx), AclAction::Deny));

        let mut ctx2 = make_ctx("10.0.0.2:1234");
        assert!(matches!(plugin.evaluate(&ctx2), AclAction::Allow));
    }
}
