// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/model/Alert.java
//
//! Alert + Risk + CWE → OWASP Top 10 taxonomy.

use crate::models::RiskLevel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub name: String,
    pub risk: RiskLevel,
    pub cwe_id: u32,
    pub url: String,
    pub description: String,
    pub solution: String,
    pub evidence: Option<String>,
    pub plugin_id: u32,
}

/// OWASP Top 10 2021 categories. We retain the upstream short codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwaspTop10 {
    A01BrokenAccessControl,
    A02CryptographicFailures,
    A03Injection,
    A04InsecureDesign,
    A05SecurityMisconfiguration,
    A06VulnerableComponents,
    A07IdentificationAuthnFailures,
    A08SoftwareDataIntegrityFailures,
    A09SecurityLoggingMonitoringFailures,
    A10ServerSideRequestForgery,
}

impl OwaspTop10 {
    pub fn code(self) -> &'static str {
        match self {
            Self::A01BrokenAccessControl => "A01:2021",
            Self::A02CryptographicFailures => "A02:2021",
            Self::A03Injection => "A03:2021",
            Self::A04InsecureDesign => "A04:2021",
            Self::A05SecurityMisconfiguration => "A05:2021",
            Self::A06VulnerableComponents => "A06:2021",
            Self::A07IdentificationAuthnFailures => "A07:2021",
            Self::A08SoftwareDataIntegrityFailures => "A08:2021",
            Self::A09SecurityLoggingMonitoringFailures => "A09:2021",
            Self::A10ServerSideRequestForgery => "A10:2021",
        }
    }
}

/// Map a CWE id to its primary OWASP Top 10 2021 bucket. Derived from
/// the OWASP Top 10 2021 mapping document and ZAP's
/// `Alert.getCweOwaspId` lookup table. Returns the most-specific
/// category — multi-mapped CWEs (e.g. 89 lands in both A03 Injection
/// and A04 Insecure Design) follow ZAP's primary choice.
pub fn cwe_to_owasp(cwe: u32) -> Option<OwaspTop10> {
    use OwaspTop10::*;
    Some(match cwe {
        // A01 Broken Access Control.
        22 | 23 | 35 | 200 | 201 | 219 | 264 | 275 | 276 | 284 | 285 | 352 | 359 | 377 | 402
        | 425 | 441 | 497 | 538 | 540 | 548 | 552 | 566 | 601 | 639 | 651 | 668 | 706 | 862
        | 863 | 913 | 922 | 1275 => A01BrokenAccessControl,
        // A02 Cryptographic Failures.
        261 | 296 | 310 | 311 | 312 | 319 | 321 | 322 | 323 | 324 | 325 | 326 | 327 | 328 | 329
        | 330 | 331 | 335 | 336 | 337 | 338 | 340 | 347 | 523 | 720 | 757 | 759 | 760 | 818
        | 916 => A02CryptographicFailures,
        // A03 Injection.
        20 | 74 | 75 | 77 | 78 | 79 | 80 | 83 | 87 | 88 | 89 | 90 | 91 | 93 | 94 | 95 | 96 | 97
        | 98 | 99 | 100 | 113 | 116 | 138 | 184 | 470 | 471 | 564 | 610 | 643 | 644 | 652 | 917 => {
            A03Injection
        }
        // A04 Insecure Design. CWE 311/312/313/316 already mapped to A02
        // (the OWASP 2021 doc lists them in both buckets — primary stays
        // A02 per ZAP's lookup).
        209 | 256 | 257 | 266 | 269 | 280 | 419 | 430 | 434 | 444 | 451 | 472 | 501 | 522 | 555
        | 656 | 657 | 799 | 807 | 840 | 841 | 927 | 1021 | 1173 => A04InsecureDesign,
        // A05 Security Misconfiguration.
        2 | 11 | 13 | 15 | 16 | 260 | 315 | 520 | 526 | 537 | 541 | 547 | 611 | 614 | 756 | 776
        | 942 | 1004 | 1032 | 1174 => A05SecurityMisconfiguration,
        // A06 Vulnerable and Outdated Components.
        937 | 1035 | 1104 => A06VulnerableComponents,
        // A07 Identification and Authentication Failures.
        255 | 259 | 287 | 288 | 290 | 294 | 295 | 297 | 300 | 302 | 304 | 306 | 307 | 346 | 384
        | 521 | 613 | 620 | 640 | 798 => A07IdentificationAuthnFailures,
        // A08 Software and Data Integrity Failures.
        345 | 353 | 426 | 494 | 502 | 565 | 784 | 829 | 830 | 915 => {
            A08SoftwareDataIntegrityFailures
        }
        // A09 Security Logging and Monitoring Failures.
        117 | 223 | 532 | 778 => A09SecurityLoggingMonitoringFailures,
        // A10 SSRF.
        918 => A10ServerSideRequestForgery,
        // 693 (Protection Mechanism Failure) covers missing security headers.
        693 => A05SecurityMisconfiguration,
        _ => return None,
    })
}

/// Reverse lookup — get a representative CWE list for a category.
pub fn owasp_cwe_examples(cat: OwaspTop10) -> Vec<u32> {
    use OwaspTop10::*;
    match cat {
        A01BrokenAccessControl => vec![22, 200, 352, 862, 863],
        A02CryptographicFailures => vec![310, 311, 327, 916],
        A03Injection => vec![77, 78, 79, 89, 94],
        A04InsecureDesign => vec![209, 256, 434, 656],
        A05SecurityMisconfiguration => vec![16, 611, 614, 693, 942],
        A06VulnerableComponents => vec![937, 1035, 1104],
        A07IdentificationAuthnFailures => vec![287, 384, 521, 613, 798],
        A08SoftwareDataIntegrityFailures => vec![345, 426, 494, 502],
        A09SecurityLoggingMonitoringFailures => vec![117, 223, 532, 778],
        A10ServerSideRequestForgery => vec![918],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwe_to_owasp_known() {
        assert_eq!(cwe_to_owasp(79), Some(OwaspTop10::A03Injection));
        assert_eq!(cwe_to_owasp(89), Some(OwaspTop10::A03Injection));
        assert_eq!(cwe_to_owasp(78), Some(OwaspTop10::A03Injection));
        assert_eq!(cwe_to_owasp(22), Some(OwaspTop10::A01BrokenAccessControl));
        assert_eq!(cwe_to_owasp(352), Some(OwaspTop10::A01BrokenAccessControl));
        assert_eq!(
            cwe_to_owasp(918),
            Some(OwaspTop10::A10ServerSideRequestForgery)
        );
        assert_eq!(
            cwe_to_owasp(611),
            Some(OwaspTop10::A05SecurityMisconfiguration)
        );
        assert_eq!(
            cwe_to_owasp(614),
            Some(OwaspTop10::A05SecurityMisconfiguration)
        );
        assert_eq!(
            cwe_to_owasp(287),
            Some(OwaspTop10::A07IdentificationAuthnFailures)
        );
        assert_eq!(
            cwe_to_owasp(327),
            Some(OwaspTop10::A02CryptographicFailures)
        );
    }

    #[test]
    fn cwe_to_owasp_unknown_returns_none() {
        assert_eq!(cwe_to_owasp(999_999), None);
    }

    #[test]
    fn owasp_codes_are_2021_style() {
        for c in [
            OwaspTop10::A01BrokenAccessControl,
            OwaspTop10::A03Injection,
            OwaspTop10::A10ServerSideRequestForgery,
        ] {
            assert!(c.code().starts_with('A'));
            assert!(c.code().ends_with("2021"));
        }
    }

    #[test]
    fn owasp_cwe_examples_non_empty() {
        for c in [
            OwaspTop10::A01BrokenAccessControl,
            OwaspTop10::A02CryptographicFailures,
            OwaspTop10::A03Injection,
            OwaspTop10::A04InsecureDesign,
            OwaspTop10::A05SecurityMisconfiguration,
            OwaspTop10::A06VulnerableComponents,
            OwaspTop10::A07IdentificationAuthnFailures,
            OwaspTop10::A08SoftwareDataIntegrityFailures,
            OwaspTop10::A09SecurityLoggingMonitoringFailures,
            OwaspTop10::A10ServerSideRequestForgery,
        ] {
            assert!(!owasp_cwe_examples(c).is_empty());
        }
    }

    #[test]
    fn alert_roundtrip_json() {
        let a = Alert {
            name: "SQL Injection".to_string(),
            risk: RiskLevel::High,
            cwe_id: 89,
            url: "http://x/".to_string(),
            description: "d".to_string(),
            solution: "s".to_string(),
            evidence: Some("e".to_string()),
            plugin_id: 40018,
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: Alert = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}
