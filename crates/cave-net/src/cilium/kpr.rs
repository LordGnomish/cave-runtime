//! kube-proxy-replacement (KPR) config.
//!
//! Mirrors `pkg/kpr/kpr.go`. The agent has a small per-feature config
//! cell that resolves the user-facing flags `--kube-proxy-replacement`
//! and `--bpf-lb-sock` into the effective `KprConfig`. Upstream's
//! invariant: when `kube-proxy-replacement` is true, socket-LB is forced
//! on regardless of the explicit flag value. We preserve that.

use crate::cilium::types::Cite;
use serde::{Deserialize, Serialize};

/// Raw flag values as provided on the command line (or via config-map).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KprFlags {
    pub kube_proxy_replacement: bool,
    /// Mapped from `--bpf-lb-sock`.
    pub enable_socket_lb: bool,
}

/// Resolved config after the upstream invariant has been applied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KprConfig {
    pub kube_proxy_replacement: bool,
    pub enable_socket_lb: bool,
}

/// Cilium flag names this module owns. They match the strings registered
/// in upstream `KPRFlags.Flags(*pflag.FlagSet)`.
pub mod flag {
    pub const KUBE_PROXY_REPLACEMENT: &str = "kube-proxy-replacement";
    pub const BPF_LB_SOCK: &str = "bpf-lb-sock";
}

/// Resolve raw flags into the effective config, applying the upstream
/// invariant `KubeProxyReplacement => EnableSocketLB`.
pub fn resolve(flags: KprFlags) -> KprConfig {
    let mut cfg = KprConfig {
        kube_proxy_replacement: flags.kube_proxy_replacement,
        enable_socket_lb: flags.enable_socket_lb,
    };
    if flags.kube_proxy_replacement {
        cfg.enable_socket_lb = true;
    }
    cfg
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/kpr/kpr.go", "KPRConfig");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn flag_names_match_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Flags.Names", "tenant-kpr-fn");
        assert_eq!(flag::KUBE_PROXY_REPLACEMENT, "kube-proxy-replacement");
        assert_eq!(flag::BPF_LB_SOCK, "bpf-lb-sock");
    }

    #[test]
    fn defaults_are_all_false() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Flags.Default", "tenant-kpr-d");
        let f = KprFlags::default();
        assert!(!f.kube_proxy_replacement);
        assert!(!f.enable_socket_lb);
    }

    #[test]
    fn resolve_passes_through_when_kpr_disabled() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Resolve.PassThrough", "tenant-kpr-rp");
        let cfg = resolve(KprFlags { kube_proxy_replacement: false, enable_socket_lb: false });
        assert!(!cfg.kube_proxy_replacement);
        assert!(!cfg.enable_socket_lb);
    }

    #[test]
    fn resolve_forces_socket_lb_when_kpr_enabled() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Resolve.ForceSockLB", "tenant-kpr-rf");
        let cfg = resolve(KprFlags { kube_proxy_replacement: true, enable_socket_lb: false });
        assert!(cfg.kube_proxy_replacement);
        assert!(cfg.enable_socket_lb, "KPR=true must force socket-LB on");
    }

    #[test]
    fn resolve_keeps_socket_lb_when_explicit_and_kpr_off() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Resolve.ExplicitSockLB", "tenant-kpr-re");
        let cfg = resolve(KprFlags { kube_proxy_replacement: false, enable_socket_lb: true });
        assert!(!cfg.kube_proxy_replacement);
        assert!(cfg.enable_socket_lb);
    }

    #[test]
    fn config_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Config.Serde", "tenant-kpr-cs");
        let c = KprConfig { kube_proxy_replacement: true, enable_socket_lb: true };
        let s = serde_json::to_string(&c).unwrap();
        let back: KprConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn flags_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Flags.Serde", "tenant-kpr-fs");
        let f = KprFlags { kube_proxy_replacement: true, enable_socket_lb: false };
        let s = serde_json::to_string(&f).unwrap();
        let back: KprFlags = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn resolve_is_idempotent() {
        let (_c, _t) = cilium_test_ctx!("pkg/kpr/kpr.go", "Resolve.Idempotent", "tenant-kpr-ri");
        let f = KprFlags { kube_proxy_replacement: true, enable_socket_lb: false };
        let a = resolve(f);
        let b = resolve(KprFlags { kube_proxy_replacement: a.kube_proxy_replacement, enable_socket_lb: a.enable_socket_lb });
        assert_eq!(a, b);
    }
}
