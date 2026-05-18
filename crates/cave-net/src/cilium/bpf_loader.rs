// SPDX-License-Identifier: AGPL-3.0-or-later
//! BPF program loader simulator — mirrors the agent-side loader that
//! consumes compiled `.o` ELFs and attaches them to TC / cgroup / XDP
//! hooks.
//!
//! Mirrors `pkg/datapath/loader/loader.go` plus the program-graph
//! definitions in `bpf/bpf_lxc.c`, `bpf/bpf_host.c`, `bpf/bpf_overlay.c`.
//! In production this would call into libbpf via the `ebpf-go`
//! bindings; we model the loader's *state-keeping* (which programs
//! are loaded, what they're attached to, what the verifier said).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BpfProgKind {
    /// `BPF_PROG_TYPE_SCHED_CLS` — TC ingress/egress classifier.
    SchedCls,
    /// `BPF_PROG_TYPE_XDP` — XDP entry point.
    Xdp,
    /// `BPF_PROG_TYPE_CGROUP_SOCK_ADDR` — connect4/sendmsg/etc.
    CgroupSockAddr,
    /// `BPF_PROG_TYPE_CGROUP_SOCK` — sock create/release.
    CgroupSock,
    /// `BPF_PROG_TYPE_SOCK_OPS` — TCP sockops hooks.
    SockOps,
    /// `BPF_PROG_TYPE_LWT_*` — lightweight tunnel programs.
    LwtIn,
    LwtOut,
}

impl BpfProgKind {
    pub fn name(self) -> &'static str {
        match self {
            BpfProgKind::SchedCls => "BPF_PROG_TYPE_SCHED_CLS",
            BpfProgKind::Xdp => "BPF_PROG_TYPE_XDP",
            BpfProgKind::CgroupSockAddr => "BPF_PROG_TYPE_CGROUP_SOCK_ADDR",
            BpfProgKind::CgroupSock => "BPF_PROG_TYPE_CGROUP_SOCK",
            BpfProgKind::SockOps => "BPF_PROG_TYPE_SOCK_OPS",
            BpfProgKind::LwtIn => "BPF_PROG_TYPE_LWT_IN",
            BpfProgKind::LwtOut => "BPF_PROG_TYPE_LWT_OUT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BpfProgram {
    pub object_path: String, // e.g. "bpf_lxc.o"
    pub section: String,     // e.g. "from-container"
    pub kind: BpfProgKind,
    pub instructions: u32,
    pub map_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachPoint {
    pub kind: BpfProgKind,
    pub iface_or_cgroup: String,
    pub direction: AttachDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachDirection {
    Ingress,
    Egress,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierResult {
    pub ok: bool,
    pub log: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LoaderError {
    #[error("program `{0}` not loaded")]
    ProgNotLoaded(String),
    #[error("attach point `{0}` already in use by `{1}`")]
    AttachInUse(String, String),
    #[error("verifier rejected `{0}`: {1}")]
    VerifierRejected(String, String),
    #[error("tenant {tenant} cannot mutate loader owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct BpfLoader {
    pub tenant: TenantId,
    /// section-name → loaded BpfProgram.
    loaded: BTreeMap<String, BpfProgram>,
    /// "iface:dir" or "cgroup:path" → section-name attached.
    attached: BTreeMap<String, String>,
    /// Verifier results per program (latest).
    verifier_log: BTreeMap<String, VerifierResult>,
}

impl BpfLoader {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            loaded: BTreeMap::new(),
            attached: BTreeMap::new(),
            verifier_log: BTreeMap::new(),
        }
    }

    /// Load a program. Mirrors `loader.LoadCollection`.
    pub fn load(&mut self, prog: BpfProgram) -> Result<(), LoaderError> {
        let verdict = simulated_verifier(&prog);
        self.verifier_log.insert(prog.section.clone(), verdict.clone());
        if !verdict.ok {
            return Err(LoaderError::VerifierRejected(prog.section.clone(), verdict.log));
        }
        self.loaded.insert(prog.section.clone(), prog);
        Ok(())
    }

    pub fn unload(&mut self, section: &str) -> Result<(), LoaderError> {
        self.loaded.remove(section).ok_or_else(|| LoaderError::ProgNotLoaded(section.to_string()))?;
        self.attached.retain(|_, sec| sec != section);
        Ok(())
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded.len()
    }

    pub fn attach(&mut self, section: &str, point: AttachPoint) -> Result<String, LoaderError> {
        if !self.loaded.contains_key(section) {
            return Err(LoaderError::ProgNotLoaded(section.to_string()));
        }
        let key = attach_key(&point);
        if let Some(existing) = self.attached.get(&key) {
            if existing != section {
                return Err(LoaderError::AttachInUse(key, existing.clone()));
            }
        }
        self.attached.insert(key.clone(), section.to_string());
        Ok(key)
    }

    pub fn detach(&mut self, point: &AttachPoint) -> bool {
        let key = attach_key(point);
        self.attached.remove(&key).is_some()
    }

    pub fn attached_count(&self) -> usize {
        self.attached.len()
    }

    pub fn attached_program_at(&self, point: &AttachPoint) -> Option<&str> {
        self.attached.get(&attach_key(point)).map(|s| s.as_str())
    }

    pub fn verifier_result(&self, section: &str) -> Option<&VerifierResult> {
        self.verifier_log.get(section)
    }
}

fn attach_key(p: &AttachPoint) -> String {
    let dir = match p.direction {
        AttachDirection::Ingress => "ingress",
        AttachDirection::Egress => "egress",
        AttachDirection::None => "*",
    };
    format!("{}:{}:{}", p.kind.name(), p.iface_or_cgroup, dir)
}

/// Simulated verifier: rejects programs over 1M instructions or
/// referencing maps that look invalid (`""` or starting with `_`).
fn simulated_verifier(prog: &BpfProgram) -> VerifierResult {
    if prog.instructions > 1_000_000 {
        return VerifierResult {
            ok: false,
            log: format!("program too large: {} instructions (max 1M)", prog.instructions),
        };
    }
    for m in &prog.map_refs {
        if m.is_empty() || m.starts_with('_') {
            return VerifierResult {
                ok: false,
                log: format!("invalid map reference: `{m}`"),
            };
        }
    }
    VerifierResult { ok: true, log: "verifier accepted".into() }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/datapath/loader/loader.go", "Loader");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn loader(tenant: TenantId) -> BpfLoader {
        BpfLoader::new(tenant)
    }

    fn good_program(section: &str) -> BpfProgram {
        BpfProgram {
            object_path: "bpf_lxc.o".into(),
            section: section.into(),
            kind: BpfProgKind::SchedCls,
            instructions: 1024,
            map_refs: vec!["cilium_ipcache".into()],
        }
    }

    fn attach(kind: BpfProgKind, iface: &str, dir: AttachDirection) -> AttachPoint {
        AttachPoint { kind, iface_or_cgroup: iface.into(), direction: dir }
    }

    // ── ProgKind ───────────────────────────────────────────────────────────

    #[test]
    fn prog_kind_names_match_kernel() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "ProgKind.Name", "tenant-bl-pn");
        assert_eq!(BpfProgKind::SchedCls.name(), "BPF_PROG_TYPE_SCHED_CLS");
        assert_eq!(BpfProgKind::Xdp.name(), "BPF_PROG_TYPE_XDP");
        assert_eq!(BpfProgKind::CgroupSockAddr.name(), "BPF_PROG_TYPE_CGROUP_SOCK_ADDR");
        assert_eq!(BpfProgKind::SockOps.name(), "BPF_PROG_TYPE_SOCK_OPS");
    }

    // ── Load ───────────────────────────────────────────────────────────────

    #[test]
    fn load_good_program_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Load", "tenant-bl-l");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        assert_eq!(l.loaded_count(), 1);
    }

    #[test]
    fn load_too_large_program_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Load.TooLarge", "tenant-bl-ltl");
        let mut l = loader(tenant);
        let mut p = good_program("from-container");
        p.instructions = 2_000_000;
        let err = l.load(p).unwrap_err();
        assert!(matches!(err, LoaderError::VerifierRejected(_, _)));
    }

    #[test]
    fn load_with_invalid_map_ref_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Load.InvalidMap", "tenant-bl-lim");
        let mut l = loader(tenant);
        let mut p = good_program("from-container");
        p.map_refs = vec!["_internal".into()];
        let err = l.load(p).unwrap_err();
        assert!(matches!(err, LoaderError::VerifierRejected(_, _)));
    }

    #[test]
    fn load_records_verifier_result() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Load.VerifierLog", "tenant-bl-lv");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        assert!(l.verifier_result("from-container").unwrap().ok);
    }

    #[test]
    fn load_failed_records_verifier_log() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Load.VerifierLog.Failure", "tenant-bl-lvf");
        let mut l = loader(tenant);
        let mut p = good_program("from-container");
        p.instructions = 5_000_000;
        let _ = l.load(p);
        let r = l.verifier_result("from-container").unwrap();
        assert!(!r.ok);
        assert!(r.log.contains("too large"));
    }

    // ── Unload ─────────────────────────────────────────────────────────────

    #[test]
    fn unload_drops_program() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Unload", "tenant-bl-u");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        l.unload("from-container").unwrap();
        assert_eq!(l.loaded_count(), 0);
    }

    #[test]
    fn unload_unknown_returns_not_loaded() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Unload.NotLoaded", "tenant-bl-unl");
        let mut l = loader(tenant);
        let err = l.unload("ghost").unwrap_err();
        assert!(matches!(err, LoaderError::ProgNotLoaded(_)));
    }

    #[test]
    fn unload_also_drops_attachments() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Unload.DropAttach", "tenant-bl-uda");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        l.unload("from-container").unwrap();
        assert_eq!(l.attached_count(), 0);
    }

    // ── Attach ─────────────────────────────────────────────────────────────

    #[test]
    fn attach_loaded_program_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Attach", "tenant-bl-a");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        let key = l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        assert!(key.contains("eth0"));
    }

    #[test]
    fn attach_unloaded_program_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Attach.NotLoaded", "tenant-bl-anl");
        let mut l = loader(tenant);
        let err = l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap_err();
        assert!(matches!(err, LoaderError::ProgNotLoaded(_)));
    }

    #[test]
    fn attach_in_use_by_other_program_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Attach.InUse", "tenant-bl-aiu");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        let mut p2 = good_program("from-host");
        p2.section = "from-host".into();
        l.load(p2).unwrap();
        l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        let err = l.attach("from-host", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap_err();
        assert!(matches!(err, LoaderError::AttachInUse(_, _)));
    }

    #[test]
    fn attach_same_program_twice_idempotent() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Attach.Idempotent", "tenant-bl-aid");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        assert_eq!(l.attached_count(), 1);
    }

    #[test]
    fn detach_drops_attachment() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Detach", "tenant-bl-d");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        assert!(l.detach(&attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)));
        assert_eq!(l.attached_count(), 0);
    }

    #[test]
    fn detach_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Detach.NotFound", "tenant-bl-dnf");
        let mut l = loader(tenant);
        assert!(!l.detach(&attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)));
    }

    #[test]
    fn attached_program_at_returns_section_name() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "AttachedAt", "tenant-bl-at");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        let p = attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress);
        l.attach("from-container", p.clone()).unwrap();
        assert_eq!(l.attached_program_at(&p), Some("from-container"));
    }

    #[test]
    fn attached_program_at_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "AttachedAt.NotFound", "tenant-bl-atnf");
        let l = loader(tenant);
        assert!(l.attached_program_at(&attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).is_none());
    }

    // ── Different attach points ────────────────────────────────────────────

    #[test]
    fn ingress_and_egress_attach_points_distinct() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "AttachPoints.Distinct", "tenant-bl-apd");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        l.load(BpfProgram {
            object_path: "bpf_lxc.o".into(), section: "to-container".into(),
            kind: BpfProgKind::SchedCls, instructions: 1024,
            map_refs: vec!["cilium_ipcache".into()],
        }).unwrap();
        l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        l.attach("to-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Egress)).unwrap();
        assert_eq!(l.attached_count(), 2);
    }

    #[test]
    fn xdp_attach_distinct_from_tc_attach() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Attach.XdpVsTc", "tenant-bl-axt");
        let mut l = loader(tenant);
        l.load(good_program("from-container")).unwrap();
        l.load(BpfProgram {
            object_path: "bpf_xdp.o".into(), section: "from-netdev-xdp".into(),
            kind: BpfProgKind::Xdp, instructions: 2048,
            map_refs: vec!["cilium_lb4_services".into()],
        }).unwrap();
        l.attach("from-container", attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress)).unwrap();
        l.attach("from-netdev-xdp", attach(BpfProgKind::Xdp, "eth0", AttachDirection::None)).unwrap();
        assert_eq!(l.attached_count(), 2);
    }

    // ── Counts ─────────────────────────────────────────────────────────────

    #[test]
    fn loaded_count_tracks_loads() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Count", "tenant-bl-c");
        let mut l = loader(tenant);
        for s in ["a", "b", "c"] {
            l.load(BpfProgram {
                object_path: format!("{s}.o"), section: s.into(),
                kind: BpfProgKind::SchedCls, instructions: 1000,
                map_refs: vec![],
            }).unwrap();
        }
        assert_eq!(l.loaded_count(), 3);
    }

    // ── Verifier ───────────────────────────────────────────────────────────

    #[test]
    fn verifier_result_unknown_section_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Verifier.NotFound", "tenant-bl-vnf");
        let l = loader(tenant);
        assert!(l.verifier_result("ghost").is_none());
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn bpf_program_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Program.Serde", "tenant-bl-pserde");
        let p = good_program("from-container");
        let s = serde_json::to_string(&p).unwrap();
        let back: BpfProgram = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn attach_point_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "AttachPoint.Serde", "tenant-bl-aserde");
        let a = attach(BpfProgKind::SchedCls, "eth0", AttachDirection::Ingress);
        let s = serde_json::to_string(&a).unwrap();
        let back: AttachPoint = serde_json::from_str(&s).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn verifier_result_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/loader/loader.go", "Verifier.Serde", "tenant-bl-vserde");
        let v = VerifierResult { ok: true, log: "ok".into() };
        let s = serde_json::to_string(&v).unwrap();
        let back: VerifierResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
    }
}
