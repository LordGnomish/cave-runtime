// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// Template plugin — dynamic response generation using regex templates.
use async_trait::async_trait;
use hickory_proto::op::ResponseCode;
use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};
use regex::Regex;
use tracing::debug;

use crate::{
    config::{TemplateConfig, TemplateRule},
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

struct CompiledTemplate {
    rule: TemplateRule,
    regex: Regex,
    qtype: Option<RecordType>,
}

fn str_to_rcode(s: &str) -> ResponseCode {
    match s.to_uppercase().as_str() {
        "NOERROR" => ResponseCode::NoError,
        "NXDOMAIN" => ResponseCode::NXDomain,
        "SERVFAIL" => ResponseCode::ServFail,
        "REFUSED" => ResponseCode::Refused,
        "FORMERR" => ResponseCode::FormErr,
        "NOTIMP" => ResponseCode::NotImp,
        _ => ResponseCode::NoError,
    }
}

pub struct TemplatePlugin {
    templates: Vec<CompiledTemplate>,
}

impl TemplatePlugin {
    pub fn new(config: TemplateConfig) -> DnsResult<Self> {
        let templates = config
            .templates
            .into_iter()
            .map(|rule| {
                let regex = Regex::new(&rule.match_regex)
                    .map_err(|e| DnsError::Config(format!("template regex: {e}")))?;
                let qtype =
                    if rule.qtype.is_empty() || rule.qtype == "ANY" {
                        None
                    } else {
                        Some(rule.qtype.parse::<RecordType>().map_err(|_| {
                            DnsError::Config(format!("unknown type: {}", rule.qtype))
                        })?)
                    };
                Ok(CompiledTemplate { rule, regex, qtype })
            })
            .collect::<DnsResult<Vec<_>>>()?;
        Ok(Self { templates })
    }

    fn substitute(template: &str, name: &str, caps: &regex::Captures<'_>) -> String {
        let mut out = template.replace("{name}", name);
        // {1}, {2}, … → capture groups
        for i in 1..=9 {
            let placeholder = format!("{{{i}}}");
            let replacement = caps.get(i).map(|m| m.as_str()).unwrap_or("");
            out = out.replace(&placeholder, replacement);
        }
        out
    }
}

#[async_trait]
impl Plugin for TemplatePlugin {
    fn name(&self) -> &str {
        "template"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        let q = match ctx.request.queries().first() {
            Some(q) => q.clone(),
            None => return next.run(ctx).await,
        };

        let qname = q.name().to_string();
        let qtype = q.query_type();

        for tmpl in &self.templates {
            // Type filter
            if let Some(t) = tmpl.qtype {
                if t != qtype && qtype != RecordType::ANY {
                    continue;
                }
            }

            if let Some(caps) = tmpl.regex.captures(&qname) {
                debug!(name = %qname, "template matched");

                // Set RCODE
                let rcode = str_to_rcode(&tmpl.rule.rcode);
                ctx.response.set_response_code(rcode);

                // Build answer records from template strings
                for ans_tmpl in &tmpl.rule.answer {
                    let rdata_str = Self::substitute(ans_tmpl, &qname, &caps);
                    // Best-effort: parse as TXT
                    let mut r = Record::new();
                    r.set_name(q.name().clone());
                    r.set_ttl(300);
                    r.set_record_type(RecordType::TXT);
                    r.set_dns_class(DNSClass::IN);
                    r.set_data(Some(RData::TXT(hickory_proto::rr::rdata::TXT::new(vec![
                        rdata_str,
                    ]))));
                    ctx.response.add_answer(r);
                }

                if !tmpl.rule.fall_through {
                    return Ok(());
                }
            }
        }

        next.run(ctx).await
    }
}
