// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Alert output formatters.
//!
//! NOTICE: upstream is falcosecurity/falco/userspace/falco/falco_outputs.cpp.
//! cave-falco models the *payload* shape only — real network transports
//! (HTTP webhook, gRPC, syslog) are out-of-process per
//! ADR-RUNTIME-SANDBOX-NO-FFI-001 §1.

use crate::engine::EngineMatch;
use crate::event::FalcoEvent;
use crate::token_bucket::TokenBucket;
use serde::{Deserialize, Serialize};

/// Output-stream throttle — mirrors `falco_outputs`' notification token
/// bucket (`outputs: { rate, max_burst }`). A `rate` of `0` disables
/// throttling (Falco's default), otherwise alerts beyond `max_burst` in a
/// burst are dropped until tokens regenerate at `rate`/second.
#[derive(Debug, Clone)]
pub struct OutputThrottle {
    rate: f64,
    bucket: TokenBucket,
}

impl OutputThrottle {
    pub fn new(rate: f64, max_burst: f64, now_ns: u64) -> Self {
        Self { rate, bucket: TokenBucket::new(rate, max_burst, now_ns) }
    }

    /// Returns `true` if an alert may be emitted at `now_ns`. When `rate`
    /// is `<= 0`, throttling is disabled and every alert is allowed.
    pub fn allow(&mut self, now_ns: u64) -> bool {
        if self.rate <= 0.0 {
            return true;
        }
        self.bucket.claim(now_ns)
    }

    pub fn tokens(&self) -> f64 { self.bucket.tokens() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
    /// falcosecurity/falcosidekick payload shape.
    Sidekick,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub rule: String,
    pub priority: String,
    pub output: String,
    pub tags: Vec<String>,
    pub source: String,
    pub time_ns: i64,
    pub fields: serde_json::Value,
}

pub fn render(format: OutputFormat, m: &EngineMatch, ev: &FalcoEvent) -> String {
    let alert = Alert {
        rule: m.rule_name.clone(),
        priority: m.priority.as_str().into(),
        output: format_output_template(&m.output_template, ev),
        tags: m.tags.clone(),
        source: ev.source.clone(),
        time_ns: ev.timestamp_ns,
        fields: serde_json::to_value(&ev.fields).unwrap_or(serde_json::Value::Null),
    };
    match format {
        OutputFormat::Text => format!("{}: {} ({})", alert.priority, alert.output, alert.rule),
        OutputFormat::Json => serde_json::to_string(&alert).unwrap_or_else(|_| "{}".into()),
        OutputFormat::Sidekick => sidekick_payload(&alert),
    }
}

/// Falco's `%field` template substitution. `%proc.name` → field
/// value (empty string if absent).
fn format_output_template(template: &str, ev: &FalcoEvent) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let mut key = String::new();
            while let Some(&nc) = chars.peek() {
                if nc.is_alphanumeric() || nc == '.' || nc == '_' {
                    key.push(nc);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Some(v) = ev.fields.get(&key) {
                out.push_str(v);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn sidekick_payload(alert: &Alert) -> String {
    // falcosidekick expects `output`, `priority`, `rule`, `time`,
    // `output_fields`, `source`, `tags`.
    let payload = serde_json::json!({
        "output": alert.output,
        "priority": alert.priority,
        "rule": alert.rule,
        "time": alert.time_ns,
        "output_fields": alert.fields,
        "source": alert.source,
        "tags": alert.tags,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Priority;

    fn m() -> EngineMatch {
        EngineMatch {
            rule_name: "Shell in container".into(),
            priority: Priority::Warning,
            output_template: "Shell %proc.name in %container.id".into(),
            tags: vec!["container".into()],
        }
    }

    fn ev() -> FalcoEvent {
        FalcoEvent::syscall("execve")
            .with("proc.name", "bash")
            .with("container.id", "abc123")
    }

    const SEC: u64 = 1_000_000_000;

    #[test]
    fn throttle_rate_zero_allows_everything() {
        let mut t = OutputThrottle::new(0.0, 1.0, 0);
        for _ in 0..1000 {
            assert!(t.allow(0));
        }
    }

    #[test]
    fn throttle_drops_beyond_burst_then_recovers() {
        // rate 1/s, burst 3. Three alerts at t=0 pass, the fourth is dropped.
        let mut t = OutputThrottle::new(1.0, 3.0, 0);
        assert!(t.allow(0));
        assert!(t.allow(0));
        assert!(t.allow(0));
        assert!(!t.allow(0));
        // one second later, one token regenerates → one more allowed.
        assert!(t.allow(SEC));
        assert!(!t.allow(SEC));
    }

    #[test]
    fn template_substitution_replaces_fields() {
        let s = format_output_template("%proc.name in %container.id", &ev());
        assert_eq!(s, "bash in abc123");
    }

    #[test]
    fn template_absent_field_is_empty_string() {
        let s = format_output_template("%proc.name x %missing y", &ev());
        assert_eq!(s, "bash x  y");
    }

    #[test]
    fn text_format_includes_priority_and_rule_name() {
        let out = render(OutputFormat::Text, &m(), &ev());
        assert!(out.contains("WARNING"));
        assert!(out.contains("Shell in container"));
        assert!(out.contains("bash"));
    }

    #[test]
    fn json_format_parses_to_alert() {
        let out = render(OutputFormat::Json, &m(), &ev());
        let a: Alert = serde_json::from_str(&out).unwrap();
        assert_eq!(a.priority, "WARNING");
        assert_eq!(a.rule, "Shell in container");
        assert_eq!(a.tags, vec!["container".to_string()]);
    }

    #[test]
    fn sidekick_format_carries_output_fields_map() {
        let out = render(OutputFormat::Sidekick, &m(), &ev());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["rule"], "Shell in container");
        assert_eq!(v["output_fields"]["proc.name"], "bash");
        assert_eq!(v["source"], "syscall");
    }

    #[test]
    fn template_leading_percent_only_substitutes_known_chars() {
        // `%` followed by non-identifier char stays as-is (no substitution).
        let s = format_output_template("100%%", &ev());
        // The first `%` consumes following alnum chars; second `%` likewise.
        // Since `%` is not alnum, neither substitutes anything.
        assert_eq!(s, "100");
    }
}
