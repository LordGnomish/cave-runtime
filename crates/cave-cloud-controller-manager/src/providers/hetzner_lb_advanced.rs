// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hetzner Cloud — deeper LB / network / server features.
//!
//! Upstream: `hetznercloud/hcloud-cloud-controller-manager` @
//! [`super::hetzner::PROVIDER_VERSION`]. Covers:
//!
//! * **Sticky cookies** — LB session-stickiness via cookie injection.
//! * **Redirect rules** — HTTP→HTTPS / path rewrites.
//! * **Cert API integration** — managed certificate lifecycle (request
//!   → DNS challenge → issued → renewing).
//! * **Backend cert verification** — skip / strict modes when the LB
//!   talks HTTPS to the backend.
//! * **Rescue mode** — boot images (linux64 / linux32 / freebsd64).
//! * **Console access** — VNC URL + temp password.
//! * **Dual-stack network** — v4 + v6 subnet pair.

use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── Sticky cookies ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbStickyCookie {
    pub name: String,
    pub lifetime_seconds: u32,
    pub secure: bool,
    pub http_only: bool,
}

impl LbStickyCookie {
    pub fn http(name: &str, lifetime: u32) -> Self {
        Self {
            name: name.into(),
            lifetime_seconds: lifetime,
            secure: false,
            http_only: true,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        // RFC 6265 token rules — alpha/digit/'-_' is the practical subset.
        if self.name.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "sticky cookie name must not be empty".into(),
            });
        }
        if !self.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("sticky cookie name {:?} contains illegal characters", self.name),
            });
        }
        if !(60..=86_400).contains(&self.lifetime_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "sticky cookie lifetime {} outside [60, 86400] s",
                    self.lifetime_seconds
                ),
            });
        }
        Ok(())
    }
}

// ─── Redirect rules ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RedirectStatus {
    MovedPermanently301,
    Found302,
    TemporaryRedirect307,
    PermanentRedirect308,
}

impl RedirectStatus {
    pub const fn code(self) -> u16 {
        match self {
            RedirectStatus::MovedPermanently301 => 301,
            RedirectStatus::Found302 => 302,
            RedirectStatus::TemporaryRedirect307 => 307,
            RedirectStatus::PermanentRedirect308 => 308,
        }
    }
    pub const fn preserves_method(self) -> bool {
        matches!(
            self,
            RedirectStatus::TemporaryRedirect307 | RedirectStatus::PermanentRedirect308
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbRedirectRule {
    pub from_path: String,
    pub to_url: String,
    pub status: RedirectStatus,
}

impl LbRedirectRule {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.from_path.starts_with('/') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("redirect from_path {:?} must start with /", self.from_path),
            });
        }
        if !self.to_url.starts_with("https://") && !self.to_url.starts_with("http://") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("redirect to_url {:?} must be http(s)", self.to_url),
            });
        }
        Ok(())
    }
}

// ─── Certificate API integration ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CertIssuanceStatus {
    Requested,
    DnsChallengeIssued,
    Issued,
    Renewing,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedCertRequest {
    pub id: u64,
    pub domains: Vec<String>,
    pub status: CertIssuanceStatus,
    pub challenge_records: Vec<String>,
    pub error_code: Option<String>,
}

impl ManagedCertRequest {
    pub fn requested(id: u64, domains: Vec<String>) -> Self {
        Self {
            id,
            domains,
            status: CertIssuanceStatus::Requested,
            challenge_records: Vec::new(),
            error_code: None,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if self.domains.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "managed cert request must specify at least one domain".into(),
            });
        }
        for d in &self.domains {
            if !d.contains('.') || d.starts_with('.') || d.ends_with('.') {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!("invalid domain {d:?}"),
                });
            }
        }
        Ok(())
    }

    pub fn issue_challenge(&mut self, records: Vec<String>) -> Result<(), CloudError> {
        if self.status != CertIssuanceStatus::Requested {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "cert request {} not in Requested state",
                    self.id
                ),
            });
        }
        self.status = CertIssuanceStatus::DnsChallengeIssued;
        self.challenge_records = records;
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<(), CloudError> {
        if !matches!(
            self.status,
            CertIssuanceStatus::DnsChallengeIssued | CertIssuanceStatus::Renewing
        ) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "cert request {} cannot be finalized from {:?}",
                    self.id, self.status
                ),
            });
        }
        self.status = CertIssuanceStatus::Issued;
        Ok(())
    }

    pub fn begin_renewal(&mut self) -> Result<(), CloudError> {
        if self.status != CertIssuanceStatus::Issued {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("cert request {} cannot renew from {:?}", self.id, self.status),
            });
        }
        self.status = CertIssuanceStatus::Renewing;
        Ok(())
    }

    pub fn fail(&mut self, code: impl Into<String>) {
        self.status = CertIssuanceStatus::Failed;
        self.error_code = Some(code.into());
    }
}

// ─── Backend cert verification ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCertVerification {
    pub skip_verify: bool,
    pub ca_bundle: Option<String>,
}

impl BackendCertVerification {
    pub fn strict(ca_bundle: &str) -> Self {
        Self { skip_verify: false, ca_bundle: Some(ca_bundle.into()) }
    }
    pub fn skip() -> Self {
        Self { skip_verify: true, ca_bundle: None }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.skip_verify && self.ca_bundle.as_deref().unwrap_or("").is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "strict backend verification requires a non-empty ca_bundle".into(),
            });
        }
        if self.skip_verify && self.ca_bundle.is_some() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "skip_verify=true must not carry a ca_bundle".into(),
            });
        }
        Ok(())
    }
}

// ─── Rescue mode ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RescueImage {
    Linux64,
    Linux32,
    Freebsd64,
}

impl RescueImage {
    pub const fn key(self) -> &'static str {
        match self {
            RescueImage::Linux64 => "linux64",
            RescueImage::Linux32 => "linux32",
            RescueImage::Freebsd64 => "freebsd64",
        }
    }
    pub const fn architecture(self) -> &'static str {
        match self {
            RescueImage::Linux64 | RescueImage::Freebsd64 => "x86_64",
            RescueImage::Linux32 => "x86",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RescueModeRequest {
    pub server_id: u64,
    pub image: RescueImage,
    pub ssh_key_ids: Vec<u64>,
}

impl RescueModeRequest {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.ssh_key_ids.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "rescue mode requires at least one ssh key id".into(),
            });
        }
        Ok(())
    }
}

// ─── Console access ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleAccess {
    pub server_id: u64,
    pub vnc_url: String,
    pub password: String,
    pub expires_in_seconds: u32,
}

impl ConsoleAccess {
    pub fn validate(&self) -> Result<(), CloudError> {
        if !self.vnc_url.starts_with("wss://") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("console vnc_url {:?} must be wss://", self.vnc_url),
            });
        }
        if self.password.len() < 8 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "console password must be at least 8 characters".into(),
            });
        }
        if !(60..=3_600).contains(&self.expires_in_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "console expires_in {} outside [60, 3600] s",
                    self.expires_in_seconds
                ),
            });
        }
        Ok(())
    }
}

// ─── Dual-stack network ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DualStackNetwork {
    pub name: String,
    pub v4_subnet: String,
    pub v6_subnet: String,
}

impl DualStackNetwork {
    pub fn validate(&self) -> Result<(), CloudError> {
        let (v4_addr, v4_mask) =
            self.v4_subnet
                .split_once('/')
                .ok_or_else(|| CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!("v4_subnet {:?} not in CIDR form", self.v4_subnet),
                })?;
        if !v4_addr.contains('.') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("v4_subnet {:?} must contain dotted IPv4", self.v4_subnet),
            });
        }
        let _: u8 = v4_mask.parse().map_err(|_| CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: format!("v4_subnet mask {v4_mask:?} not numeric"),
        })?;
        let (v6_addr, v6_mask) =
            self.v6_subnet
                .split_once('/')
                .ok_or_else(|| CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!("v6_subnet {:?} not in CIDR form", self.v6_subnet),
                })?;
        if !v6_addr.contains(':') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("v6_subnet {:?} must contain colon-separated IPv6", self.v6_subnet),
            });
        }
        let _: u8 = v6_mask.parse().map_err(|_| CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: format!("v6_subnet mask {v6_mask:?} not numeric"),
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::hetzner::PROVIDER_VERSION;
    use crate::test_ctx;
    use crate::types::TenantId;

    const REPO: &str = "hetznercloud/hcloud-cloud-controller-manager";

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) -> TenantId {
        let (cite, t) = test_ctx!(ext: REPO, PROVIDER_VERSION, path, sym, tenant);
        assert_eq!(cite.repo, REPO);
        t
    }

    // ─── Sticky cookies ──────────────────────────────────────────────────────

    #[test]
    fn sticky_cookie_http_constructor_is_valid() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "StickySessions");
        assert!(LbStickyCookie::http("HCBALANCERID", 3600).validate().is_ok());
    }

    #[test]
    fn sticky_cookie_name_must_not_be_empty() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "StickySessions");
        let mut c = LbStickyCookie::http("X", 3600);
        c.name.clear();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn sticky_cookie_name_rejects_illegal_characters() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "StickySessions");
        let mut c = LbStickyCookie::http("good", 3600);
        c.name = "with space".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        c.name = "with;semi".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn sticky_cookie_lifetime_outside_60_86400_is_rejected() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "StickySessions");
        let c = LbStickyCookie::http("X", 30);
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let c = LbStickyCookie::http("X", 200_000);
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── Redirect rules ──────────────────────────────────────────────────────

    #[test]
    fn redirect_status_codes_match_http() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "RedirectRule");
        assert_eq!(RedirectStatus::MovedPermanently301.code(), 301);
        assert_eq!(RedirectStatus::Found302.code(), 302);
        assert_eq!(RedirectStatus::TemporaryRedirect307.code(), 307);
        assert_eq!(RedirectStatus::PermanentRedirect308.code(), 308);
    }

    #[test]
    fn redirect_307_and_308_preserve_request_method() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "RedirectRule");
        assert!(RedirectStatus::TemporaryRedirect307.preserves_method());
        assert!(RedirectStatus::PermanentRedirect308.preserves_method());
        assert!(!RedirectStatus::MovedPermanently301.preserves_method());
        assert!(!RedirectStatus::Found302.preserves_method());
    }

    #[test]
    fn redirect_rule_from_path_must_start_with_slash() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "RedirectRule");
        let r = LbRedirectRule {
            from_path: "no-slash".into(),
            to_url: "https://example.com".into(),
            status: RedirectStatus::MovedPermanently301,
        };
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn redirect_rule_to_url_must_be_http_or_https() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "RedirectRule");
        let r = LbRedirectRule {
            from_path: "/old".into(),
            to_url: "ftp://example.com".into(),
            status: RedirectStatus::MovedPermanently301,
        };
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let r = LbRedirectRule {
            from_path: "/old".into(),
            to_url: "https://example.com/new".into(),
            status: RedirectStatus::MovedPermanently301,
        };
        assert!(r.validate().is_ok());
    }

    // ─── Managed cert request lifecycle ──────────────────────────────────────

    fn cert_req() -> ManagedCertRequest {
        ManagedCertRequest::requested(7, vec!["example.com".into(), "www.example.com".into()])
    }

    #[test]
    fn managed_cert_request_validates_domain_list() {
        let _ = ctx("acme", "hcloud/certificate.go", "CreateManaged");
        assert!(cert_req().validate().is_ok());
        let mut bad = cert_req();
        bad.domains = vec![];
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn managed_cert_request_rejects_invalid_domain() {
        let _ = ctx("acme", "hcloud/certificate.go", "CreateManaged");
        let mut bad = cert_req();
        bad.domains = vec!["nodot".into()];
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        bad.domains = vec!["leading.dot.".into()];
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn managed_cert_request_lifecycle_progression() {
        let _ = ctx("acme", "hcloud/certificate.go", "CreateManaged");
        let mut c = cert_req();
        c.issue_challenge(vec!["_acme-challenge.example.com TXT abc".into()]).unwrap();
        assert_eq!(c.status, CertIssuanceStatus::DnsChallengeIssued);
        c.finalize().unwrap();
        assert_eq!(c.status, CertIssuanceStatus::Issued);
        c.begin_renewal().unwrap();
        assert_eq!(c.status, CertIssuanceStatus::Renewing);
        c.finalize().unwrap();
        assert_eq!(c.status, CertIssuanceStatus::Issued);
    }

    #[test]
    fn managed_cert_request_finalize_rejects_unfinalizable_states() {
        let _ = ctx("acme", "hcloud/certificate.go", "FinalizeManaged");
        let mut c = cert_req();
        assert!(matches!(c.finalize().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn managed_cert_request_renew_only_from_issued() {
        let _ = ctx("acme", "hcloud/certificate.go", "RenewManaged");
        let mut c = cert_req();
        assert!(matches!(c.begin_renewal().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn managed_cert_request_fail_records_error_code() {
        let _ = ctx("acme", "hcloud/certificate.go", "CreateManaged");
        let mut c = cert_req();
        c.fail("dns_validation_failed");
        assert_eq!(c.status, CertIssuanceStatus::Failed);
        assert_eq!(c.error_code.as_deref(), Some("dns_validation_failed"));
    }

    // ─── Backend cert verification ───────────────────────────────────────────

    #[test]
    fn backend_cert_verification_strict_requires_ca_bundle() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "BackendVerification");
        assert!(BackendCertVerification::strict("ca-pem-here").validate().is_ok());
        let mut bad = BackendCertVerification::strict("ca-pem-here");
        bad.ca_bundle = Some(String::new());
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn backend_cert_verification_skip_must_not_carry_ca_bundle() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "BackendVerification");
        let mut bad = BackendCertVerification::skip();
        bad.ca_bundle = Some("x".into());
        assert!(matches!(bad.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let good = BackendCertVerification::skip();
        assert!(good.validate().is_ok());
    }

    // ─── Rescue mode ─────────────────────────────────────────────────────────

    #[test]
    fn rescue_image_keys_match_api_strings() {
        let _ = ctx("acme", "hcloud/server.go", "EnableRescue");
        assert_eq!(RescueImage::Linux64.key(), "linux64");
        assert_eq!(RescueImage::Linux32.key(), "linux32");
        assert_eq!(RescueImage::Freebsd64.key(), "freebsd64");
    }

    #[test]
    fn rescue_image_architecture_matches_image() {
        let _ = ctx("acme", "hcloud/server.go", "EnableRescue");
        assert_eq!(RescueImage::Linux64.architecture(), "x86_64");
        assert_eq!(RescueImage::Linux32.architecture(), "x86");
        assert_eq!(RescueImage::Freebsd64.architecture(), "x86_64");
    }

    #[test]
    fn rescue_mode_request_requires_ssh_key() {
        let _ = ctx("acme", "hcloud/server.go", "EnableRescue");
        let r = RescueModeRequest { server_id: 7, image: RescueImage::Linux64, ssh_key_ids: vec![] };
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let r = RescueModeRequest { server_id: 7, image: RescueImage::Linux64, ssh_key_ids: vec![1] };
        assert!(r.validate().is_ok());
    }

    // ─── Console access ──────────────────────────────────────────────────────

    fn console() -> ConsoleAccess {
        ConsoleAccess {
            server_id: 7,
            vnc_url: "wss://console.hetzner.cloud/vnc/abc".into(),
            password: "secret123".into(),
            expires_in_seconds: 600,
        }
    }

    #[test]
    fn console_access_validates_minimum_config() {
        let _ = ctx("acme", "hcloud/server.go", "RequestConsole");
        assert!(console().validate().is_ok());
    }

    #[test]
    fn console_access_rejects_non_wss_url() {
        let _ = ctx("acme", "hcloud/server.go", "RequestConsole");
        let mut c = console();
        c.vnc_url = "ws://insecure".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn console_access_rejects_short_password() {
        let _ = ctx("acme", "hcloud/server.go", "RequestConsole");
        let mut c = console();
        c.password = "short".into();
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn console_access_rejects_expiry_outside_60_3600() {
        let _ = ctx("acme", "hcloud/server.go", "RequestConsole");
        let mut c = console();
        c.expires_in_seconds = 30;
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        c.expires_in_seconds = 7_200;
        assert!(matches!(c.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── Dual-stack network ──────────────────────────────────────────────────

    fn dsn() -> DualStackNetwork {
        DualStackNetwork {
            name: "k8s".into(),
            v4_subnet: "10.0.0.0/24".into(),
            v6_subnet: "2001:db8::/64".into(),
        }
    }

    #[test]
    fn dual_stack_network_default_validates() {
        let _ = ctx("acme", "hcloud/network.go", "DualStack");
        assert!(dsn().validate().is_ok());
    }

    #[test]
    fn dual_stack_network_rejects_v4_without_dots() {
        let _ = ctx("acme", "hcloud/network.go", "DualStack");
        let mut n = dsn();
        n.v4_subnet = "abc/24".into();
        assert!(matches!(n.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn dual_stack_network_rejects_v6_without_colons() {
        let _ = ctx("acme", "hcloud/network.go", "DualStack");
        let mut n = dsn();
        n.v6_subnet = "10.0.0.0/24".into();
        assert!(matches!(n.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn dual_stack_network_rejects_missing_mask() {
        let _ = ctx("acme", "hcloud/network.go", "DualStack");
        let mut n = dsn();
        n.v4_subnet = "10.0.0.0".into();
        assert!(matches!(n.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        let mut n = dsn();
        n.v6_subnet = "2001:db8::".into();
        assert!(matches!(n.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn dual_stack_network_rejects_non_numeric_mask() {
        let _ = ctx("acme", "hcloud/network.go", "DualStack");
        let mut n = dsn();
        n.v4_subnet = "10.0.0.0/twenty-four".into();
        assert!(matches!(n.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }
}
