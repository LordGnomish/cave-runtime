// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cert-manager webhook admission validation engine (pure logic).
//!
//! Faithful line-port of cert-manager v1.17.2
//! `internal/apis/certmanager/validation/certificate.go`:
//!   - `ValidateCertificateSpec`
//!   - `ValidateDuration`
//!   - `validateIssuerRef`
//!   - `validateIPAddresses`
//!   - `validateEmailAddresses`
//!   - `validateUsages`
//!
//! Constants come from cert-manager:
//!   - `pkg/util/pki/generate.go`: `MinRSAKeySize = 2048`, `MaxRSAKeySize = 8192`
//!   - `pkg/apis/certmanager/v1/const.go`:
//!     `MinimumCertificateDuration = 1h`, `MinimumRenewBefore = 5m`,
//!     `DefaultCertificateDuration = 90d`
//!   - `pkg/api/util/usages.go`: the keyUsages + extKeyUsages tables.
//!
//! cert-manager's webhook is a *separate Go binary* serving K8s AdmissionReview
//! over HTTPS. That transport (TLS server, AdmissionReview decode, K8s
//! integration) is genuinely cross-crate (cave-admission). The pure spec
//! validation algorithm ported here, however, is in-crate runtime logic and is
//! exactly what the webhook executes per request.

/// Cite: cert-manager `pkg/util/pki/generate.go`.
pub const MIN_RSA_KEY_SIZE: i32 = 2048;
pub const MAX_RSA_KEY_SIZE: i32 = 8192;

/// Cite: cert-manager `pkg/apis/certmanager/v1/const.go`.
pub const MINIMUM_CERTIFICATE_DURATION_SECS: i64 = 3600; // 1h
pub const MINIMUM_RENEW_BEFORE_SECS: i64 = 5 * 60; // 5m
pub const DEFAULT_CERTIFICATE_DURATION_SECS: i64 = 90 * 24 * 3600; // 90d

/// A single admission validation error, mirroring k8s
/// `field.Error{Field, BadValue, Detail}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The dotted field path, e.g. `spec.privateKey.size`.
    pub field: String,
    /// Human-readable detail.
    pub message: String,
}

impl ValidationError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Cite: cert-manager `Certificate.spec.privateKey.algorithm` (subset that the
/// webhook validates: rsa / ecdsa / ed25519).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookKeyAlgorithm {
    /// Empty algorithm string defaults to RSA in cert-manager — represented here
    /// explicitly as `Rsa`.
    Rsa,
    Ecdsa,
    Ed25519,
}

/// Cite: cert-manager `Certificate.spec.privateKey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookPrivateKey {
    pub algorithm: WebhookKeyAlgorithm,
    /// 0 means "unset" (Go zero value); validation only runs when > 0.
    pub size: i32,
}

/// Cite: cert-manager `cmmeta.ObjectReference` used by `IssuerRef`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookIssuerRef {
    pub name: String,
    pub kind: String,
    pub group: String,
}

/// The slice of `CertificateSpec` that the webhook admission validator reads.
/// Cite: cert-manager `internal/apis/certmanager.CertificateSpec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookCertificateSpec {
    pub secret_name: String,
    pub issuer_ref: WebhookIssuerRef,
    pub common_name: Option<String>,
    pub dns_names: Vec<String>,
    pub ip_addresses: Vec<String>,
    pub email_addresses: Vec<String>,
    pub uris: Vec<String>,
    pub usages: Vec<String>,
    pub private_key: Option<WebhookPrivateKey>,
    /// `None` => default 90d (Go `*metav1.Duration` nil).
    pub duration_seconds: Option<i64>,
    pub renew_before_seconds: Option<i64>,
    pub is_ca: bool,
    pub revision_history_limit: Option<i32>,
}

const CERT_MANAGER_GROUP: &str = "cert-manager.io";

/// Cite: cert-manager `ValidateCertificateSpec`
/// (`internal/apis/certmanager/validation/certificate.go:45`).
pub fn validate_certificate_spec(crt: &WebhookCertificateSpec, fld: &str) -> Vec<ValidationError> {
    let mut el: Vec<ValidationError> = Vec::new();

    // SecretName required + must be a DNS-1123 subdomain.
    // Cite: certificate.go:47-53.
    if crt.secret_name.is_empty() {
        el.push(ValidationError::new(
            format!("{fld}.secretName"),
            "must be specified",
        ));
    } else {
        for msg in name_is_dns_subdomain(&crt.secret_name) {
            el.push(ValidationError::new(format!("{fld}.secretName"), msg));
        }
    }

    // Cite: certificate.go:55.
    el.extend(validate_issuer_ref(&crt.issuer_ref, fld));

    let common_name = crt.common_name.clone().unwrap_or_default();

    // At least one SAN/identity must be set.
    // Cite: certificate.go:106-113.
    if common_name.is_empty()
        && crt.dns_names.is_empty()
        && crt.uris.is_empty()
        && crt.email_addresses.is_empty()
        && crt.ip_addresses.is_empty()
    {
        el.push(ValidationError::new(
            fld,
            "at least one of commonName (from the commonName field or from a \
             literalSubject), dnsNames, uriSANs, ipAddresses, emailSANs or \
             otherNames must be set",
        ));
    }

    // commonName <= 64 chars. Cite: certificate.go:116-118.
    if common_name.chars().count() > 64 {
        el.push(ValidationError::new(
            format!("{fld}.commonName"),
            "must be no more than 64 characters",
        ));
    }

    // Cite: certificate.go:120-122.
    if !crt.ip_addresses.is_empty() {
        el.extend(validate_ip_addresses(crt, fld));
    }

    // Cite: certificate.go:124-126.
    if !crt.email_addresses.is_empty() {
        el.extend(validate_email_addresses(crt, fld));
    }

    // privateKey size/algorithm. Cite: certificate.go:148-163.
    if let Some(pk) = &crt.private_key {
        match pk.algorithm {
            WebhookKeyAlgorithm::Rsa => {
                if pk.size > 0 && (pk.size < MIN_RSA_KEY_SIZE || pk.size > MAX_RSA_KEY_SIZE) {
                    el.push(ValidationError::new(
                        format!("{fld}.privateKey.size"),
                        format!(
                            "must be between {MIN_RSA_KEY_SIZE} and {MAX_RSA_KEY_SIZE} for rsa keyAlgorithm"
                        ),
                    ));
                }
            }
            WebhookKeyAlgorithm::Ecdsa => {
                if pk.size > 0 && pk.size != 256 && pk.size != 384 && pk.size != 521 {
                    el.push(ValidationError::new(
                        format!("{fld}.privateKey.size"),
                        "supported values: \"256\", \"384\", \"521\"",
                    ));
                }
            }
            WebhookKeyAlgorithm::Ed25519 => {}
        }
    }

    // Duration / renewBefore. Cite: certificate.go:165-167.
    if crt.duration_seconds.is_some() || crt.renew_before_seconds.is_some() {
        el.extend(validate_duration(crt, fld));
    }

    // Usages. Cite: certificate.go:168-170.
    if !crt.usages.is_empty() {
        el.extend(validate_usages(crt, fld));
    }

    // revisionHistoryLimit >= 1. Cite: certificate.go:171-173.
    if let Some(rhl) = crt.revision_history_limit {
        if rhl < 1 {
            el.push(ValidationError::new(
                format!("{fld}.revisionHistoryLimit"),
                "must not be less than 1",
            ));
        }
    }

    el
}

/// Cite: cert-manager `validateIssuerRef` (certificate.go:219-259).
fn validate_issuer_ref(issuer_ref: &WebhookIssuerRef, fld: &str) -> Vec<ValidationError> {
    let mut el = Vec::new();
    let issuer_ref_path = format!("{fld}.issuerRef");

    if issuer_ref.name.is_empty() {
        el.push(ValidationError::new(
            format!("{issuer_ref_path}.name"),
            "must be specified",
        ));
    }

    // If group is blank or the built-in cert-manager.io group, validate Kind.
    // Cite: certificate.go:228-256.
    if issuer_ref.group.is_empty() || issuer_ref.group == CERT_MANAGER_GROUP {
        match issuer_ref.kind.as_str() {
            "" | "Issuer" | "ClusterIssuer" => {}
            _ => {
                let kind_path = format!("{issuer_ref_path}.kind");
                let mut err_msg = String::from("must be one of Issuer or ClusterIssuer");
                if issuer_ref.group.is_empty() {
                    // Hint: external kind set but group forgotten.
                    // Cite: certificate.go:244-251.
                    err_msg.push_str(&format!(
                        " (did you forget to set {issuer_ref_path}.group?)"
                    ));
                }
                el.push(ValidationError::new(kind_path, err_msg));
            }
        }
    }

    el
}

/// Cite: cert-manager `validateIPAddresses` (certificate.go:261-273).
fn validate_ip_addresses(a: &WebhookCertificateSpec, fld: &str) -> Vec<ValidationError> {
    let mut el = Vec::new();
    for (i, d) in a.ip_addresses.iter().enumerate() {
        if parse_ip(d).is_none() {
            el.push(ValidationError::new(
                format!("{fld}.ipAddresses[{i}]"),
                "invalid IP address",
            ));
        }
    }
    el
}

/// Cite: cert-manager `validateEmailAddresses` (certificate.go:275-291).
/// Go uses `net/mail.ParseAddress`; an RFC-5322 name-form (`Name <addr>`) parses
/// but `e.Address != d`, which cert-manager rejects.
fn validate_email_addresses(a: &WebhookCertificateSpec, fld: &str) -> Vec<ValidationError> {
    let mut el = Vec::new();
    for (i, d) in a.email_addresses.iter().enumerate() {
        match parse_mail_address(d) {
            None => el.push(ValidationError::new(
                format!("{fld}.emailAddresses[{i}]"),
                format!("invalid email address: {d}"),
            )),
            Some(addr) if &addr != d => el.push(ValidationError::new(
                format!("{fld}.emailAddresses[{i}]"),
                "invalid email address: make sure the supplied value only \
                 contains the email address itself",
            )),
            Some(_) => {}
        }
    }
    el
}

/// Cite: cert-manager `validateUsages` (certificate.go:293-303).
fn validate_usages(a: &WebhookCertificateSpec, fld: &str) -> Vec<ValidationError> {
    let mut el = Vec::new();
    for (i, u) in a.usages.iter().enumerate() {
        if !is_known_usage(u) {
            el.push(ValidationError::new(
                format!("{fld}.usages[{i}]"),
                "unknown keyusage",
            ));
        }
    }
    el
}

/// Cite: cert-manager `ValidateDuration` (certificate.go:323-359).
pub fn validate_duration(crt: &WebhookCertificateSpec, fld: &str) -> Vec<ValidationError> {
    let mut el = Vec::new();

    // util.DefaultCertDuration — nil => 90d.
    let duration = crt
        .duration_seconds
        .unwrap_or(DEFAULT_CERTIFICATE_DURATION_SECS);

    if duration < MINIMUM_CERTIFICATE_DURATION_SECS {
        el.push(ValidationError::new(
            format!("{fld}.duration"),
            format!(
                "certificate duration must be greater than {MINIMUM_CERTIFICATE_DURATION_SECS}s"
            ),
        ));
    }

    if let Some(rb) = crt.renew_before_seconds {
        // renewBefore must be >= minimum. Cite: certificate.go:338-340.
        if rb < MINIMUM_RENEW_BEFORE_SECS {
            el.push(ValidationError::new(
                format!("{fld}.renewBefore"),
                format!(
                    "certificate renewBefore must be greater than {MINIMUM_RENEW_BEFORE_SECS}s"
                ),
            ));
        }
        // renewBefore must be < duration. Cite: certificate.go:342-344.
        if rb >= duration {
            el.push(ValidationError::new(
                format!("{fld}.renewBefore"),
                format!(
                    "certificate duration {duration}s must be greater than renewBefore {rb}s"
                ),
            ));
        }
    }

    el
}

/// Known key/ext-key usages. Cite: cert-manager `pkg/api/util/usages.go`
/// `keyUsages` + `extKeyUsages` map keys.
fn is_known_usage(u: &str) -> bool {
    matches!(
        u,
        // keyUsages
        "signing"
            | "digital signature"
            | "content commitment"
            | "key encipherment"
            | "key agreement"
            | "data encipherment"
            | "cert sign"
            | "crl sign"
            | "encipher only"
            | "decipher only"
            // extKeyUsages
            | "any"
            | "server auth"
            | "client auth"
            | "code signing"
            | "email protection"
            | "s/mime"
            | "ipsec end system"
            | "ipsec tunnel"
            | "ipsec user"
            | "timestamping"
            | "ocsp signing"
            | "microsoft sgc"
            | "netscape sgc"
    )
}

/// Faithful port of `k8s.io/apimachinery/pkg/api/validation.NameIsDNSSubdomain`
/// (RFC-1123 subdomain). Returns a list of human-readable error messages (empty
/// = valid). Cite: certificate.go:50.
fn name_is_dns_subdomain(name: &str) -> Vec<String> {
    let mut errs = Vec::new();
    const MAX_LEN: usize = 253;
    if name.len() > MAX_LEN {
        errs.push(format!("must be no more than {MAX_LEN} characters"));
    }
    if !is_dns1123_subdomain(name) {
        errs.push(
            "a lowercase RFC 1123 subdomain must consist of lower case \
             alphanumeric characters, '-' or '.', and must start and end with \
             an alphanumeric character"
                .to_string(),
        );
    }
    errs
}

/// RFC-1123 subdomain: one or more dot-separated DNS-1123 labels.
fn is_dns1123_subdomain(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.split('.').all(is_dns1123_label)
}

/// RFC-1123 label: `[a-z0-9]([-a-z0-9]*[a-z0-9])?`, max 63 chars.
fn is_dns1123_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }
    let bytes = label.as_bytes();
    let valid_char = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-';
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !alnum(bytes[0]) || !alnum(bytes[bytes.len() - 1]) {
        return false;
    }
    bytes.iter().all(|&c| valid_char(c))
}

/// Port of Go `net.ParseIP`: accepts dotted-decimal IPv4 and IPv6 forms.
fn parse_ip(s: &str) -> Option<std::net::IpAddr> {
    s.parse::<std::net::IpAddr>().ok()
}

/// Port of Go `net/mail.ParseAddress` reduced to the cert-manager use: returns
/// the bare address if the input is a single addr-spec, or the inner address if
/// the input is a `Name <addr>` form. `None` if unparseable.
fn parse_mail_address(s: &str) -> Option<String> {
    let trimmed = s.trim();
    // Name <addr> form.
    if let (Some(lt), Some(gt)) = (trimmed.find('<'), trimmed.rfind('>')) {
        if gt > lt {
            let inner = trimmed[lt + 1..gt].trim();
            if is_addr_spec(inner) {
                return Some(inner.to_string());
            }
            return None;
        }
    }
    if is_addr_spec(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// Minimal RFC-5321 addr-spec check: exactly one `@`, non-empty local and domain
/// parts, no embedded whitespace, and a domain containing at least one dot or a
/// bracketed literal.
fn is_addr_spec(s: &str) -> bool {
    if s.is_empty() || s.contains(char::is_whitespace) {
        return false;
    }
    let mut parts = s.splitn(2, '@');
    let local = parts.next().unwrap_or("");
    let domain = match parts.next() {
        Some(d) => d,
        None => return false,
    };
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    // Reject a second '@'.
    if domain.contains('@') {
        return false;
    }
    // Domain must look like a hostname or bracketed literal.
    if domain.starts_with('[') && domain.ends_with(']') {
        return true;
    }
    domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}
