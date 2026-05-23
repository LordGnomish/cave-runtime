// SPDX-License-Identifier: AGPL-3.0-or-later
//! Post-quantum TLS key exchange — ML-KEM + hybrid X25519+ML-KEM (codepoint 0x11ec).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KemMode { Classical, PqcOnly, Hybrid }
impl Default for KemMode { fn default() -> Self { Self::Classical } }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KemPolicy { pub mode: KemMode, pub advertise_hybrid_codepoint: bool }
impl Default for KemPolicy {
    fn default() -> Self { Self { mode: KemMode::Classical, advertise_hybrid_codepoint: false } }
}
impl KemPolicy {
    pub fn enable_hybrid() -> Self { Self { mode: KemMode::Hybrid, advertise_hybrid_codepoint: true } }
    pub fn group_codepoint(&self) -> Option<u16> {
        match self.mode {
            KemMode::Classical => Some(0x001d),
            KemMode::PqcOnly => Some(0x4242),
            KemMode::Hybrid => Some(0x11ec),
        }
    }
    pub fn is_quantum_resistant(&self) -> bool { matches!(self.mode, KemMode::PqcOnly | KemMode::Hybrid) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn classical_default() {
        let p = KemPolicy::default();
        assert_eq!(p.mode, KemMode::Classical);
        assert!(!p.is_quantum_resistant());
        assert_eq!(p.group_codepoint(), Some(0x001d));
    }
    #[test] fn hybrid_codepoint() {
        let p = KemPolicy::enable_hybrid();
        assert_eq!(p.group_codepoint(), Some(0x11ec));
        assert!(p.is_quantum_resistant());
        assert!(p.advertise_hybrid_codepoint);
    }
    #[test] fn pqc_only_quantum_resistant() {
        let p = KemPolicy { mode: KemMode::PqcOnly, advertise_hybrid_codepoint: false };
        assert!(p.is_quantum_resistant());
    }
}
