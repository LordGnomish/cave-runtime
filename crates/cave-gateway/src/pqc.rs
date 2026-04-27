//! Post-quantum cryptography configuration for the TLS data plane.
//!
//! Models the IETF ML-KEM-768 hybrid handshake groups (RFC 9794-style
//! `X25519MLKEM768`, `SecP256r1MLKEM768`, `SecP384r1MLKEM768`) plus the legacy
//! ECDH groups they accompany. The actual KEM is wired through aws-lc-rs /
//! a future rustls feature flag; this module ships the *configuration* parity
//! surface so listeners and ServerHello negotiation can be exercised today.
//!
//! Out of scope: actual hybrid handshake bytes (those land when
//! aws-lc-rs/PQC is feature-gated in tls.rs).

use serde::{Deserialize, Serialize};

/// IETF NamedGroup codepoints for the TLS 1.3 supported_groups extension.
///
/// Hybrid PQC codepoints are taken from
/// `draft-ietf-tls-hybrid-design-12` and the IANA registry; classical
/// values are RFC 8446 Section 4.2.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NamedGroup {
    // Classical (RFC 8446)
    Secp256r1,
    Secp384r1,
    Secp521r1,
    X25519,
    X448,
    // Pure PQC
    MlKem768,
    MlKem1024,
    // Hybrid (PQC + classical)
    X25519MlKem768,
    Secp256r1MlKem768,
    Secp384r1MlKem768,
    Secp384r1MlKem1024,
}

impl NamedGroup {
    /// IANA TLS NamedGroup codepoint.
    pub fn ietf_codepoint(self) -> u16 {
        match self {
            NamedGroup::Secp256r1 => 0x0017,
            NamedGroup::Secp384r1 => 0x0018,
            NamedGroup::Secp521r1 => 0x0019,
            NamedGroup::X25519 => 0x001D,
            NamedGroup::X448 => 0x001E,
            // ML-KEM (draft codepoints — IANA private use range until
            // ratification; values come from rustls + BoringSSL agreement).
            NamedGroup::MlKem768 => 0x0768,
            NamedGroup::MlKem1024 => 0x1024,
            NamedGroup::X25519MlKem768 => 0x11EC,
            NamedGroup::Secp256r1MlKem768 => 0x11EB,
            NamedGroup::Secp384r1MlKem768 => 0x11ED,
            NamedGroup::Secp384r1MlKem1024 => 0x11EF,
        }
    }

    /// True if the group's secret derivation includes a post-quantum KEM.
    pub fn is_post_quantum(self) -> bool {
        matches!(
            self,
            NamedGroup::MlKem768
                | NamedGroup::MlKem1024
                | NamedGroup::X25519MlKem768
                | NamedGroup::Secp256r1MlKem768
                | NamedGroup::Secp384r1MlKem768
                | NamedGroup::Secp384r1MlKem1024
        )
    }

    /// True if the group combines a classical ECDH with a post-quantum KEM.
    pub fn is_hybrid(self) -> bool {
        matches!(
            self,
            NamedGroup::X25519MlKem768
                | NamedGroup::Secp256r1MlKem768
                | NamedGroup::Secp384r1MlKem768
                | NamedGroup::Secp384r1MlKem1024
        )
    }

    pub fn is_classical(self) -> bool {
        !self.is_post_quantum()
    }

    /// FIPS 140-3 approved? ML-KEM-768/1024 are approved under FIPS 203;
    /// pure ECDH groups ≥ secp256r1 are approved under FIPS 186-4.
    pub fn is_fips_approved(self) -> bool {
        matches!(
            self,
            NamedGroup::Secp256r1
                | NamedGroup::Secp384r1
                | NamedGroup::Secp521r1
                | NamedGroup::MlKem768
                | NamedGroup::MlKem1024
                | NamedGroup::Secp256r1MlKem768
                | NamedGroup::Secp384r1MlKem768
                | NamedGroup::Secp384r1MlKem1024
        )
    }

    /// Approximate post-quantum security strength in bits, for ranking
    /// preference lists.
    pub fn pq_strength_bits(self) -> u32 {
        match self {
            NamedGroup::Secp256r1 | NamedGroup::X25519 => 0,
            NamedGroup::Secp384r1 => 0,
            NamedGroup::Secp521r1 | NamedGroup::X448 => 0,
            NamedGroup::MlKem768
            | NamedGroup::X25519MlKem768
            | NamedGroup::Secp256r1MlKem768
            | NamedGroup::Secp384r1MlKem768 => 192,
            NamedGroup::MlKem1024 | NamedGroup::Secp384r1MlKem1024 => 256,
        }
    }
}

// ── PqcConfig — listener-wide preference ────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PqcConfig {
    /// Preference-ordered group list (most-preferred first).
    pub groups: Vec<NamedGroup>,
    /// Disallow connections whose final group is purely classical.
    pub require_post_quantum: bool,
    /// Disallow non-FIPS groups (only FIPS-approved are accepted).
    pub fips_only: bool,
}

impl Default for PqcConfig {
    fn default() -> Self {
        // Sovereign default: hybrid first, classical fallback for legacy
        // peers, X448 absent (rare in client stacks).
        Self {
            groups: vec![
                NamedGroup::X25519MlKem768,
                NamedGroup::Secp256r1MlKem768,
                NamedGroup::X25519,
                NamedGroup::Secp256r1,
            ],
            require_post_quantum: false,
            fips_only: false,
        }
    }
}

impl PqcConfig {
    pub fn fips_strict() -> Self {
        Self {
            groups: vec![
                NamedGroup::Secp384r1MlKem1024,
                NamedGroup::Secp384r1MlKem768,
                NamedGroup::Secp256r1MlKem768,
                NamedGroup::Secp384r1,
                NamedGroup::Secp256r1,
            ],
            require_post_quantum: true,
            fips_only: true,
        }
    }

    pub fn legacy_classical_only() -> Self {
        Self {
            groups: vec![
                NamedGroup::X25519,
                NamedGroup::Secp256r1,
                NamedGroup::Secp384r1,
            ],
            require_post_quantum: false,
            fips_only: false,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.groups.is_empty() {
            return Err("at least one group required".into());
        }
        let mut seen = std::collections::HashSet::new();
        for g in &self.groups {
            if !seen.insert(*g) {
                return Err(format!("duplicate group: {:?}", g));
            }
            if self.fips_only && !g.is_fips_approved() {
                return Err(format!("non-FIPS group with fips_only: {:?}", g));
            }
        }
        if self.require_post_quantum && !self.groups.iter().any(|g| g.is_post_quantum()) {
            return Err("require_post_quantum but no PQ groups configured".into());
        }
        Ok(())
    }

    /// Pick the highest-preference group both sides support.
    pub fn negotiate(&self, peer_supported: &[NamedGroup]) -> Option<NamedGroup> {
        for g in &self.groups {
            if peer_supported.contains(g) {
                if self.require_post_quantum && !g.is_post_quantum() {
                    continue;
                }
                if self.fips_only && !g.is_fips_approved() {
                    continue;
                }
                return Some(*g);
            }
        }
        None
    }

    /// Wire the supported_groups extension as IETF codepoints, big-endian.
    pub fn extension_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.groups.len() * 2);
        for g in &self.groups {
            let cp = g.ietf_codepoint();
            out.push((cp >> 8) as u8);
            out.push((cp & 0xff) as u8);
        }
        out
    }

    /// Returns the strongest PQ strength offered, or 0 if none.
    pub fn min_pq_strength(&self) -> u32 {
        self.groups
            .iter()
            .filter(|g| g.is_post_quantum())
            .map(|g| g.pq_strength_bits())
            .min()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Codepoints ──

    #[test]
    fn codepoints_classical() {
        assert_eq!(NamedGroup::Secp256r1.ietf_codepoint(), 0x0017);
        assert_eq!(NamedGroup::X25519.ietf_codepoint(), 0x001D);
    }

    #[test]
    fn codepoints_pqc_unique() {
        let groups = [
            NamedGroup::Secp256r1,
            NamedGroup::Secp384r1,
            NamedGroup::Secp521r1,
            NamedGroup::X25519,
            NamedGroup::X448,
            NamedGroup::MlKem768,
            NamedGroup::MlKem1024,
            NamedGroup::X25519MlKem768,
            NamedGroup::Secp256r1MlKem768,
            NamedGroup::Secp384r1MlKem768,
            NamedGroup::Secp384r1MlKem1024,
        ];
        let mut seen = std::collections::HashSet::new();
        for g in groups {
            assert!(seen.insert(g.ietf_codepoint()), "duplicate codepoint: {:?}", g);
        }
    }

    // ── Group traits ──

    #[test]
    fn is_post_quantum_classical_false() {
        assert!(!NamedGroup::X25519.is_post_quantum());
        assert!(!NamedGroup::Secp256r1.is_post_quantum());
    }

    #[test]
    fn is_post_quantum_pqc_true() {
        assert!(NamedGroup::MlKem768.is_post_quantum());
        assert!(NamedGroup::X25519MlKem768.is_post_quantum());
    }

    #[test]
    fn is_hybrid_only_for_combined() {
        assert!(NamedGroup::X25519MlKem768.is_hybrid());
        assert!(!NamedGroup::MlKem768.is_hybrid());
        assert!(!NamedGroup::X25519.is_hybrid());
    }

    #[test]
    fn is_classical_inverse_of_pq() {
        assert!(NamedGroup::X25519.is_classical());
        assert!(!NamedGroup::MlKem768.is_classical());
    }

    #[test]
    fn is_fips_approved_x25519_no() {
        assert!(!NamedGroup::X25519.is_fips_approved());
    }

    #[test]
    fn is_fips_approved_secp_yes() {
        assert!(NamedGroup::Secp256r1.is_fips_approved());
        assert!(NamedGroup::Secp384r1.is_fips_approved());
        assert!(NamedGroup::Secp521r1.is_fips_approved());
    }

    #[test]
    fn is_fips_approved_mlkem_yes() {
        assert!(NamedGroup::MlKem768.is_fips_approved());
        assert!(NamedGroup::MlKem1024.is_fips_approved());
    }

    #[test]
    fn is_fips_approved_x25519_hybrid_no() {
        // X25519 is non-FIPS, so the X25519+ML-KEM hybrid is non-FIPS.
        assert!(!NamedGroup::X25519MlKem768.is_fips_approved());
    }

    #[test]
    fn pq_strength_classical_zero() {
        assert_eq!(NamedGroup::X25519.pq_strength_bits(), 0);
    }

    #[test]
    fn pq_strength_mlkem768_192() {
        assert_eq!(NamedGroup::MlKem768.pq_strength_bits(), 192);
        assert_eq!(NamedGroup::X25519MlKem768.pq_strength_bits(), 192);
    }

    #[test]
    fn pq_strength_mlkem1024_256() {
        assert_eq!(NamedGroup::MlKem1024.pq_strength_bits(), 256);
        assert_eq!(NamedGroup::Secp384r1MlKem1024.pq_strength_bits(), 256);
    }

    // ── PqcConfig defaults ──

    #[test]
    fn default_starts_with_hybrid() {
        let c = PqcConfig::default();
        assert_eq!(c.groups[0], NamedGroup::X25519MlKem768);
    }

    #[test]
    fn default_validates() {
        PqcConfig::default().validate().unwrap();
    }

    #[test]
    fn fips_strict_validates() {
        PqcConfig::fips_strict().validate().unwrap();
    }

    #[test]
    fn legacy_classical_only_validates() {
        PqcConfig::legacy_classical_only().validate().unwrap();
    }

    // ── Validate failures ──

    #[test]
    fn validate_empty_rejects() {
        let c = PqcConfig {
            groups: vec![],
            require_post_quantum: false,
            fips_only: false,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_duplicate_rejects() {
        let c = PqcConfig {
            groups: vec![NamedGroup::X25519, NamedGroup::X25519],
            require_post_quantum: false,
            fips_only: false,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_fips_only_with_x25519_rejects() {
        let c = PqcConfig {
            groups: vec![NamedGroup::X25519],
            require_post_quantum: false,
            fips_only: true,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_require_pq_with_classical_only_rejects() {
        let c = PqcConfig {
            groups: vec![NamedGroup::X25519, NamedGroup::Secp256r1],
            require_post_quantum: true,
            fips_only: false,
        };
        assert!(c.validate().is_err());
    }

    // ── negotiate ──

    #[test]
    fn negotiate_picks_first_mutual() {
        let c = PqcConfig::default();
        let peer = vec![NamedGroup::X25519, NamedGroup::Secp256r1];
        assert_eq!(c.negotiate(&peer), Some(NamedGroup::X25519));
    }

    #[test]
    fn negotiate_prefers_hybrid_when_offered() {
        let c = PqcConfig::default();
        let peer = vec![NamedGroup::X25519MlKem768, NamedGroup::X25519];
        assert_eq!(c.negotiate(&peer), Some(NamedGroup::X25519MlKem768));
    }

    #[test]
    fn negotiate_skips_classical_when_require_pq() {
        let mut c = PqcConfig::default();
        c.require_post_quantum = true;
        let peer = vec![NamedGroup::X25519, NamedGroup::X25519MlKem768];
        // X25519 first in our preference, but PQ required, so we skip to
        // X25519MlKem768.
        assert_eq!(c.negotiate(&peer), Some(NamedGroup::X25519MlKem768));
    }

    #[test]
    fn negotiate_returns_none_when_no_pq_overlap() {
        let mut c = PqcConfig::default();
        c.require_post_quantum = true;
        let peer = vec![NamedGroup::X25519];
        assert!(c.negotiate(&peer).is_none());
    }

    #[test]
    fn negotiate_skips_non_fips_when_fips_only() {
        // FIPS-only without PQ requirement so we can see the FIPS filter
        // in isolation: peer offers X25519 (non-FIPS) and Secp256r1 (FIPS),
        // the FIPS-only server picks Secp256r1.
        let mut c = PqcConfig::fips_strict();
        c.require_post_quantum = false;
        let peer = vec![NamedGroup::X25519, NamedGroup::Secp256r1];
        assert_eq!(c.negotiate(&peer), Some(NamedGroup::Secp256r1));
    }

    #[test]
    fn negotiate_fips_strict_returns_none_when_only_classical_overlap() {
        let c = PqcConfig::fips_strict();
        let peer = vec![NamedGroup::X25519, NamedGroup::Secp256r1];
        // Strict mode requires PQ; classical-only overlap → no negotiation.
        assert!(c.negotiate(&peer).is_none());
    }

    #[test]
    fn negotiate_no_overlap_returns_none() {
        let c = PqcConfig::default();
        let peer = vec![NamedGroup::Secp521r1];
        assert!(c.negotiate(&peer).is_none());
    }

    // ── extension_bytes ──

    #[test]
    fn extension_bytes_length_matches_groups() {
        let c = PqcConfig::default();
        let b = c.extension_bytes();
        assert_eq!(b.len(), c.groups.len() * 2);
    }

    #[test]
    fn extension_bytes_big_endian() {
        let c = PqcConfig {
            groups: vec![NamedGroup::X25519],
            require_post_quantum: false,
            fips_only: false,
        };
        let b = c.extension_bytes();
        assert_eq!(b, vec![0x00, 0x1D]);
    }

    // ── min_pq_strength ──

    #[test]
    fn min_pq_strength_default() {
        // Default has X25519MlKem768 (192) and SecP256r1MlKem768 (192).
        assert_eq!(PqcConfig::default().min_pq_strength(), 192);
    }

    #[test]
    fn min_pq_strength_classical_only_zero() {
        assert_eq!(PqcConfig::legacy_classical_only().min_pq_strength(), 0);
    }

    #[test]
    fn min_pq_strength_mixed_192_and_256() {
        let c = PqcConfig {
            groups: vec![NamedGroup::MlKem768, NamedGroup::MlKem1024],
            require_post_quantum: true,
            fips_only: false,
        };
        assert_eq!(c.min_pq_strength(), 192);
    }

    // ── Serde ──

    #[test]
    fn pqc_config_json_roundtrip() {
        let c = PqcConfig::fips_strict();
        let s = serde_json::to_string(&c).unwrap();
        let c2: PqcConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }

    #[test]
    fn named_group_json_kebab() {
        assert_eq!(
            serde_json::to_string(&NamedGroup::X25519MlKem768).unwrap(),
            r#""x25519-ml-kem768""#
        );
    }
}
