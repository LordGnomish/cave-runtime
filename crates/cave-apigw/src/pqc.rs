// SPDX-License-Identifier: AGPL-3.0-or-later
//! Post-quantum TLS key exchange — ML-KEM + hybrid X25519+ML-KEM.
//!
//! Wire policy: when `pqc_hybrid_enabled` is set, the gateway advertises the
//! `X25519MLKEM768` group code (codepoint `0x11ec`, draft-ietf-tls-hybrid-design-10).
//! The real KEM is delegated to the runtime adapter; this module exposes the
//! policy + suite-selection logic for tests and Admin API surface.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KemMode {
    /// Classical only — X25519.
    Classical,
    /// PQC only — ML-KEM-768.
    PqcOnly,
    /// Hybrid — X25519 + ML-KEM-768 concatenated shared secret.
    Hybrid,
}

impl Default for KemMode {
    fn default() -> Self { Self::Classical }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KemPolicy {
    pub mode: KemMode,
    pub advertise_hybrid_codepoint: bool,
}

impl Default for KemPolicy {
    fn default() -> Self {
        Self { mode: KemMode::Classical, advertise_hybrid_codepoint: false }
    }
}

impl KemPolicy {
    pub fn enable_hybrid() -> Self {
        Self { mode: KemMode::Hybrid, advertise_hybrid_codepoint: true }
    }

    /// Wire codepoint to advertise on ClientHello key_share extensions, if any.
    pub fn group_codepoint(&self) -> Option<u16> {
        match self.mode {
            KemMode::Classical => Some(0x001d), // X25519
            KemMode::PqcOnly => Some(0x4242),   // experimental ML-KEM-768 only (TBD)
            KemMode::Hybrid => Some(0x11ec),    // X25519MLKEM768 (hybrid)
        }
    }

    pub fn is_quantum_resistant(&self) -> bool {
        matches!(self.mode, KemMode::PqcOnly | KemMode::Hybrid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_classical() {
        let p = KemPolicy::default();
        assert_eq!(p.mode, KemMode::Classical);
        assert!(!p.is_quantum_resistant());
        assert_eq!(p.group_codepoint(), Some(0x001d));
    }

    #[test]
    fn hybrid_advertises_correct_codepoint() {
        let p = KemPolicy::enable_hybrid();
        assert_eq!(p.group_codepoint(), Some(0x11ec));
        assert!(p.is_quantum_resistant());
        assert!(p.advertise_hybrid_codepoint);
    }

    #[test]
    fn pqc_only_quantum_resistant() {
        let p = KemPolicy { mode: KemMode::PqcOnly, advertise_hybrid_codepoint: false };
        assert!(p.is_quantum_resistant());
    }
}
