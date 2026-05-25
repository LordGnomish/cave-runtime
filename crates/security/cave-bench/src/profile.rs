// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Profile manager — predefined bundles of checks.
//!
//! Upstream: kube-bench `cfg/config.yaml::target_mapping` + kubescape
//! `regolibrary/frameworks/{NSA,MITRE,cis-...}.json`.

use crate::cis_control_plane::control_plane_checks;
use crate::cis_etcd::etcd_checks;
use crate::cis_master::master_checks;
use crate::cis_node::node_checks;
use crate::error::{BenchError, Result};
use crate::kubescape_mitre::mitre_techniques;
use crate::kubescape_nsa::nsa_controls;
use crate::models::{Framework, Profile};

/// Built-in profile catalogue.
pub fn builtin_profiles() -> Vec<Profile> {
    vec![
        Profile {
            id: "cis-1.9".into(),
            framework: Framework::CisK8s,
            name: "CIS Kubernetes Benchmark v1.9".into(),
            description: "Master + node + etcd + control-plane controls — CIS v1.9 baseline.".into(),
            check_ids: all_cis_ids(),
        },
        Profile {
            id: "cis-1.10".into(),
            framework: Framework::CisK8s,
            name: "CIS Kubernetes Benchmark v1.10".into(),
            description: "Master + node + etcd + control-plane controls — CIS v1.10 (kube-bench v0.15.5).".into(),
            check_ids: all_cis_ids(),
        },
        Profile {
            id: "nsa-2025".into(),
            framework: Framework::NsaHardening,
            name: "NSA Kubernetes Hardening Guide".into(),
            description: "NSA/CISA Cybersecurity Technical Report — Kubernetes Hardening Guidance.".into(),
            check_ids: nsa_controls().iter().map(|c| c.check.id.clone()).collect(),
        },
        Profile {
            id: "mitre-attck-k8s".into(),
            framework: Framework::MitreAttack,
            name: "MITRE ATT&CK for Kubernetes".into(),
            description: "Detection coverage across 10 ATT&CK tactics for Kubernetes-native attacks.".into(),
            check_ids: mitre_techniques().iter().map(|t| t.id.clone()).collect(),
        },
        Profile {
            id: "soc2-cc-7".into(),
            framework: Framework::SocControls,
            name: "SOC 2 CC7 — Cluster runtime hygiene".into(),
            description: "Distilled SOC2-CC7-aligned subset of CIS-master + NSA + MITRE T1611/T1485.".into(),
            check_ids: soc2_cc7_ids(),
        },
    ]
}

fn all_cis_ids() -> Vec<String> {
    let mut ids: Vec<String> = master_checks().iter().map(|(c, _)| c.id.clone()).collect();
    ids.extend(node_checks().iter().map(|(c, _)| c.id.clone()));
    ids.extend(etcd_checks().iter().map(|(c, _)| c.id.clone()));
    ids.extend(control_plane_checks().iter().map(|(c, _)| c.id.clone()));
    ids
}

fn soc2_cc7_ids() -> Vec<String> {
    // High-impact subset: audit-policy + RBAC + host-namespace + impact techniques.
    vec![
        "cis-1.2.16".into(), // audit-log-path
        "cis-1.2.20".into(), // service-account-lookup
        "cis-1.2.25".into(), // encryption provider config
        "cis-3.2.1".into(),  // audit-policy-file
        "cis-3.2.2".into(),  // audit-policy file exists
        "C-0015".into(),     // RBAC secrets list
        "C-0035".into(),     // cluster-admin binding
        "C-0038".into(),     // host PID/IPC
        "C-0057".into(),     // privileged
        "C-0066".into(),     // etcd encryption
        "T1611".into(),      // Escape to Host
        "T1485".into(),      // Data Destruction
        "T1496".into(),      // Resource Hijacking
    ]
}

/// Look up a profile by id.
pub fn find_profile(id: &str) -> Result<Profile> {
    builtin_profiles()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| BenchError::ProfileNotFound(id.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_profiles_has_at_least_four() {
        assert!(builtin_profiles().len() >= 4);
    }

    #[test]
    fn test_builtin_profiles_have_unique_ids() {
        let mut ids: Vec<_> = builtin_profiles().into_iter().map(|p| p.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn test_find_profile_known() {
        assert!(find_profile("cis-1.10").is_ok());
        assert!(find_profile("nsa-2025").is_ok());
    }

    #[test]
    fn test_find_profile_unknown_returns_error() {
        assert!(find_profile("nonexistent").is_err());
    }

    #[test]
    fn test_cis_profile_has_many_checks() {
        let p = find_profile("cis-1.10").unwrap();
        assert!(p.check_ids.len() >= 40);
    }

    #[test]
    fn test_nsa_profile_has_nsa_framework() {
        let p = find_profile("nsa-2025").unwrap();
        assert_eq!(p.framework, Framework::NsaHardening);
    }

    #[test]
    fn test_soc2_subset_nonempty() {
        let p = find_profile("soc2-cc-7").unwrap();
        assert!(!p.check_ids.is_empty());
    }
}
