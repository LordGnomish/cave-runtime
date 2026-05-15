// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Rewrite plugin — query name/type/class/TTL rewriting.
use async_trait::async_trait;
use hickory_proto::rr::{Name, RecordType};
use regex::Regex;
use tracing::debug;

use crate::{
    config::{MatchType, RewriteAction, RewriteConfig, RewriteRule},
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

struct CompiledRule {
    rule: RewriteRule,
    regex: Option<Regex>,
}

pub struct RewritePlugin {
    rules: Vec<CompiledRule>,
}

impl RewritePlugin {
    pub fn new(config: RewriteConfig) -> DnsResult<Self> {
        let rules = config
            .rules
            .into_iter()
            .map(|rule| {
                let regex = if matches!(rule.match_type, MatchType::Regex) {
                    Some(
                        Regex::new(&rule.from)
                            .map_err(|e| DnsError::Config(format!("rewrite regex {}: {e}", rule.from)))?,
                    )
                } else {
                    None
                };
                Ok(CompiledRule { rule, regex })
            })
            .collect::<DnsResult<Vec<_>>>()?;
        Ok(Self { rules })
    }

    fn rewrite_name<'a>(&self, name: &'a str, rule: &CompiledRule) -> Option<String> {
        match rule.rule.match_type {
            MatchType::Exact => {
                if name == rule.rule.from || name == format!("{}.", rule.rule.from) {
                    Some(rule.rule.to.clone())
                } else {
                    None
                }
            }
            MatchType::Prefix => {
                if name.starts_with(&rule.rule.from) {
                    Some(name.replacen(&rule.rule.from, &rule.rule.to, 1))
                } else {
                    None
                }
            }
            MatchType::Suffix => {
                if name.ends_with(&rule.rule.from) {
                    let len = name.len() - rule.rule.from.len();
                    Some(format!("{}{}", &name[..len], rule.rule.to))
                } else {
                    None
                }
            }
            MatchType::Substring => {
                if name.contains(&rule.rule.from) {
                    Some(name.replace(&rule.rule.from, &rule.rule.to))
                } else {
                    None
                }
            }
            MatchType::Regex => {
                if let Some(re) = &rule.regex {
                    if re.is_match(name) {
                        Some(re.replace(name, rule.rule.to.as_str()).into_owned())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
    }
}

#[async_trait]
impl Plugin for RewritePlugin {
    fn name(&self) -> &str {
        "rewrite"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let original_name = q.name().to_string();

        for rule in &self.rules {
            if let Some(new_name) = self.rewrite_name(&original_name, rule) {
                match rule.rule.action {
                    RewriteAction::Name => {
                        // Rewrite the query name in-place
                        if let Ok(new_n) = new_name.parse::<Name>() {
                            debug!(from = %original_name, to = %new_n, "rewrite");
                            // Modify the request's query section
                            let mut new_q = q.clone();
                            new_q.set_name(new_n);
                            ctx.request.take_queries();
                            ctx.request.add_query(new_q);
                        }
                    }
                    RewriteAction::Type => {
                        if let Ok(new_type) = new_name.parse::<RecordType>() {
                            let mut new_q = q.clone();
                            new_q.set_query_type(new_type);
                            ctx.request.take_queries();
                            ctx.request.add_query(new_q);
                        }
                    }
                    RewriteAction::Class | RewriteAction::Ttl => {
                        // TTL rewrite handled in response; class rewrite is rare
                    }
                }

                if !rule.rule.continue_on_match {
                    break;
                }
            }
        }

        next.run(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MatchType, RewriteAction, RewriteRule};

    #[test]
    fn exact_name_rewrite() {
        let plugin = RewritePlugin::new(RewriteConfig {
            rules: vec![RewriteRule {
                match_type: MatchType::Exact,
                from: "old.example.com.".into(),
                to: "new.example.com.".into(),
                action: RewriteAction::Name,
                continue_on_match: false,
            }],
        })
        .unwrap();

        let rule = &plugin.rules[0];
        let result = plugin.rewrite_name("old.example.com.", rule);
        assert_eq!(result, Some("new.example.com.".into()));

        let no_match = plugin.rewrite_name("other.example.com.", rule);
        assert!(no_match.is_none());
    }

    #[test]
    fn regex_rewrite() {
        let plugin = RewritePlugin::new(RewriteConfig {
            rules: vec![RewriteRule {
                match_type: MatchType::Regex,
                from: r"^(\w+)\.old\.example\.com\.$".into(),
                to: "${1}.new.example.com.".into(),
                action: RewriteAction::Name,
                continue_on_match: false,
            }],
        })
        .unwrap();

        let rule = &plugin.rules[0];
        let result = plugin.rewrite_name("www.old.example.com.", rule);
        assert!(result.is_some());
    }
}
