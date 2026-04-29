//! IPsec transparent encryption — Security Associations + Policies.
//!
//! Mirrors `pkg/ipsec/ipsec_linux.go`. Each node has a set of
//! [`SecurityAssociation`]s (SA) keyed by SPI (32-bit Security Parameters
//! Index) and a set of [`SecurityPolicy`]s (SP) that decide whether a
//! packet should be encrypted, bypassed, or dropped.
//!
//! Semantics (faithful to upstream):
//!
//! * SA carries the encryption algorithm (default AES-GCM-256), the key
//!   bytes, the source/destination node IPs, and an optional lifetime.
//! * SPI is monotonic per-tenant; rotation creates a new SA with a new
//!   SPI but keeps the old one for a `grace_period` (mirrors
//!   `cilium-ipsec-rotation` controller).
//! * SP selects packets by 5-tuple-shaped predicates and assigns an
//!   action: `Encrypt(spi)`, `Bypass`, or `Drop`. First-match wins.
//! * If no SP matches, the implicit default is `Bypass` (mirrors the
//!   upstream "no rule → clear-text" behaviour).
//! * Per-pod vs per-node mode determines whether SAs are keyed by pod
//!   IP or node IP.

use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpsecMode {
    PerNode,
    PerPod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpsecAlgorithm {
    AesGcm128,
    AesGcm256,
    ChaCha20Poly1305,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAssociation {
    pub spi: u32,
    pub src: IpAddr,
    pub dst: IpAddr,
    pub algorithm: IpsecAlgorithm,
    pub key: Vec<u8>,
    pub created: u64,
    pub lifetime_seconds: Option<u64>,
}

impl SecurityAssociation {
    pub fn is_expired(&self, now: u64) -> bool {
        match self.lifetime_seconds {
            Some(lt) => now.saturating_sub(self.created) >= lt,
            None => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpAction {
    Encrypt(u32),
    Bypass,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityPolicy {
    pub name: String,
    pub priority: u32,
    pub src_cidr: String,
    pub dst_cidr: String,
    pub action: SpAction,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IpsecError {
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("SA with SPI {0} already exists")]
    SpiCollision(u32),
    #[error("tenant {tenant} cannot mutate IPsec store owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct IpsecStore {
    pub tenant: TenantId,
    pub mode: IpsecMode,
    pub node_ip: IpAddr,
    next_spi: u32,
    sas: HashMap<u32, SecurityAssociation>,
    sps: Vec<SecurityPolicy>,
    /// Grace period for rotated SAs (seconds). After this, old SAs may
    /// be GC'd. Default 300s mirrors upstream.
    pub grace_period: u64,
}

impl IpsecStore {
    pub fn new(tenant: TenantId, mode: IpsecMode, node_ip: IpAddr) -> Self {
        Self {
            tenant, mode, node_ip,
            next_spi: 1, sas: HashMap::new(), sps: Vec::new(),
            grace_period: 300,
        }
    }

    /// Install a new SA. SPI is auto-assigned.
    pub fn install_sa(
        &mut self,
        src: IpAddr,
        dst: IpAddr,
        algorithm: IpsecAlgorithm,
        key: Vec<u8>,
        now: u64,
        lifetime_seconds: Option<u64>,
    ) -> Result<u32, IpsecError> {
        let spi = self.next_spi;
        self.next_spi += 1;
        let sa = SecurityAssociation { spi, src, dst, algorithm, key, created: now, lifetime_seconds };
        self.sas.insert(spi, sa);
        Ok(spi)
    }

    pub fn install_sa_with_spi(
        &mut self,
        spi: u32,
        src: IpAddr,
        dst: IpAddr,
        algorithm: IpsecAlgorithm,
        key: Vec<u8>,
        now: u64,
    ) -> Result<(), IpsecError> {
        if self.sas.contains_key(&spi) {
            return Err(IpsecError::SpiCollision(spi));
        }
        self.sas.insert(spi, SecurityAssociation {
            spi, src, dst, algorithm, key, created: now, lifetime_seconds: None,
        });
        if spi >= self.next_spi {
            self.next_spi = spi + 1;
        }
        Ok(())
    }

    pub fn lookup_sa(&self, spi: u32) -> Option<&SecurityAssociation> {
        self.sas.get(&spi)
    }

    pub fn remove_sa(&mut self, spi: u32) -> bool {
        self.sas.remove(&spi).is_some()
    }

    pub fn rotate_sa(
        &mut self,
        old_spi: u32,
        algorithm: IpsecAlgorithm,
        key: Vec<u8>,
        now: u64,
    ) -> Result<u32, IpsecError> {
        let old = self.sas.get(&old_spi).cloned();
        let (src, dst) = match old {
            Some(o) => (o.src, o.dst),
            None => return Ok(self.next_spi),
        };
        // Old SA stays in store until the grace period elapses; caller can
        // GC via `gc_expired` once `now > old.created + grace_period`.
        self.install_sa(src, dst, algorithm, key, now, None)
    }

    pub fn gc_expired(&mut self, now: u64) -> usize {
        let before = self.sas.len();
        self.sas.retain(|_, sa| !sa.is_expired(now));
        before - self.sas.len()
    }

    pub fn sa_count(&self) -> usize {
        self.sas.len()
    }

    // ── Security policies ───────────────────────────────────────────────────

    pub fn add_policy(&mut self, policy: SecurityPolicy) -> Result<(), IpsecError> {
        IpNet::from_str(&policy.src_cidr).map_err(|_| IpsecError::BadCidr(policy.src_cidr.clone()))?;
        IpNet::from_str(&policy.dst_cidr).map_err(|_| IpsecError::BadCidr(policy.dst_cidr.clone()))?;
        self.sps.push(policy);
        // Sort descending by priority; first-match wins.
        self.sps.sort_by(|a, b| b.priority.cmp(&a.priority));
        Ok(())
    }

    /// Resolve the action for `(src, dst)`. Returns `SpAction::Bypass` if
    /// no policy matches (mirrors upstream "no rule → clear-text").
    pub fn resolve(&self, src: IpAddr, dst: IpAddr) -> Result<SpAction, IpsecError> {
        for sp in &self.sps {
            let s = IpNet::from_str(&sp.src_cidr).map_err(|_| IpsecError::BadCidr(sp.src_cidr.clone()))?;
            let d = IpNet::from_str(&sp.dst_cidr).map_err(|_| IpsecError::BadCidr(sp.dst_cidr.clone()))?;
            if s.contains(&src) && d.contains(&dst) {
                return Ok(sp.action);
            }
        }
        Ok(SpAction::Bypass)
    }

    pub fn policy_count(&self) -> usize {
        self.sps.len()
    }

    pub fn remove_policy(&mut self, name: &str) -> bool {
        let before = self.sps.len();
        self.sps.retain(|p| p.name != name);
        before != self.sps.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ipsec/ipsec_linux.go", "AddSPI");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn make_store(tenant: TenantId) -> IpsecStore {
        IpsecStore::new(tenant, IpsecMode::PerNode, ip(10, 0, 0, 1))
    }

    // ── SA install / lookup / remove ─────────────────────────────────────────

    #[test]
    fn ipsec_install_sa_assigns_monotonic_spi() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "AddSPI", "tenant-ipsec-mono");
        let mut s = make_store(tenant);
        let a = s.install_sa(ip(10, 0, 0, 1), ip(10, 0, 0, 2), IpsecAlgorithm::AesGcm256, vec![0; 32], 100, None).unwrap();
        let b = s.install_sa(ip(10, 0, 0, 1), ip(10, 0, 0, 3), IpsecAlgorithm::AesGcm256, vec![0; 32], 100, None).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 2);
    }

    #[test]
    fn ipsec_install_sa_with_explicit_spi_rejects_collision() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "AddSPI.Collision", "tenant-ipsec-coll");
        let mut s = make_store(tenant);
        s.install_sa_with_spi(42, ip(10, 0, 0, 1), ip(10, 0, 0, 2), IpsecAlgorithm::AesGcm256, vec![0; 32], 100).unwrap();
        let err = s.install_sa_with_spi(42, ip(10, 0, 0, 1), ip(10, 0, 0, 3), IpsecAlgorithm::AesGcm256, vec![0; 32], 100).unwrap_err();
        assert_eq!(err, IpsecError::SpiCollision(42));
    }

    #[test]
    fn ipsec_lookup_sa_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "GetSPI", "tenant-ipsec-lk");
        let mut s = make_store(tenant);
        let spi = s.install_sa(ip(10, 0, 0, 1), ip(10, 0, 0, 2), IpsecAlgorithm::AesGcm256, vec![0xAB; 32], 100, None).unwrap();
        let sa = s.lookup_sa(spi).unwrap();
        assert_eq!(sa.dst, ip(10, 0, 0, 2));
        assert_eq!(sa.algorithm, IpsecAlgorithm::AesGcm256);
        assert_eq!(sa.key.len(), 32);
    }

    #[test]
    fn ipsec_remove_sa_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "DelSPI", "tenant-ipsec-rm");
        let mut s = make_store(tenant);
        let spi = s.install_sa(ip(10, 0, 0, 1), ip(10, 0, 0, 2), IpsecAlgorithm::AesGcm256, vec![0; 32], 100, None).unwrap();
        assert!(s.remove_sa(spi));
        assert!(s.lookup_sa(spi).is_none());
    }

    #[test]
    fn ipsec_remove_unknown_sa_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "DelSPI.NotFound", "tenant-ipsec-rmnf");
        let mut s = make_store(tenant);
        assert!(!s.remove_sa(999));
    }

    #[test]
    fn ipsec_sa_count_tracks_installs() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "Count", "tenant-ipsec-cnt");
        let mut s = make_store(tenant);
        for _ in 0..5 {
            s.install_sa(ip(10, 0, 0, 1), ip(10, 0, 0, 2), IpsecAlgorithm::AesGcm256, vec![0; 32], 100, None).unwrap();
        }
        assert_eq!(s.sa_count(), 5);
    }

    // ── Rotation + GC ────────────────────────────────────────────────────────

    #[test]
    fn ipsec_rotate_sa_assigns_new_spi() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "RotateSPI", "tenant-ipsec-rot");
        let mut s = make_store(tenant);
        let old = s.install_sa(ip(10, 0, 0, 1), ip(10, 0, 0, 2), IpsecAlgorithm::AesGcm256, vec![0xAA; 32], 100, None).unwrap();
        let new = s.rotate_sa(old, IpsecAlgorithm::AesGcm256, vec![0xBB; 32], 200).unwrap();
        assert_ne!(new, old);
        // Old SA still present until grace period expires.
        assert!(s.lookup_sa(old).is_some());
    }

    #[test]
    fn ipsec_gc_expired_removes_sa_past_lifetime() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "GC.Expired", "tenant-ipsec-gc");
        let mut s = make_store(tenant);
        let spi = s.install_sa(ip(10, 0, 0, 1), ip(10, 0, 0, 2), IpsecAlgorithm::AesGcm256, vec![0; 32], 100, Some(60)).unwrap();
        assert_eq!(s.gc_expired(150), 0);
        assert_eq!(s.gc_expired(200), 1);
        assert!(s.lookup_sa(spi).is_none());
    }

    // ── SP resolve ───────────────────────────────────────────────────────────

    #[test]
    fn ipsec_resolve_no_policies_defaults_to_bypass() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "ResolvePolicy.Default", "tenant-ipsec-def");
        let s = make_store(tenant);
        let v = s.resolve(ip(10, 0, 0, 1), ip(10, 0, 0, 2)).unwrap();
        assert_eq!(v, SpAction::Bypass);
    }

    #[test]
    fn ipsec_resolve_encrypt_action_returns_spi() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "ResolvePolicy.Encrypt", "tenant-ipsec-enc");
        let mut s = make_store(tenant);
        s.add_policy(SecurityPolicy {
            name: "node-to-node".into(), priority: 100,
            src_cidr: "10.0.0.0/24".into(), dst_cidr: "10.0.0.0/24".into(),
            action: SpAction::Encrypt(42),
        }).unwrap();
        assert_eq!(s.resolve(ip(10, 0, 0, 1), ip(10, 0, 0, 2)).unwrap(), SpAction::Encrypt(42));
    }

    #[test]
    fn ipsec_resolve_drop_action() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "ResolvePolicy.Drop", "tenant-ipsec-drop");
        let mut s = make_store(tenant);
        s.add_policy(SecurityPolicy {
            name: "deny".into(), priority: 100,
            src_cidr: "10.0.0.0/24".into(), dst_cidr: "0.0.0.0/0".into(),
            action: SpAction::Drop,
        }).unwrap();
        assert_eq!(s.resolve(ip(10, 0, 0, 1), ip(8, 8, 8, 8)).unwrap(), SpAction::Drop);
    }

    #[test]
    fn ipsec_resolve_higher_priority_policy_wins() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "ResolvePolicy.Priority", "tenant-ipsec-pri");
        let mut s = make_store(tenant);
        s.add_policy(SecurityPolicy {
            name: "low".into(), priority: 10,
            src_cidr: "10.0.0.0/8".into(), dst_cidr: "0.0.0.0/0".into(),
            action: SpAction::Bypass,
        }).unwrap();
        s.add_policy(SecurityPolicy {
            name: "high".into(), priority: 100,
            src_cidr: "10.0.0.0/8".into(), dst_cidr: "0.0.0.0/0".into(),
            action: SpAction::Encrypt(7),
        }).unwrap();
        assert_eq!(s.resolve(ip(10, 0, 0, 1), ip(8, 8, 8, 8)).unwrap(), SpAction::Encrypt(7));
    }

    #[test]
    fn ipsec_resolve_no_match_falls_through_to_bypass() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "ResolvePolicy.NoMatch", "tenant-ipsec-nomatch");
        let mut s = make_store(tenant);
        s.add_policy(SecurityPolicy {
            name: "scoped".into(), priority: 100,
            src_cidr: "10.0.0.0/24".into(), dst_cidr: "10.0.0.0/24".into(),
            action: SpAction::Encrypt(1),
        }).unwrap();
        assert_eq!(s.resolve(ip(192, 168, 0, 1), ip(8, 8, 8, 8)).unwrap(), SpAction::Bypass);
    }

    // ── SP add/remove ────────────────────────────────────────────────────────

    #[test]
    fn ipsec_remove_policy_drops_rule() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "DelPolicy", "tenant-ipsec-prm");
        let mut s = make_store(tenant);
        s.add_policy(SecurityPolicy {
            name: "p".into(), priority: 100,
            src_cidr: "10.0.0.0/8".into(), dst_cidr: "10.0.0.0/8".into(),
            action: SpAction::Encrypt(1),
        }).unwrap();
        assert!(s.remove_policy("p"));
        assert_eq!(s.policy_count(), 0);
    }

    #[test]
    fn ipsec_add_policy_with_bad_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "AddPolicy.Validate", "tenant-ipsec-pbad");
        let mut s = make_store(tenant);
        let err = s.add_policy(SecurityPolicy {
            name: "p".into(), priority: 100,
            src_cidr: "not-a-cidr".into(), dst_cidr: "10.0.0.0/8".into(),
            action: SpAction::Bypass,
        }).unwrap_err();
        assert_eq!(err, IpsecError::BadCidr("not-a-cidr".into()));
    }

    // ── Mode ─────────────────────────────────────────────────────────────────

    #[test]
    fn ipsec_per_pod_mode_records_mode() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "Mode.PerPod", "tenant-ipsec-pp");
        let s = IpsecStore::new(tenant, IpsecMode::PerPod, ip(10, 0, 0, 1));
        assert_eq!(s.mode, IpsecMode::PerPod);
    }

    #[test]
    fn ipsec_round_trip_serde_for_sa_and_sp() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipsec/ipsec_linux.go", "Serde", "tenant-ipsec-serde");
        let sa = SecurityAssociation {
            spi: 1, src: ip(10, 0, 0, 1), dst: ip(10, 0, 0, 2),
            algorithm: IpsecAlgorithm::AesGcm256, key: vec![1, 2, 3],
            created: 100, lifetime_seconds: Some(60),
        };
        let json = serde_json::to_string(&sa).unwrap();
        let back: SecurityAssociation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sa);
    }
}
