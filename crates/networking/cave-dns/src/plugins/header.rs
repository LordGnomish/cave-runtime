// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Header plugin — set/clear DNS message header flags.
//!
//! Line-by-line port of CoreDNS v1.14.3 `plugin/header/header.go` +
//! `plugin/header/handler.go`. Supported flags: `aa` (authoritative),
//! `ra` (recursion available), `rd` (recursion desired). Query rules are
//! applied to the incoming request; response rules to the outgoing response.
use async_trait::async_trait;
use hickory_proto::op::Message;

use crate::{
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

/// A single set/clear directive for one header flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderRule {
    pub flag: String,
    pub state: bool,
}

pub struct HeaderPlugin {
    query_rules: Vec<HeaderRule>,
    response_rules: Vec<HeaderRule>,
}

impl HeaderPlugin {
    pub fn new(query_rules: Vec<HeaderRule>, response_rules: Vec<HeaderRule>) -> Self {
        Self {
            query_rules,
            response_rules,
        }
    }

    /// `header.go` newRules(): validate the action (`set`/`clear`) and each
    /// flag (`aa`/`ra`/`rd`), producing one rule per flag.
    pub fn parse_rules(action: &str, flags: &[&str]) -> DnsResult<Vec<HeaderRule>> {
        let state = match action.to_lowercase().as_str() {
            "set" => true,
            "clear" => false,
            other => {
                return Err(DnsError::Config(format!(
                    "unknown flag action={other}, should be set or clear"
                )))
            }
        };
        if flags.is_empty() {
            return Err(DnsError::Config(
                "invalid length for flags, at least one should be provided".into(),
            ));
        }
        let mut rules = Vec::with_capacity(flags.len());
        for raw in flags {
            let flag = raw.to_lowercase();
            match flag.as_str() {
                "aa" | "ra" | "rd" => {}
                other => {
                    return Err(DnsError::Config(format!("unknown/unsupported flag={other}")))
                }
            }
            rules.push(HeaderRule { flag, state });
        }
        Ok(rules)
    }

    /// `header.go` applyRules(): mutate the supported flags on a message.
    fn apply(rules: &[HeaderRule], msg: &mut Message) {
        for rule in rules {
            match rule.flag.as_str() {
                "aa" => msg.set_authoritative(rule.state),
                "ra" => msg.set_recursion_available(rule.state),
                "rd" => msg.set_recursion_desired(rule.state),
                _ => {}
            };
        }
    }
}

#[async_trait]
impl Plugin for HeaderPlugin {
    fn name(&self) -> &str {
        "header"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        Self::apply(&self.query_rules, &mut ctx.request);
        next.run(ctx).await?;
        Self::apply(&self.response_rules, &mut ctx.response);
        Ok(())
    }
}
