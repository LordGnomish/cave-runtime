// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hetzner Cloud LoadBalancer programming model.
//!
//! Upstream: `hetznercloud/hcloud-cloud-controller-manager` @
//! [`super::hetzner::PROVIDER_VERSION`].
//!
//! Captures the bits an `EnsureLoadBalancer` call has to push beyond a bare
//! IP allocation:
//!
//! * **Algorithm** — the LB's load-distribution algorithm
//!   (round-robin / least-connections / IP hash).
//! * **Health check** — protocol, port, interval, timeout, retries.
//! * **Certificate** — managed (issued via Hetzner Cert API) or uploaded.
//! * **Service config** — listen port, destination port, sticky sessions,
//!   proxy-protocol toggle, plus the algorithm/healthcheck/certificate trio.
//!
//! Mirrors the shape of the upstream
//! `hcloud/load_balancer.go::LoadBalancerService` struct.

use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LbAlgorithm {
    RoundRobin,
    LeastConnections,
    /// Source-IP hash. Hetzner refers to it as `least_connections` with
    /// stickiness disabled in some docs; we model it explicitly.
    IpHash,
}

impl LbAlgorithm {
    pub const fn name(self) -> &'static str {
        match self {
            LbAlgorithm::RoundRobin => "round_robin",
            LbAlgorithm::LeastConnections => "least_connections",
            LbAlgorithm::IpHash => "ip_hash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HealthCheckProtocol {
    Tcp,
    Http,
    Https,
}

impl HealthCheckProtocol {
    pub const fn name(self) -> &'static str {
        match self {
            HealthCheckProtocol::Tcp => "tcp",
            HealthCheckProtocol::Http => "http",
            HealthCheckProtocol::Https => "https",
        }
    }
    pub const fn requires_path(self) -> bool {
        matches!(self, HealthCheckProtocol::Http | HealthCheckProtocol::Https)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbHealthCheck {
    pub protocol: HealthCheckProtocol,
    pub port: u16,
    /// Probe interval in seconds (Hetzner enforces 3..=60).
    pub interval_seconds: u32,
    /// Probe timeout in seconds (Hetzner enforces 1..=interval).
    pub timeout_seconds: u32,
    /// Consecutive successful probes before flipping target healthy.
    pub retries: u32,
    /// HTTP path; `None` for TCP probes.
    pub path: Option<String>,
}

impl LbHealthCheck {
    pub fn tcp(port: u16) -> Self {
        Self {
            protocol: HealthCheckProtocol::Tcp,
            port,
            interval_seconds: 15,
            timeout_seconds: 10,
            retries: 3,
            path: None,
        }
    }

    pub fn http(port: u16, path: impl Into<String>) -> Self {
        Self {
            protocol: HealthCheckProtocol::Http,
            port,
            interval_seconds: 15,
            timeout_seconds: 10,
            retries: 3,
            path: Some(path.into()),
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if !(3..=60).contains(&self.interval_seconds) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "healthcheck interval {} outside [3, 60] seconds",
                    self.interval_seconds
                ),
            });
        }
        if self.timeout_seconds == 0 || self.timeout_seconds > self.interval_seconds {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "healthcheck timeout {} must be in [1, {}]",
                    self.timeout_seconds, self.interval_seconds
                ),
            });
        }
        if !(1..=10).contains(&self.retries) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("healthcheck retries {} outside [1, 10]", self.retries),
            });
        }
        if self.protocol.requires_path() && self.path.as_deref().unwrap_or("").is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "healthcheck protocol {} requires a non-empty path",
                    self.protocol.name()
                ),
            });
        }
        if !self.protocol.requires_path() && self.path.is_some() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "TCP healthcheck must not carry an HTTP path".into(),
            });
        }
        Ok(())
    }
}

/// LB certificate model. Hetzner ships two paths: a managed certificate
/// issued by their built-in ACME (`Managed`), and an upload-your-own variant
/// (`Uploaded`) referencing an existing certificate ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbCertificate {
    Managed { domains: Vec<String> },
    Uploaded { certificate_id: u64 },
}

impl LbCertificate {
    pub fn validate(&self) -> Result<(), CloudError> {
        match self {
            LbCertificate::Managed { domains } => {
                if domains.is_empty() {
                    return Err(CloudError::InvalidConfig {
                        provider: ProviderName::Hetzner,
                        reason: "managed certificate requires at least one domain".into(),
                    });
                }
                for d in domains {
                    if !d.contains('.') || d.starts_with('.') {
                        return Err(CloudError::InvalidConfig {
                            provider: ProviderName::Hetzner,
                            reason: format!("invalid managed-cert domain {d:?}"),
                        });
                    }
                }
                Ok(())
            }
            LbCertificate::Uploaded { certificate_id } => {
                if *certificate_id == 0 {
                    return Err(CloudError::InvalidConfig {
                        provider: ProviderName::Hetzner,
                        reason: "uploaded certificate id must be non-zero".into(),
                    });
                }
                Ok(())
            }
        }
    }

    pub const fn kind(&self) -> &'static str {
        match self {
            LbCertificate::Managed { .. } => "managed",
            LbCertificate::Uploaded { .. } => "uploaded",
        }
    }
}

/// Per-service LB configuration. Mirrors `hcloud/load_balancer.go::LoadBalancerService`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbServiceConfig {
    pub listen_port: u16,
    pub destination_port: u16,
    pub algorithm: LbAlgorithm,
    pub health_check: LbHealthCheck,
    pub certificate: Option<LbCertificate>,
    pub sticky_sessions: bool,
    pub proxy_protocol: bool,
}

impl LbServiceConfig {
    pub fn http(listen: u16, dest: u16) -> Self {
        Self {
            listen_port: listen,
            destination_port: dest,
            algorithm: LbAlgorithm::RoundRobin,
            health_check: LbHealthCheck::http(dest, "/healthz"),
            certificate: None,
            sticky_sessions: false,
            proxy_protocol: false,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if self.listen_port == 0 || self.destination_port == 0 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "listen_port and destination_port must be non-zero".into(),
            });
        }
        self.health_check.validate()?;
        if let Some(cert) = &self.certificate {
            cert.validate()?;
        }
        // HTTPS terminates need a certificate.
        if self.health_check.protocol == HealthCheckProtocol::Https && self.certificate.is_none() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "HTTPS healthcheck requires a certificate".into(),
            });
        }
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

    // ─── Algorithm ───────────────────────────────────────────────────────────

    #[test]
    fn lb_algorithm_names_match_hetzner_api_strings() {
        let _ = ctx(
            "acme",
            "hcloud/load_balancer.go",
            "LoadBalancerAlgorithmType",
        );
        assert_eq!(LbAlgorithm::RoundRobin.name(), "round_robin");
        assert_eq!(LbAlgorithm::LeastConnections.name(), "least_connections");
        assert_eq!(LbAlgorithm::IpHash.name(), "ip_hash");
    }

    // ─── Health check ────────────────────────────────────────────────────────

    #[test]
    fn tcp_healthcheck_constructor_is_valid() {
        let _ = ctx(
            "acme",
            "hcloud/load_balancer.go",
            "LoadBalancerServiceHealthCheck",
        );
        assert!(LbHealthCheck::tcp(8080).validate().is_ok());
    }

    #[test]
    fn http_healthcheck_constructor_is_valid() {
        let _ = ctx(
            "acme",
            "hcloud/load_balancer.go",
            "LoadBalancerServiceHealthCheck",
        );
        assert!(LbHealthCheck::http(8080, "/healthz").validate().is_ok());
    }

    #[test]
    fn healthcheck_interval_outside_3_to_60_is_rejected() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateHealthCheck");
        let mut h = LbHealthCheck::tcp(80);
        h.interval_seconds = 2;
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        h.interval_seconds = 120;
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn healthcheck_timeout_must_not_exceed_interval() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateHealthCheck");
        let mut h = LbHealthCheck::tcp(80);
        h.interval_seconds = 5;
        h.timeout_seconds = 10;
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn healthcheck_retries_must_be_in_range() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateHealthCheck");
        let mut h = LbHealthCheck::tcp(80);
        h.retries = 0;
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        h.retries = 99;
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn http_healthcheck_requires_a_path() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateHealthCheck");
        let mut h = LbHealthCheck::http(80, "/healthz");
        h.path = None;
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        h.path = Some("".into());
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn tcp_healthcheck_must_not_carry_a_path() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateHealthCheck");
        let mut h = LbHealthCheck::tcp(80);
        h.path = Some("/wrong".into());
        assert!(matches!(
            h.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn healthcheck_protocol_names_match_api_strings() {
        let _ = ctx(
            "acme",
            "hcloud/load_balancer.go",
            "LoadBalancerServiceHealthCheckProtocol",
        );
        assert_eq!(HealthCheckProtocol::Tcp.name(), "tcp");
        assert_eq!(HealthCheckProtocol::Http.name(), "http");
        assert_eq!(HealthCheckProtocol::Https.name(), "https");
        assert!(HealthCheckProtocol::Http.requires_path());
        assert!(!HealthCheckProtocol::Tcp.requires_path());
    }

    // ─── Certificate ─────────────────────────────────────────────────────────

    #[test]
    fn managed_certificate_requires_one_domain() {
        let _ = ctx("acme", "hcloud/certificate.go", "CertificateTypeManaged");
        let cert = LbCertificate::Managed { domains: vec![] };
        assert!(matches!(
            cert.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        let cert = LbCertificate::Managed {
            domains: vec!["example.com".into()],
        };
        assert!(cert.validate().is_ok());
    }

    #[test]
    fn managed_certificate_rejects_invalid_domains() {
        let _ = ctx("acme", "hcloud/certificate.go", "CertificateTypeManaged");
        let cert = LbCertificate::Managed {
            domains: vec!["nodot".into()],
        };
        assert!(matches!(
            cert.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        let cert = LbCertificate::Managed {
            domains: vec![".leading-dot.example".into()],
        };
        assert!(matches!(
            cert.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn uploaded_certificate_requires_nonzero_id() {
        let _ = ctx("acme", "hcloud/certificate.go", "CertificateTypeUploaded");
        let bad = LbCertificate::Uploaded { certificate_id: 0 };
        assert!(matches!(
            bad.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        let good = LbCertificate::Uploaded { certificate_id: 7 };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn certificate_kind_reports_managed_or_uploaded() {
        let _ = ctx("acme", "hcloud/certificate.go", "Certificate");
        assert_eq!(
            LbCertificate::Managed {
                domains: vec!["a.b".into()]
            }
            .kind(),
            "managed"
        );
        assert_eq!(
            LbCertificate::Uploaded { certificate_id: 1 }.kind(),
            "uploaded"
        );
    }

    // ─── Service config ──────────────────────────────────────────────────────

    #[test]
    fn lb_service_config_http_constructor_is_valid() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "LoadBalancerService");
        assert!(LbServiceConfig::http(80, 8080).validate().is_ok());
    }

    #[test]
    fn lb_service_config_zero_ports_are_rejected() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "LoadBalancerService");
        let mut c = LbServiceConfig::http(80, 8080);
        c.listen_port = 0;
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn lb_service_config_https_healthcheck_requires_certificate() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "LoadBalancerService");
        let mut c = LbServiceConfig::http(443, 8443);
        c.health_check.protocol = HealthCheckProtocol::Https;
        c.health_check.path = Some("/healthz".into());
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        c.certificate = Some(LbCertificate::Uploaded { certificate_id: 7 });
        assert!(c.validate().is_ok());
    }

    #[test]
    fn lb_service_config_propagates_health_check_validation() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "LoadBalancerService");
        let mut c = LbServiceConfig::http(80, 8080);
        c.health_check.interval_seconds = 1;
        assert!(matches!(
            c.validate().unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn lb_service_config_carries_proxy_protocol_and_sticky_flags() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "LoadBalancerService");
        let mut c = LbServiceConfig::http(80, 8080);
        c.proxy_protocol = true;
        c.sticky_sessions = true;
        assert!(c.validate().is_ok());
        assert!(c.proxy_protocol);
        assert!(c.sticky_sessions);
    }
}
