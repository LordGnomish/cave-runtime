//! KubeletConfiguration — the full kubelet config object that the kubelet
//! reads on startup.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `staging/src/k8s.io/kubelet/config/v1beta1/types.go`
//!     (`KubeletConfiguration` struct + Defaults).
//!   `pkg/kubelet/apis/config/validation/validation.go`
//!     (cross-field validation rules: GC thresholds, eviction signals, …).
//!
//! This module models a representative-but-bounded subset that tenants
//! reference when overriding policy: image GC thresholds, eviction
//! signals, manager policies, sync frequencies, and feature gates.
//! Validation reproduces upstream cross-field invariants.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("imageGCHighThresholdPercent ({high}) must be > imageGCLowThresholdPercent ({low})")]
    ImageGcThresholdInverted { high: u8, low: u8 },
    #[error("imageGC*ThresholdPercent must be ≤ 100, got {0}")]
    ImageGcOverHundred(u8),
    #[error("podsPerCore must be ≥ 0, got negative")]
    PodsPerCoreNegative,
    #[error("syncFrequency must be > 0")]
    SyncFrequencyZero,
    #[error("nodeStatusUpdateFrequency must be > 0")]
    NodeStatusUpdateFrequencyZero,
    #[error("eviction signal '{0}' missing threshold")]
    EvictionThresholdMissing(String),
    #[error("eviction signal '{signal}' duplicate threshold")]
    EvictionThresholdDuplicate { signal: String },
    #[error("cpuManagerPolicy '{0}' invalid (none|static)")]
    CpuManagerPolicyInvalid(String),
    #[error("topologyManagerPolicy '{0}' invalid")]
    TopologyManagerPolicyInvalid(String),
    #[error("registry credential entry must have non-empty server")]
    RegistryCredentialEmptyServer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuManagerPolicy {
    None,
    Static,
}

impl CpuManagerPolicy {
    pub fn parse(s: &str) -> Result<Self, ConfigError> {
        match s {
            "none" | "None" => Ok(Self::None),
            "static" | "Static" => Ok(Self::Static),
            other => Err(ConfigError::CpuManagerPolicyInvalid(other.into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopologyManagerPolicyName {
    None,
    BestEffort,
    Restricted,
    SingleNumaNode,
}

impl TopologyManagerPolicyName {
    pub fn parse(s: &str) -> Result<Self, ConfigError> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "best-effort" | "besteffort" => Ok(Self::BestEffort),
            "restricted" => Ok(Self::Restricted),
            "single-numa-node" | "singlenumanode" => Ok(Self::SingleNumaNode),
            _ => Err(ConfigError::TopologyManagerPolicyInvalid(s.into())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvictionSignal {
    pub signal: String,
    pub threshold: String,
    pub grace_period_secs: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryCredential {
    pub server: String,
    pub username: String,
    pub password_ref: String,
}

/// KubeletConfiguration. `tenant_id` scopes overrides to a tenant's nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubeletConfiguration {
    pub tenant_id: String,
    pub sync_frequency_secs: u32,
    pub node_status_update_frequency_secs: u32,
    pub image_gc_high_threshold_percent: u8,
    pub image_gc_low_threshold_percent: u8,
    pub max_pods: u32,
    pub pods_per_core: u32,
    pub cpu_manager_policy: CpuManagerPolicy,
    pub topology_manager_policy: TopologyManagerPolicyName,
    pub eviction_hard: Vec<EvictionSignal>,
    pub feature_gates: BTreeMap<String, bool>,
    pub registry_credentials: Vec<RegistryCredential>,
}

impl KubeletConfiguration {
    /// Upstream defaults (mirrors `pkg/kubelet/apis/config/v1beta1/defaults.go`).
    pub fn defaults(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            sync_frequency_secs: 60,
            node_status_update_frequency_secs: 10,
            image_gc_high_threshold_percent: 85,
            image_gc_low_threshold_percent: 80,
            max_pods: 110,
            pods_per_core: 0,
            cpu_manager_policy: CpuManagerPolicy::None,
            topology_manager_policy: TopologyManagerPolicyName::None,
            eviction_hard: vec![
                EvictionSignal {
                    signal: "memory.available".into(),
                    threshold: "100Mi".into(),
                    grace_period_secs: 0,
                },
                EvictionSignal {
                    signal: "nodefs.available".into(),
                    threshold: "10%".into(),
                    grace_period_secs: 0,
                },
                EvictionSignal {
                    signal: "imagefs.available".into(),
                    threshold: "15%".into(),
                    grace_period_secs: 0,
                },
            ],
            feature_gates: BTreeMap::new(),
            registry_credentials: vec![],
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.image_gc_high_threshold_percent > 100 {
            return Err(ConfigError::ImageGcOverHundred(
                self.image_gc_high_threshold_percent,
            ));
        }
        if self.image_gc_low_threshold_percent > 100 {
            return Err(ConfigError::ImageGcOverHundred(
                self.image_gc_low_threshold_percent,
            ));
        }
        if self.image_gc_high_threshold_percent <= self.image_gc_low_threshold_percent {
            return Err(ConfigError::ImageGcThresholdInverted {
                high: self.image_gc_high_threshold_percent,
                low: self.image_gc_low_threshold_percent,
            });
        }
        if self.sync_frequency_secs == 0 {
            return Err(ConfigError::SyncFrequencyZero);
        }
        if self.node_status_update_frequency_secs == 0 {
            return Err(ConfigError::NodeStatusUpdateFrequencyZero);
        }
        let mut seen = std::collections::HashSet::new();
        for sig in &self.eviction_hard {
            if sig.threshold.is_empty() {
                return Err(ConfigError::EvictionThresholdMissing(sig.signal.clone()));
            }
            if !seen.insert(sig.signal.clone()) {
                return Err(ConfigError::EvictionThresholdDuplicate {
                    signal: sig.signal.clone(),
                });
            }
        }
        for cred in &self.registry_credentials {
            if cred.server.is_empty() {
                return Err(ConfigError::RegistryCredentialEmptyServer);
            }
        }
        Ok(())
    }

    /// Compute effective `max_pods` taking pods_per_core into account.
    pub fn effective_max_pods(&self, cpu_cores: u32) -> u32 {
        if self.pods_per_core == 0 {
            return self.max_pods;
        }
        let by_cores = self.pods_per_core.saturating_mul(cpu_cores);
        if by_cores == 0 {
            self.max_pods
        } else {
            self.max_pods.min(by_cores)
        }
    }

    /// Toggle feature gate; returns previous value (defaults to false).
    pub fn set_feature_gate(&mut self, name: &str, enabled: bool) -> bool {
        self.feature_gates
            .insert(name.into(), enabled)
            .unwrap_or(false)
    }

    pub fn feature_enabled(&self, name: &str) -> bool {
        *self.feature_gates.get(name).unwrap_or(&false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_clean() {
        let c = KubeletConfiguration::defaults("acme");
        c.validate().unwrap();
    }

    #[test]
    fn image_gc_threshold_inverted_rejected() {
        let mut c = KubeletConfiguration::defaults("acme");
        c.image_gc_low_threshold_percent = 90;
        c.image_gc_high_threshold_percent = 80;
        assert!(matches!(
            c.validate(),
            Err(ConfigError::ImageGcThresholdInverted { .. })
        ));
    }

    #[test]
    fn image_gc_over_hundred_rejected() {
        let mut c = KubeletConfiguration::defaults("acme");
        c.image_gc_high_threshold_percent = 200;
        assert!(matches!(c.validate(), Err(ConfigError::ImageGcOverHundred(200))));
    }

    #[test]
    fn sync_frequency_zero_rejected() {
        let mut c = KubeletConfiguration::defaults("acme");
        c.sync_frequency_secs = 0;
        assert!(matches!(c.validate(), Err(ConfigError::SyncFrequencyZero)));
    }

    #[test]
    fn duplicate_eviction_signal_rejected() {
        let mut c = KubeletConfiguration::defaults("acme");
        c.eviction_hard.push(EvictionSignal {
            signal: "memory.available".into(),
            threshold: "200Mi".into(),
            grace_period_secs: 0,
        });
        assert!(matches!(
            c.validate(),
            Err(ConfigError::EvictionThresholdDuplicate { .. })
        ));
    }

    #[test]
    fn missing_eviction_threshold_rejected() {
        let mut c = KubeletConfiguration::defaults("acme");
        c.eviction_hard[0].threshold = "".into();
        assert!(matches!(
            c.validate(),
            Err(ConfigError::EvictionThresholdMissing(_))
        ));
    }

    #[test]
    fn cpu_manager_policy_parse() {
        assert_eq!(
            CpuManagerPolicy::parse("static").unwrap(),
            CpuManagerPolicy::Static
        );
        assert_eq!(
            CpuManagerPolicy::parse("none").unwrap(),
            CpuManagerPolicy::None
        );
        assert!(CpuManagerPolicy::parse("dynamic").is_err());
    }

    #[test]
    fn topology_policy_parse_aliases() {
        assert_eq!(
            TopologyManagerPolicyName::parse("best-effort").unwrap(),
            TopologyManagerPolicyName::BestEffort
        );
        assert_eq!(
            TopologyManagerPolicyName::parse("BestEffort").unwrap(),
            TopologyManagerPolicyName::BestEffort
        );
        assert_eq!(
            TopologyManagerPolicyName::parse("single-numa-node").unwrap(),
            TopologyManagerPolicyName::SingleNumaNode
        );
        assert!(TopologyManagerPolicyName::parse("madeup").is_err());
    }

    #[test]
    fn effective_max_pods_caps_by_cores() {
        let mut c = KubeletConfiguration::defaults("acme");
        c.pods_per_core = 10;
        assert_eq!(c.effective_max_pods(8), 80);
        // pods_per_core×cores > max_pods → clamp to max_pods
        c.pods_per_core = 50;
        assert_eq!(c.effective_max_pods(8), 110);
    }

    #[test]
    fn effective_max_pods_zero_pods_per_core_uses_max() {
        let c = KubeletConfiguration::defaults("acme");
        assert_eq!(c.effective_max_pods(8), 110);
    }

    #[test]
    fn feature_gate_round_trip() {
        let mut c = KubeletConfiguration::defaults("acme");
        assert!(!c.feature_enabled("DynamicResourceAllocation"));
        let prev = c.set_feature_gate("DynamicResourceAllocation", true);
        assert!(!prev);
        assert!(c.feature_enabled("DynamicResourceAllocation"));
    }

    #[test]
    fn registry_credential_empty_server_rejected() {
        let mut c = KubeletConfiguration::defaults("acme");
        c.registry_credentials.push(RegistryCredential {
            server: "".into(),
            username: "u".into(),
            password_ref: "secret/x".into(),
        });
        assert!(matches!(
            c.validate(),
            Err(ConfigError::RegistryCredentialEmptyServer)
        ));
    }

    #[test]
    fn tenant_id_threaded_through() {
        let c = KubeletConfiguration::defaults("acme");
        assert_eq!(c.tenant_id, "acme");
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"tenant_id\":\"acme\""));
    }
}
