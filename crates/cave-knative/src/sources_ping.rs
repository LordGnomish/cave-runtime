// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PingSource — periodic CloudEvent emitter (cron-driven).
//!
//! upstream: knative/eventing — pkg/apis/sources/v1/ping_types.go
//! + pkg/adapter/v2/ping/adapter.go
//!
//! A PingSource fires a CloudEvent at a cron-defined cadence and pushes the
//! resulting envelope at a configured sink. Upstream uses a Go cron parser
//! and gokit/sdk-go for the CloudEvent payload; we ship a self-contained
//! 5-field cron evaluator (minute, hour, day-of-month, month, day-of-week)
//! plus a CloudEvent v1.0 envelope builder.
//!
//! Out of scope for this port: cluster-wide leader election (a cave-runtime
//! reconciler will own the singleton lease via cave-controller-manager);
//! delivery retry budget (handled by cave-keda/cave-runtime data-plane).

use std::collections::HashMap;
use crate::meta::ObjectMeta;

#[derive(Default, Debug, Clone)]
pub struct PingSource {
    pub metadata: ObjectMeta,
    pub spec: PingSourceSpec,
    pub status: PingSourceStatus,
}

#[derive(Debug, Clone)]
pub struct PingSourceSpec {
    /// 5-field cron expression in UTC: minute hour dom month dow.
    pub schedule: String,
    /// Optional CloudEvent `data` payload (base64-free, UTF-8 string).
    pub data: Option<String>,
    /// CloudEvent `contenttype` attribute (default `application/json`).
    pub content_type: String,
    /// Destination resolved by sink-resolver; we treat it as a URL.
    pub sink: Option<String>,
    /// `ceOverrides` extension attributes pushed onto every emitted event.
    pub ce_overrides: HashMap<String, String>,
}

impl Default for PingSourceSpec {
    fn default() -> Self {
        Self {
            schedule: "*/1 * * * *".to_string(),
            data: None,
            content_type: "application/json".to_string(),
            sink: None,
            ce_overrides: HashMap::new(),
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct PingSourceStatus {
    pub sink_uri: Option<String>,
    pub observed_generation: i64,
    pub last_fired_minute_of_day: Option<u32>,
}

/// A single emitted CloudEvent envelope (v1.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudEvent {
    pub id: String,
    pub source: String,
    pub spec_version: String,
    pub event_type: String,
    pub content_type: String,
    pub data: Option<String>,
    pub extensions: HashMap<String, String>,
}

impl PingSource {
    pub fn new(tenant_id: &str, schedule: &str) -> Self {
        let mut p = Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: PingSourceSpec::default(),
            status: PingSourceStatus::default(),
        };
        p.spec.schedule = schedule.to_string();
        p
    }

    /// Resolve the sink URI from spec.sink, recording it on status.
    pub fn resolve_sink(&mut self) -> Option<&str> {
        if let Some(ref s) = self.spec.sink {
            self.status.sink_uri = Some(s.clone());
        }
        self.status.sink_uri.as_deref()
    }

    /// Should this source fire at the given minute-of-day (UTC)?
    ///
    /// `minute_of_day` is `hour*60 + minute` in 0..1440.  We evaluate the 5
    /// cron fields against an arbitrary (dom, month, dow) using the supplied
    /// helpers so the same shape works for whatever wall clock the caller
    /// owns.
    pub fn fires_at(&self, minute_of_day: u32, dom: u32, month: u32, dow: u32) -> bool {
        let parts: Vec<&str> = self.spec.schedule.split_whitespace().collect();
        if parts.len() != 5 {
            return false;
        }
        let minute = minute_of_day % 60;
        let hour = minute_of_day / 60;
        cron_field_matches(parts[0], minute, 0, 59)
            && cron_field_matches(parts[1], hour, 0, 23)
            && cron_field_matches(parts[2], dom, 1, 31)
            && cron_field_matches(parts[3], month, 1, 12)
            && cron_field_matches(parts[4], dow, 0, 6)
    }

    /// Build the CloudEvent envelope for one firing.  Caller picks the
    /// event id (UUID upstream; we accept any unique string).
    pub fn emit(&self, event_id: &str) -> CloudEvent {
        let mut ext = self.spec.ce_overrides.clone();
        ext.insert("knativedev_sourceversion".to_string(), "v1.22.0".to_string());
        let source = if self.metadata.namespace.is_empty() {
            format!("/apis/sources.knative.dev/v1/pingsources/{}", self.metadata.name)
        } else {
            format!(
                "/apis/sources.knative.dev/v1/namespaces/{}/pingsources/{}",
                self.metadata.namespace, self.metadata.name
            )
        };
        CloudEvent {
            id: event_id.to_string(),
            source,
            spec_version: "1.0".to_string(),
            event_type: "dev.knative.sources.ping".to_string(),
            content_type: self.spec.content_type.clone(),
            data: self.spec.data.clone(),
            extensions: ext,
        }
    }
}

/// Evaluate a single cron field against a value.
///
/// Supports `*`, integer literals, comma-separated lists, `a-b` ranges, and
/// `*/N` step expressions.  Anything else parses as no-match.
fn cron_field_matches(field: &str, value: u32, min: u32, max: u32) -> bool {
    for token in field.split(',') {
        if cron_token_matches(token, value, min, max) {
            return true;
        }
    }
    false
}

fn cron_token_matches(token: &str, value: u32, min: u32, max: u32) -> bool {
    let token = token.trim();
    if token == "*" {
        return value >= min && value <= max;
    }
    if let Some(rest) = token.strip_prefix("*/") {
        if let Ok(step) = rest.parse::<u32>() {
            if step == 0 {
                return false;
            }
            return value >= min && value <= max && (value - min) % step == 0;
        }
        return false;
    }
    if let Some((a, b)) = token.split_once('-') {
        if let (Ok(lo), Ok(hi)) = (a.parse::<u32>(), b.parse::<u32>()) {
            return value >= lo && value <= hi && value >= min && value <= max;
        }
        return false;
    }
    if let Ok(n) = token.parse::<u32>() {
        return n == value;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_default_schedule_every_minute() {
        let p = PingSource::new("t", "*/1 * * * *");
        assert!(p.fires_at(0, 1, 1, 0));
        assert!(p.fires_at(7 * 60 + 23, 14, 6, 3));
    }

    #[test]
    fn ping_specific_hour_only() {
        let p = PingSource::new("t", "0 9 * * *");
        assert!(p.fires_at(9 * 60, 1, 1, 0));
        assert!(!p.fires_at(9 * 60 + 1, 1, 1, 0));
        assert!(!p.fires_at(10 * 60, 1, 1, 0));
    }

    #[test]
    fn ping_range_dow() {
        let p = PingSource::new("t", "0 0 * * 1-5");
        // Monday(1) through Friday(5) at midnight
        for dow in 1..=5 {
            assert!(p.fires_at(0, 1, 1, dow), "dow={dow}");
        }
        assert!(!p.fires_at(0, 1, 1, 0));
        assert!(!p.fires_at(0, 1, 1, 6));
    }

    #[test]
    fn ping_step_minute() {
        let p = PingSource::new("t", "*/15 * * * *");
        assert!(p.fires_at(0, 1, 1, 0));
        assert!(p.fires_at(15, 1, 1, 0));
        assert!(p.fires_at(60 + 30, 1, 1, 0));
        assert!(!p.fires_at(7, 1, 1, 0));
    }

    #[test]
    fn ping_comma_list_minutes() {
        let p = PingSource::new("t", "5,15,25 * * * *");
        assert!(p.fires_at(5, 1, 1, 0));
        assert!(p.fires_at(15, 1, 1, 0));
        assert!(p.fires_at(25, 1, 1, 0));
        assert!(!p.fires_at(10, 1, 1, 0));
    }

    #[test]
    fn ping_emit_includes_cloudevent_v1_attributes() {
        let mut p = PingSource::new("alpha", "*/5 * * * *");
        p.metadata.name = "tick".to_string();
        p.metadata.namespace = "default".to_string();
        p.spec.data = Some("{\"hello\":\"world\"}".to_string());
        let ev = p.emit("abc-123");
        assert_eq!(ev.id, "abc-123");
        assert_eq!(ev.spec_version, "1.0");
        assert_eq!(ev.event_type, "dev.knative.sources.ping");
        assert_eq!(ev.content_type, "application/json");
        assert!(ev.source.contains("pingsources/tick"));
        assert!(ev.source.contains("namespaces/default"));
        assert_eq!(ev.data.as_deref(), Some("{\"hello\":\"world\"}"));
    }

    #[test]
    fn ping_emit_carries_ce_overrides() {
        let mut p = PingSource::new("t", "*/5 * * * *");
        p.spec.ce_overrides.insert("tenant".to_string(), "alpha".to_string());
        let ev = p.emit("id-1");
        assert_eq!(ev.extensions.get("tenant").map(|s| s.as_str()), Some("alpha"));
    }

    #[test]
    fn ping_resolve_sink_records_status() {
        let mut p = PingSource::new("t", "*/1 * * * *");
        p.spec.sink = Some("https://sink.example/in".to_string());
        assert_eq!(p.resolve_sink(), Some("https://sink.example/in"));
        assert_eq!(p.status.sink_uri.as_deref(), Some("https://sink.example/in"));
    }

    #[test]
    fn ping_bad_schedule_never_fires() {
        let p = PingSource::new("t", "not a cron");
        assert!(!p.fires_at(0, 1, 1, 0));
    }
}
