// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sampler implementations matching the OpenTelemetry SDK spec:
//! AlwaysOn, AlwaysOff, TraceIdRatioBased, ParentBased, and a TailSampler
//! used by the post-export pipeline.

use crate::types::{Attributes, SpanContext, SpanData, SpanKind, TraceId};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingDecision {
    /// Don't record the span and don't propagate `sampled = true`.
    Drop,
    /// Record the span but don't export it (in-process tracing only).
    RecordOnly,
    /// Record and export the span; downstream services see `sampled = true`.
    RecordAndSample,
}

impl SamplingDecision {
    pub fn is_sampled(&self) -> bool {
        matches!(self, SamplingDecision::RecordAndSample)
    }
    pub fn is_recording(&self) -> bool {
        !matches!(self, SamplingDecision::Drop)
    }
}

#[derive(Debug, Clone)]
pub struct SamplingResult {
    pub decision: SamplingDecision,
    /// Additional attributes the sampler wants attached to the span.
    pub attributes: Attributes,
    /// Trace flags to propagate (W3C trace_flags byte).
    pub trace_flags: u8,
}

pub trait Sampler: Send + Sync {
    fn name(&self) -> &'static str;
    fn should_sample(
        &self,
        parent_context: Option<&SpanContext>,
        trace_id: TraceId,
        name: &str,
        kind: SpanKind,
        attributes: &Attributes,
    ) -> SamplingResult;
}

// ─── AlwaysOn ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysOn;

impl Sampler for AlwaysOn {
    fn name(&self) -> &'static str { "AlwaysOn" }
    fn should_sample(
        &self,
        _parent_context: Option<&SpanContext>,
        _trace_id: TraceId,
        _name: &str,
        _kind: SpanKind,
        _attributes: &Attributes,
    ) -> SamplingResult {
        SamplingResult {
            decision: SamplingDecision::RecordAndSample,
            attributes: Attributes::new(),
            trace_flags: SpanContext::FLAG_SAMPLED,
        }
    }
}

// ─── AlwaysOff ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysOff;

impl Sampler for AlwaysOff {
    fn name(&self) -> &'static str { "AlwaysOff" }
    fn should_sample(
        &self,
        _parent_context: Option<&SpanContext>,
        _trace_id: TraceId,
        _name: &str,
        _kind: SpanKind,
        _attributes: &Attributes,
    ) -> SamplingResult {
        SamplingResult {
            decision: SamplingDecision::Drop,
            attributes: Attributes::new(),
            trace_flags: 0,
        }
    }
}

// ─── TraceIdRatioBased ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct TraceIdRatioBased {
    threshold: u64,
}

impl TraceIdRatioBased {
    /// `ratio` clamped to [0.0, 1.0].
    pub fn new(ratio: f64) -> Self {
        let ratio = ratio.clamp(0.0, 1.0);
        // Spec uses upper 64 bits of trace_id < ratio * 2^64
        let threshold = if ratio >= 1.0 {
            u64::MAX
        } else if ratio <= 0.0 {
            0
        } else {
            (ratio * (u64::MAX as f64)) as u64
        };
        TraceIdRatioBased { threshold }
    }

    pub fn ratio(&self) -> f64 {
        if self.threshold == 0 { 0.0 }
        else if self.threshold == u64::MAX { 1.0 }
        else { self.threshold as f64 / u64::MAX as f64 }
    }
}

impl Sampler for TraceIdRatioBased {
    fn name(&self) -> &'static str { "TraceIdRatioBased" }
    fn should_sample(
        &self,
        _parent_context: Option<&SpanContext>,
        trace_id: TraceId,
        _name: &str,
        _kind: SpanKind,
        _attributes: &Attributes,
    ) -> SamplingResult {
        let sampled = if self.threshold == u64::MAX {
            true // ratio = 1.0 → always
        } else if self.threshold == 0 {
            false // ratio = 0.0 → never
        } else {
            let upper = (trace_id >> 64) as u64;
            upper < self.threshold
        };
        SamplingResult {
            decision: if sampled {
                SamplingDecision::RecordAndSample
            } else {
                SamplingDecision::Drop
            },
            attributes: Attributes::new(),
            trace_flags: if sampled { SpanContext::FLAG_SAMPLED } else { 0 },
        }
    }
}

// ─── ParentBased ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ParentBased {
    pub root: Arc<dyn Sampler>,
    pub remote_parent_sampled: Arc<dyn Sampler>,
    pub remote_parent_not_sampled: Arc<dyn Sampler>,
    pub local_parent_sampled: Arc<dyn Sampler>,
    pub local_parent_not_sampled: Arc<dyn Sampler>,
}

impl ParentBased {
    /// Convenience constructor: use `root` for non-parent spans, mirror
    /// the parent's sampled flag otherwise.
    pub fn new(root: Arc<dyn Sampler>) -> Self {
        ParentBased {
            root,
            remote_parent_sampled: Arc::new(AlwaysOn),
            remote_parent_not_sampled: Arc::new(AlwaysOff),
            local_parent_sampled: Arc::new(AlwaysOn),
            local_parent_not_sampled: Arc::new(AlwaysOff),
        }
    }
}

impl Sampler for ParentBased {
    fn name(&self) -> &'static str { "ParentBased" }
    fn should_sample(
        &self,
        parent_context: Option<&SpanContext>,
        trace_id: TraceId,
        name: &str,
        kind: SpanKind,
        attributes: &Attributes,
    ) -> SamplingResult {
        match parent_context {
            None => self.root.should_sample(None, trace_id, name, kind, attributes),
            Some(ctx) => {
                let s = match (ctx.is_remote, ctx.is_sampled()) {
                    (true, true) => &self.remote_parent_sampled,
                    (true, false) => &self.remote_parent_not_sampled,
                    (false, true) => &self.local_parent_sampled,
                    (false, false) => &self.local_parent_not_sampled,
                };
                s.should_sample(parent_context, trace_id, name, kind, attributes)
            }
        }
    }
}

// ─── TailSampler ──────────────────────────────────────────────────────────

/// Post-export decision: keep a span if any of its policies match.
/// Used after the head sampler to retain interesting low-probability traces.
pub struct TailSampler {
    policies: Vec<Box<dyn TailPolicy>>,
}

pub trait TailPolicy: Send + Sync {
    fn should_keep(&self, span: &SpanData) -> bool;
}

impl TailSampler {
    pub fn new(policies: Vec<Box<dyn TailPolicy>>) -> Self {
        TailSampler { policies }
    }

    pub fn should_keep(&self, span: &SpanData) -> bool {
        self.policies.iter().any(|p| p.should_keep(span))
    }
}

/// Keep spans with a non-OK status.
pub struct ErrorPolicy;
impl TailPolicy for ErrorPolicy {
    fn should_keep(&self, span: &SpanData) -> bool {
        matches!(span.status, crate::types::Status::Error(_))
    }
}

/// Keep spans whose duration >= threshold.
pub struct LatencyPolicy {
    pub threshold_ms: i64,
}
impl TailPolicy for LatencyPolicy {
    fn should_keep(&self, span: &SpanData) -> bool {
        span.duration().num_milliseconds() >= self.threshold_ms
    }
}

/// Keep spans whose attribute matches an expected string value.
pub struct AttrEqualPolicy {
    pub key: String,
    pub value: String,
}
impl TailPolicy for AttrEqualPolicy {
    fn should_keep(&self, span: &SpanData) -> bool {
        match span.attributes.get(&self.key) {
            Some(crate::types::AttrValue::String(s)) => s == &self.value,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AttrValue, Status};
    use chrono::Utc;
    use std::collections::HashMap;

    fn empty_attrs() -> Attributes { Attributes::new() }

    fn span(name: &str) -> SpanData {
        let now = Utc::now();
        SpanData {
            name: name.into(),
            context: SpanContext::new(0xdeadbeef, 0xcafe, true),
            parent_span_id: None,
            kind: SpanKind::Internal,
            start_time: now,
            end_time: now + chrono::Duration::milliseconds(100),
            attributes: HashMap::new(),
            events: vec![],
            links: vec![],
            status: Status::Unset,
            instrumentation_scope: "test".into(),
            tenant_id: "anonymous".into(),
            resource: HashMap::new(),
        }
    }

    #[test]
    fn test_always_on_decision() {
        let r = AlwaysOn.should_sample(None, 1, "x", SpanKind::Internal, &empty_attrs());
        assert_eq!(r.decision, SamplingDecision::RecordAndSample);
        assert!(r.decision.is_sampled());
        assert!(r.decision.is_recording());
    }

    #[test]
    fn test_always_off_decision() {
        let r = AlwaysOff.should_sample(None, 1, "x", SpanKind::Internal, &empty_attrs());
        assert_eq!(r.decision, SamplingDecision::Drop);
        assert!(!r.decision.is_sampled());
        assert!(!r.decision.is_recording());
    }

    #[test]
    fn test_ratio_zero_drops_all() {
        let s = TraceIdRatioBased::new(0.0);
        for tid in [1u128, u128::MAX, 0xdeadbeef] {
            let r = s.should_sample(None, tid, "x", SpanKind::Internal, &empty_attrs());
            assert_eq!(r.decision, SamplingDecision::Drop, "tid {}", tid);
        }
    }

    #[test]
    fn test_ratio_one_samples_all() {
        let s = TraceIdRatioBased::new(1.0);
        for tid in [1u128, u128::MAX, 0xdeadbeef] {
            let r = s.should_sample(None, tid, "x", SpanKind::Internal, &empty_attrs());
            assert_eq!(r.decision, SamplingDecision::RecordAndSample, "tid {}", tid);
        }
    }

    #[test]
    fn test_ratio_round_trip() {
        let s = TraceIdRatioBased::new(0.25);
        assert!((s.ratio() - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_ratio_clamp_negative_to_zero() {
        let s = TraceIdRatioBased::new(-1.0);
        assert_eq!(s.ratio(), 0.0);
    }

    #[test]
    fn test_ratio_clamp_above_one() {
        let s = TraceIdRatioBased::new(2.0);
        assert_eq!(s.ratio(), 1.0);
    }

    #[test]
    fn test_ratio_decision_bias() {
        // Distribution check: with ratio = 0.5, ~half of u128 trace IDs
        // (varied upper-64-bit) should sample.
        let s = TraceIdRatioBased::new(0.5);
        let mut sampled = 0;
        for i in 0..1000u64 {
            let tid: TraceId = (i as u128) << 64;
            let r = s.should_sample(None, tid, "x", SpanKind::Internal, &empty_attrs());
            if r.decision == SamplingDecision::RecordAndSample { sampled += 1; }
        }
        // First 500 IDs (upper64 = 0..499) all under 50% of u64::MAX → all sampled.
        assert_eq!(sampled, 1000);

        // Now flip: use upper64 in the top half → none sampled.
        let mut sampled2 = 0;
        for i in (u64::MAX/2 + 1)..(u64::MAX/2 + 1001) {
            let tid: TraceId = (i as u128) << 64;
            let r = s.should_sample(None, tid, "x", SpanKind::Internal, &empty_attrs());
            if r.decision == SamplingDecision::RecordAndSample { sampled2 += 1; }
        }
        assert_eq!(sampled2, 0);
    }

    #[test]
    fn test_parent_based_root_uses_root_sampler() {
        let pb = ParentBased::new(Arc::new(AlwaysOff));
        let r = pb.should_sample(None, 1, "x", SpanKind::Internal, &empty_attrs());
        assert_eq!(r.decision, SamplingDecision::Drop);
    }

    #[test]
    fn test_parent_based_remote_sampled_inherits() {
        let pb = ParentBased::new(Arc::new(AlwaysOff));
        let mut ctx = SpanContext::new(1, 1, true);
        ctx.is_remote = true;
        let r = pb.should_sample(Some(&ctx), 1, "x", SpanKind::Internal, &empty_attrs());
        assert_eq!(r.decision, SamplingDecision::RecordAndSample);
    }

    #[test]
    fn test_parent_based_remote_not_sampled_inherits() {
        let pb = ParentBased::new(Arc::new(AlwaysOn));
        let mut ctx = SpanContext::new(1, 1, false);
        ctx.is_remote = true;
        let r = pb.should_sample(Some(&ctx), 1, "x", SpanKind::Internal, &empty_attrs());
        assert_eq!(r.decision, SamplingDecision::Drop);
    }

    #[test]
    fn test_parent_based_local_sampled_inherits() {
        let pb = ParentBased::new(Arc::new(AlwaysOff));
        let ctx = SpanContext::new(1, 1, true); // local (is_remote=false default)
        let r = pb.should_sample(Some(&ctx), 1, "x", SpanKind::Internal, &empty_attrs());
        assert_eq!(r.decision, SamplingDecision::RecordAndSample);
    }

    #[test]
    fn test_tail_error_policy() {
        let mut s = span("x");
        s.status = Status::Error("boom".into());
        let t = TailSampler::new(vec![Box::new(ErrorPolicy)]);
        assert!(t.should_keep(&s));

        s.status = Status::Ok;
        assert!(!t.should_keep(&s));
    }

    #[test]
    fn test_tail_latency_policy() {
        let s = span("x");
        let fast = TailSampler::new(vec![Box::new(LatencyPolicy { threshold_ms: 50 })]);
        assert!(fast.should_keep(&s)); // 100ms >= 50ms
        let slow = TailSampler::new(vec![Box::new(LatencyPolicy { threshold_ms: 500 })]);
        assert!(!slow.should_keep(&s));
    }

    #[test]
    fn test_tail_attr_equal_policy() {
        let mut s = span("x");
        s.attributes.insert("priority".into(), AttrValue::String("high".into()));
        let t = TailSampler::new(vec![Box::new(AttrEqualPolicy {
            key: "priority".into(),
            value: "high".into(),
        })]);
        assert!(t.should_keep(&s));
    }

    #[test]
    fn test_tail_disjunction_of_policies() {
        let s = span("x");
        let t = TailSampler::new(vec![
            Box::new(ErrorPolicy),                         // false: status Unset
            Box::new(LatencyPolicy { threshold_ms: 50 }),  // true: 100ms >= 50ms
        ]);
        assert!(t.should_keep(&s));
    }

    #[test]
    fn test_tail_no_policies_drops() {
        let s = span("x");
        let t = TailSampler::new(vec![]);
        assert!(!t.should_keep(&s));
    }
}
