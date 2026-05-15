// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant scoping helpers (Cortex/Mimir/Loki-compatible `X-Scope-OrgID`).

use crate::models::{Alert, DEFAULT_TENANT, TENANT_LABEL};
use axum::http::HeaderMap;

pub const X_SCOPE_ORG_ID: &str = "X-Scope-OrgID";

/// Pull `X-Scope-OrgID` from the headers, falling back to `DEFAULT_TENANT`.
pub fn tenant_from_headers(headers: &HeaderMap) -> String {
    headers
        .get(X_SCOPE_ORG_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_TENANT.to_string())
}

/// Inject `tenant_id` as a label on every alert.
pub fn inject_tenant(alerts: &mut [Alert], tenant: &str) {
    for a in alerts.iter_mut() {
        a.tenant_id = tenant.to_string();
        a.labels.insert(TENANT_LABEL.into(), tenant.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::collections::HashMap;

    fn alert() -> Alert {
        use chrono::Utc;
        use uuid::Uuid;
        Alert {
            id: Uuid::new_v4(),
            name: "X".into(),
            labels: HashMap::new(),
            annotations: HashMap::new(),
            severity: crate::models::AlertSeverity::Warning,
            state: crate::models::AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: "fp".into(),
            tenant_id: DEFAULT_TENANT.into(),
            generator_url: None,
        }
    }

    #[test]
    fn test_tenant_default_when_missing() {
        let h = HeaderMap::new();
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    #[test]
    fn test_tenant_from_header_value() {
        let mut h = HeaderMap::new();
        h.insert(X_SCOPE_ORG_ID, HeaderValue::from_static("acme"));
        assert_eq!(tenant_from_headers(&h), "acme");
    }

    #[test]
    fn test_tenant_invalid_utf8_falls_back() {
        let mut h = HeaderMap::new();
        h.insert(X_SCOPE_ORG_ID, HeaderValue::from_bytes(b"\xff\xff").unwrap());
        assert_eq!(tenant_from_headers(&h), DEFAULT_TENANT);
    }

    #[test]
    fn test_inject_tenant_sets_label_and_field() {
        let mut alerts = vec![alert(), alert()];
        inject_tenant(&mut alerts, "acme");
        for a in &alerts {
            assert_eq!(a.tenant_id, "acme");
            assert_eq!(a.labels.get(TENANT_LABEL), Some(&"acme".to_string()));
        }
    }
}
