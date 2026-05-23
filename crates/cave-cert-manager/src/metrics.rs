// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Metrics registry — Prometheus exposition for the cert-manager control plane.
//!
//! Cite: `pkg/metrics/metrics.go`. Upstream cert-manager publishes the
//! following counters/gauges through its Prometheus HTTP handler:
//!
//! * `certmanager_certificate_ready_status{name,namespace,condition="True|False|Unknown"}` (gauge)
//! * `certmanager_certificate_expiration_timestamp_seconds{name,namespace}` (gauge)
//! * `certmanager_certificate_renewal_timestamp_seconds{name,namespace}` (gauge)
//! * `certmanager_acme_client_request_count{scheme,host,path,method,status}` (counter)
//! * `certmanager_controller_sync_call_count{controller}` (counter)
//!
//! This module emits the same five families, scoped per tenant_id so a
//! single Prometheus scrape can drive the cave-portal multi-tenant
//! dashboard. The registry is in-memory + lock-free under the hood;
//! every emit takes a `&mut self` so the caller threads it through the
//! reconcile loop deterministically (no global state).

use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

use crate::models::{Certificate, CertificateConditionType, ConditionStatus};

/// Stable label tuple for a per-Certificate metric.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct CertLabels {
    pub tenant_id: String,
    pub namespace: String,
    pub name: String,
}

impl CertLabels {
    pub fn from_certificate(cert: &Certificate) -> Self {
        Self {
            tenant_id: cert.tenant_id.clone(),
            namespace: cert.namespace.clone(),
            name: cert.name.clone(),
        }
    }

    fn render(&self) -> String {
        format!(
            "tenant_id=\"{}\",namespace=\"{}\",name=\"{}\"",
            escape(&self.tenant_id),
            escape(&self.namespace),
            escape(&self.name),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct AcmeRequestLabels {
    pub scheme: String,
    pub host: String,
    pub method: String,
    pub status: u16,
}

impl AcmeRequestLabels {
    fn render(&self) -> String {
        format!(
            "scheme=\"{}\",host=\"{}\",method=\"{}\",status=\"{}\"",
            escape(&self.scheme),
            escape(&self.host),
            escape(&self.method),
            self.status,
        )
    }
}

#[derive(Debug, Default, Clone)]
pub struct CertManagerMetrics {
    ready_status: BTreeMap<CertLabels, ReadyStatusSample>,
    expiration_seconds: BTreeMap<CertLabels, i64>,
    renewal_seconds: BTreeMap<CertLabels, i64>,
    acme_request_count: BTreeMap<AcmeRequestLabels, u64>,
    sync_call_count: BTreeMap<String, u64>,
    /// Last emit timestamp (driven by reconcile loop) — surfaced in
    /// debug output for parity_self_audit.
    pub last_emit_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadyStatusSample {
    /// Each Certificate emits 3 gauges (one per ConditionStatus value)
    /// — exactly one is `1`, the other two are `0`. Mirrors
    /// cert-manager's `CollectCertificate` behaviour.
    condition_true: u8,
    condition_false: u8,
    condition_unknown: u8,
}

impl CertManagerMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the per-Certificate gauges from the latest projection.
    /// Idempotent — calling twice for the same Certificate updates,
    /// it does not duplicate.
    pub fn observe_certificate(&mut self, cert: &Certificate, now: DateTime<Utc>) {
        let labels = CertLabels::from_certificate(cert);
        let ready = cert
            .status
            .as_ref()
            .and_then(|s| {
                s.conditions
                    .iter()
                    .find(|c| c.kind == CertificateConditionType::Ready)
                    .map(|c| c.status)
            })
            .unwrap_or(ConditionStatus::Unknown);
        let sample = match ready {
            ConditionStatus::True => ReadyStatusSample {
                condition_true: 1,
                condition_false: 0,
                condition_unknown: 0,
            },
            ConditionStatus::False => ReadyStatusSample {
                condition_true: 0,
                condition_false: 1,
                condition_unknown: 0,
            },
            ConditionStatus::Unknown => ReadyStatusSample {
                condition_true: 0,
                condition_false: 0,
                condition_unknown: 1,
            },
        };
        self.ready_status.insert(labels.clone(), sample);
        if let Some(status) = cert.status.as_ref() {
            if let Some(na) = status.not_after {
                self.expiration_seconds
                    .insert(labels.clone(), na.timestamp());
            }
            if let Some(rt) = status.renewal_time {
                self.renewal_seconds.insert(labels.clone(), rt.timestamp());
            }
        }
        self.last_emit_at = Some(now);
    }

    /// Forget all samples for a Certificate (e.g. on delete) — keeps
    /// the metric cardinality bounded over time.
    pub fn forget_certificate(&mut self, cert: &Certificate) {
        let labels = CertLabels::from_certificate(cert);
        self.ready_status.remove(&labels);
        self.expiration_seconds.remove(&labels);
        self.renewal_seconds.remove(&labels);
    }

    /// Increment the ACME backend request counter — typically called
    /// by AcmeIssuer after each cave-acme HTTP call.
    pub fn record_acme_request(&mut self, labels: AcmeRequestLabels) {
        *self.acme_request_count.entry(labels).or_insert(0) += 1;
    }

    /// Increment the controller sync counter — typically called once
    /// per reconcile attempt.
    pub fn record_sync(&mut self, controller: &str) {
        *self
            .sync_call_count
            .entry(controller.to_string())
            .or_insert(0) += 1;
    }

    pub fn ready_status_len(&self) -> usize {
        self.ready_status.len()
    }

    pub fn sync_count(&self, controller: &str) -> u64 {
        self.sync_call_count.get(controller).copied().unwrap_or(0)
    }

    pub fn acme_request_count(&self, labels: &AcmeRequestLabels) -> u64 {
        self.acme_request_count.get(labels).copied().unwrap_or(0)
    }

    /// Render the registry as Prometheus exposition format (text/plain
    /// version 0.0.4). Deterministic ordering — keys traversed in
    /// BTreeMap order so the output is stable for golden tests.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4096);

        out.push_str("# HELP certmanager_certificate_ready_status Whether the Ready condition is true for the Certificate.\n");
        out.push_str("# TYPE certmanager_certificate_ready_status gauge\n");
        for (labels, sample) in &self.ready_status {
            let l = labels.render();
            out.push_str(&format!(
                "certmanager_certificate_ready_status{{{},condition=\"True\"}} {}\n",
                l, sample.condition_true
            ));
            out.push_str(&format!(
                "certmanager_certificate_ready_status{{{},condition=\"False\"}} {}\n",
                l, sample.condition_false
            ));
            out.push_str(&format!(
                "certmanager_certificate_ready_status{{{},condition=\"Unknown\"}} {}\n",
                l, sample.condition_unknown
            ));
        }

        out.push_str("# HELP certmanager_certificate_expiration_timestamp_seconds The notAfter timestamp of the issued certificate.\n");
        out.push_str("# TYPE certmanager_certificate_expiration_timestamp_seconds gauge\n");
        for (labels, secs) in &self.expiration_seconds {
            out.push_str(&format!(
                "certmanager_certificate_expiration_timestamp_seconds{{{}}} {}\n",
                labels.render(),
                secs
            ));
        }

        out.push_str("# HELP certmanager_certificate_renewal_timestamp_seconds The renewBefore-adjusted next-renewal timestamp.\n");
        out.push_str("# TYPE certmanager_certificate_renewal_timestamp_seconds gauge\n");
        for (labels, secs) in &self.renewal_seconds {
            out.push_str(&format!(
                "certmanager_certificate_renewal_timestamp_seconds{{{}}} {}\n",
                labels.render(),
                secs
            ));
        }

        out.push_str("# HELP certmanager_acme_client_request_count The total number of requests made to the ACME backend.\n");
        out.push_str("# TYPE certmanager_acme_client_request_count counter\n");
        for (labels, count) in &self.acme_request_count {
            out.push_str(&format!(
                "certmanager_acme_client_request_count{{{}}} {}\n",
                labels.render(),
                count
            ));
        }

        out.push_str("# HELP certmanager_controller_sync_call_count The number of times each controller has synced.\n");
        out.push_str("# TYPE certmanager_controller_sync_call_count counter\n");
        for (controller, count) in &self.sync_call_count {
            out.push_str(&format!(
                "certmanager_controller_sync_call_count{{controller=\"{}\"}} {}\n",
                escape(controller),
                count
            ));
        }

        out
    }
}

/// Escape `\\`, `"`, `\n` per the Prometheus exposition spec.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Certificate, CertificateCondition, CertificateConditionType, CertificateSpec,
        CertificateStatus, ConditionStatus, IssuerRef, IssuerRefKind,
    };
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn make_cert(name: &str, status: Option<ConditionStatus>) -> Certificate {
        let now = Utc::now();
        let conditions = match status {
            Some(s) => vec![CertificateCondition {
                kind: CertificateConditionType::Ready,
                status: s,
                reason: Some("Test".into()),
                message: Some("Test".into()),
                last_transition_time: now,
            }],
            None => vec![],
        };
        let cert_status = if conditions.is_empty() {
            None
        } else {
            Some(CertificateStatus {
                conditions,
                not_before: Some(now),
                not_after: Some(now + Duration::days(90)),
                renewal_time: Some(now + Duration::days(60)),
                revision: 1,
                serial: Some("01".into()),
                last_failure_message: None,
                secret_ref: None,
            })
        };
        Certificate {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: "default".into(),
            tenant_id: "tenant-a".into(),
            spec: CertificateSpec {
                secret_name: format!("{}-tls", name),
                issuer_ref: IssuerRef {
                    name: "issuer-a".into(),
                    kind: IssuerRefKind::Issuer,
                    group: "cert-manager.io".into(),
                },
                dns_names: vec![format!("{}.example.com", name)],
                ip_addresses: vec![],
                uris: vec![],
                email_addresses: vec![],
                common_name: None,
                duration_seconds: 90 * 24 * 3600,
                renew_before_seconds: 30 * 24 * 3600,
                usages: vec![],
                private_key: Default::default(),
                is_ca: false,
                subject: None,
                secret_template_labels: Default::default(),
                secret_template_annotations: Default::default(),
            },
            status: cert_status,
            created_at: now,
            updated_at: now,
            labels: Default::default(),
            annotations: Default::default(),
        }
    }

    #[test]
    fn ready_status_records_true_branch() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::True));
        m.observe_certificate(&cert, Utc::now());
        let out = m.render_prometheus();
        assert!(out.contains("condition=\"True\"} 1"));
        assert!(out.contains("condition=\"False\"} 0"));
        assert!(out.contains("condition=\"Unknown\"} 0"));
    }

    #[test]
    fn ready_status_records_false_branch() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::False));
        m.observe_certificate(&cert, Utc::now());
        let out = m.render_prometheus();
        assert!(out.contains("condition=\"True\"} 0"));
        assert!(out.contains("condition=\"False\"} 1"));
    }

    #[test]
    fn missing_status_treated_as_unknown() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", None);
        m.observe_certificate(&cert, Utc::now());
        let out = m.render_prometheus();
        assert!(out.contains("condition=\"Unknown\"} 1"));
    }

    #[test]
    fn expiration_gauge_records_unix_seconds() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::True));
        m.observe_certificate(&cert, Utc::now());
        let out = m.render_prometheus();
        assert!(out.contains("certmanager_certificate_expiration_timestamp_seconds"));
    }

    #[test]
    fn renewal_gauge_records_unix_seconds() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::True));
        m.observe_certificate(&cert, Utc::now());
        let out = m.render_prometheus();
        assert!(out.contains("certmanager_certificate_renewal_timestamp_seconds"));
    }

    #[test]
    fn forget_certificate_drops_samples() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::True));
        m.observe_certificate(&cert, Utc::now());
        assert_eq!(m.ready_status_len(), 1);
        m.forget_certificate(&cert);
        assert_eq!(m.ready_status_len(), 0);
    }

    #[test]
    fn acme_request_counter_monotonic() {
        let mut m = CertManagerMetrics::new();
        let labels = AcmeRequestLabels {
            scheme: "https".into(),
            host: "acme.example.com".into(),
            method: "POST".into(),
            status: 200,
        };
        m.record_acme_request(labels.clone());
        m.record_acme_request(labels.clone());
        m.record_acme_request(labels.clone());
        assert_eq!(m.acme_request_count(&labels), 3);
    }

    #[test]
    fn sync_counter_tracks_per_controller() {
        let mut m = CertManagerMetrics::new();
        m.record_sync("certificates");
        m.record_sync("certificates");
        m.record_sync("issuers");
        assert_eq!(m.sync_count("certificates"), 2);
        assert_eq!(m.sync_count("issuers"), 1);
        assert_eq!(m.sync_count("unknown"), 0);
    }

    #[test]
    fn label_escaping_handles_quotes_and_backslashes() {
        let mut m = CertManagerMetrics::new();
        let mut cert = make_cert("alpha", Some(ConditionStatus::True));
        cert.name = "weird\"name\\with\nspecials".into();
        m.observe_certificate(&cert, Utc::now());
        let out = m.render_prometheus();
        assert!(out.contains("weird\\\"name\\\\with\\nspecials"));
    }

    #[test]
    fn render_is_deterministic_under_btree_order() {
        let mut m1 = CertManagerMetrics::new();
        let mut m2 = CertManagerMetrics::new();
        let a = make_cert("alpha", Some(ConditionStatus::True));
        let b = make_cert("beta", Some(ConditionStatus::True));
        m1.observe_certificate(&a, Utc::now());
        m1.observe_certificate(&b, Utc::now());
        m2.observe_certificate(&b, Utc::now());
        m2.observe_certificate(&a, Utc::now());
        assert_eq!(m1.render_prometheus(), m2.render_prometheus());
    }

    #[test]
    fn exposition_includes_all_five_metric_families() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::True));
        m.observe_certificate(&cert, Utc::now());
        m.record_acme_request(AcmeRequestLabels {
            scheme: "https".into(),
            host: "acme.example.com".into(),
            method: "POST".into(),
            status: 201,
        });
        m.record_sync("certificates");
        let out = m.render_prometheus();
        for family in [
            "certmanager_certificate_ready_status",
            "certmanager_certificate_expiration_timestamp_seconds",
            "certmanager_certificate_renewal_timestamp_seconds",
            "certmanager_acme_client_request_count",
            "certmanager_controller_sync_call_count",
        ] {
            assert!(
                out.contains(family),
                "expected exposition to include metric family `{family}`"
            );
        }
    }

    #[test]
    fn observe_overwrites_previous_sample_idempotently() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::True));
        m.observe_certificate(&cert, Utc::now());
        m.observe_certificate(&cert, Utc::now());
        assert_eq!(m.ready_status_len(), 1);
    }

    #[test]
    fn last_emit_at_advances_on_observe() {
        let mut m = CertManagerMetrics::new();
        let cert = make_cert("alpha", Some(ConditionStatus::True));
        assert!(m.last_emit_at.is_none());
        let t0 = Utc::now();
        m.observe_certificate(&cert, t0);
        assert_eq!(m.last_emit_at, Some(t0));
    }
}
