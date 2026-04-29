//! Multi-tenant scoping helpers.
//!
//! The SDK injects a `tenant_id` resource attribute on every span when a
//! tenant has been set on the `TracerProvider`. Inbound RPCs are expected
//! to carry an `X-Scope-OrgID` header (Cortex/Mimir/Loki convention),
//! which the runtime parses with `tenant_from_headers`.

use crate::types::{SpanData, DEFAULT_TENANT, TENANT_LABEL};
use std::collections::HashMap;

pub const X_SCOPE_ORG_ID: &str = "X-Scope-OrgID";

/// Extract the tenant from a header map (case-insensitive lookup).
pub fn tenant_from_headers(headers: &HashMap<String, String>) -> String {
    for (k, v) in headers.iter() {
        if k.eq_ignore_ascii_case(X_SCOPE_ORG_ID) && !v.trim().is_empty() {
            return v.trim().to_string();
        }
    }
    DEFAULT_TENANT.to_string()
}

/// Stamp a tenant onto every span (id field + `tenant_id` resource label).
pub fn inject_tenant(spans: &mut [SpanData], tenant: &str) {
    for s in spans.iter_mut() {
        s.tenant_id = tenant.to_string();
        s.resource.insert(TENANT_LABEL.into(), tenant.into());
    }
}

/// Filter spans down to those owned by `tenant`.
pub fn filter_by_tenant<'a>(spans: &'a [SpanData], tenant: &str) -> Vec<&'a SpanData> {
    spans.iter().filter(|s| s.tenant_id == tenant).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_span(tenant: &str) -> SpanData {
        let now = Utc::now();
        SpanData {
            name: "x".into(),
            context: SpanContext::new(1, 1, true),
            parent_span_id: None,
            kind: SpanKind::Internal,
            start_time: now,
            end_time: now,
            attributes: HashMap::new(),
            events: vec![],
            links: vec![],
            status: Status::Unset,
            instrumentation_scope: "t".into(),
            tenant_id: tenant.into(),
            resource: HashMap::new(),
        }
    }

    #[test]
    fn test_tenant_default_when_header_missing() {
        let h = HashMap::new();
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    #[test]
    fn test_tenant_from_canonical_header() {
        let mut h = HashMap::new();
        h.insert(X_SCOPE_ORG_ID.into(), "acme".into());
        assert_eq!(tenant_from_headers(&h), "acme");
    }

    #[test]
    fn test_tenant_header_lookup_case_insensitive() {
        let mut h = HashMap::new();
        h.insert("x-scope-orgid".into(), "acme".into());
        assert_eq!(tenant_from_headers(&h), "acme");
    }

    #[test]
    fn test_tenant_blank_value_falls_back() {
        let mut h = HashMap::new();
        h.insert(X_SCOPE_ORG_ID.into(), "   ".into());
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    #[test]
    fn test_inject_tenant_sets_field_and_resource() {
        let mut spans = vec![make_span("anonymous"), make_span("anonymous")];
        inject_tenant(&mut spans, "acme");
        for s in &spans {
            assert_eq!(s.tenant_id, "acme");
            assert_eq!(s.resource.get(TENANT_LABEL), Some(&"acme".to_string()));
        }
    }

    #[test]
    fn test_filter_by_tenant_excludes_others() {
        let spans = vec![make_span("acme"), make_span("globex"), make_span("acme")];
        let acme = filter_by_tenant(&spans, "acme");
        assert_eq!(acme.len(), 2);
    }
}
