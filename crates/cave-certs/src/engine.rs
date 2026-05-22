// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{CertState, Certificate};
use chrono::Utc;

/// Determine certificate state based on expiry
pub fn compute_cert_state(cert: &Certificate) -> CertState {
    let now = Utc::now();
    if cert.not_after < now {
        CertState::Expired
    } else if cert.not_after < now + chrono::Duration::days(30) {
        CertState::Expiring
    } else {
        CertState::Valid
    }
}

/// Find certificates expiring within `days` days (but not already expired)
pub fn expiring_soon<'a>(certs: &'a [Certificate], days: i64) -> Vec<&'a Certificate> {
    let now = Utc::now();
    let threshold = now + chrono::Duration::days(days);
    certs
        .iter()
        .filter(|c| c.not_after > now && c.not_after <= threshold)
        .collect()
}

/// Count days until expiry (negative if already expired)
pub fn days_until_expiry(cert: &Certificate) -> i64 {
    let now = Utc::now();
    (cert.not_after - now).num_days()
}

/// Check if a certificate covers a given domain (including SAN and wildcards)
pub fn covers_domain(cert: &Certificate, domain: &str) -> bool {
    if cert.domain == domain {
        return true;
    }
    cert.san_domains.iter().any(|san| {
        san == domain || {
            if let Some(wildcard) = san.strip_prefix("*.") {
                domain.ends_with(&format!(".{wildcard}")) || domain == wildcard
            } else {
                false
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CertState;
    use uuid::Uuid;

    fn make_cert(domain: &str, san_domains: Vec<String>, not_after_days: i64) -> Certificate {
        let now = Utc::now();
        Certificate {
            id: Uuid::new_v4(),
            domain: domain.to_string(),
            san_domains,
            issuer: "Let's Encrypt".to_string(),
            not_before: now - chrono::Duration::days(60),
            not_after: now + chrono::Duration::days(not_after_days),
            serial_number: "01:AB".to_string(),
            fingerprint_sha256: "abc123".to_string(),
            state: CertState::Valid,
            auto_renew: true,
        }
    }

    #[test]
    fn test_compute_cert_state_valid() {
        let cert = make_cert("example.com", vec![], 90);
        assert_eq!(compute_cert_state(&cert), CertState::Valid);
    }

    #[test]
    fn test_compute_cert_state_expired() {
        let cert = make_cert("example.com", vec![], -1);
        assert_eq!(compute_cert_state(&cert), CertState::Expired);
    }

    #[test]
    fn test_compute_cert_state_expiring() {
        let cert = make_cert("example.com", vec![], 15);
        assert_eq!(compute_cert_state(&cert), CertState::Expiring);
    }

    #[test]
    fn test_covers_domain_exact() {
        let cert = make_cert("example.com", vec![], 90);
        assert!(covers_domain(&cert, "example.com"));
    }

    #[test]
    fn test_covers_domain_san() {
        let cert = make_cert("example.com", vec!["api.example.com".to_string()], 90);
        assert!(covers_domain(&cert, "api.example.com"));
    }

    #[test]
    fn test_covers_domain_wildcard() {
        let cert = make_cert("example.com", vec!["*.example.com".to_string()], 90);
        assert!(covers_domain(&cert, "sub.example.com"));
        assert!(covers_domain(&cert, "api.example.com"));
    }

    #[test]
    fn test_covers_domain_no_match() {
        let cert = make_cert("example.com", vec![], 90);
        assert!(!covers_domain(&cert, "other.com"));
        assert!(!covers_domain(&cert, "sub.example.com"));
    }

    #[test]
    fn test_expiring_soon_finds_certs() {
        let certs = vec![
            make_cert("a.com", vec![], 10),
            make_cert("b.com", vec![], 60),
        ];
        let soon = expiring_soon(&certs, 30);
        assert_eq!(soon.len(), 1);
        assert_eq!(soon[0].domain, "a.com");
    }
}
