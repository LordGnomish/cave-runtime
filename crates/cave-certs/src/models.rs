use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Certificate {
    pub id: Uuid,
    pub domain: String,
    pub san_domains: Vec<String>,
    pub issuer: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub serial_number: String,
    pub fingerprint_sha256: String,
    pub state: CertState,
    pub auto_renew: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CertState {
    Valid,
    Expiring,
    Expired,
    Pending,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cert(state: CertState) -> Certificate {
        let now = Utc::now();
        Certificate {
            id: Uuid::new_v4(),
            domain: "example.com".to_string(),
            san_domains: vec!["www.example.com".to_string()],
            issuer: "Let's Encrypt".to_string(),
            not_before: now - chrono::Duration::days(60),
            not_after: now + chrono::Duration::days(30),
            serial_number: "01:AB:CD".to_string(),
            fingerprint_sha256: "deadbeef".to_string(),
            state,
            auto_renew: true,
        }
    }

    #[test]
    fn test_certificate_roundtrip() {
        let cert = make_cert(CertState::Valid);
        let json = serde_json::to_string(&cert).unwrap();
        let decoded: Certificate = serde_json::from_str(&json).unwrap();
        assert_eq!(cert, decoded);
    }

    #[test]
    fn test_cert_state_serde_names() {
        let state = CertState::Expiring;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"expiring\"");
        let decoded: CertState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn test_cert_state_expired_roundtrip() {
        let state = CertState::Expired;
        let json = serde_json::to_string(&state).unwrap();
        let decoded: CertState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn test_certificate_empty_san_roundtrip() {
        let mut cert = make_cert(CertState::Pending);
        cert.san_domains = vec![];
        let json = serde_json::to_string(&cert).unwrap();
        let decoded: Certificate = serde_json::from_str(&json).unwrap();
        assert!(decoded.san_domains.is_empty());
    }

    #[test]
    fn test_certificate_failed_state_roundtrip() {
        let cert = make_cert(CertState::Failed);
        let json = serde_json::to_string(&cert).unwrap();
        let decoded: Certificate = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.state, CertState::Failed);
        assert!(decoded.auto_renew);
    }
}
