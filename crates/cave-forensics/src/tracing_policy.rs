// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TracingPolicy + TracingPolicyNamespaced CRD types and spec parser.
//!
//! Upstream: `pkg/tracingpolicy/types.go`, `pkg/tracingpolicy/policy.go`,
//! `pkg/k8s/apis/cilium.io/v1alpha1/tracing_policy_types.go`.
//!
//! Scope: the data model that a cluster operator authors. Selectors,
//! filters, and enforcement actions are defined in companion modules
//! (`selectors.rs`, `filter.rs`, `enforcer.rs`).

use crate::error::{ForensicsError, Result};
use serde::{Deserialize, Serialize};

/// Cluster-scoped or namespace-scoped tracing policy. Mirrors
/// `cilium.io/v1alpha1::TracingPolicy{,Namespaced}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TracingPolicy {
    pub api_version: String,
    pub kind: PolicyKind,
    pub metadata: PolicyMeta,
    pub spec: TracingPolicySpec,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum PolicyKind {
    /// Cluster-scoped — applies to all namespaces.
    TracingPolicy,
    /// Namespace-scoped — `metadata.namespace` selects scope.
    TracingPolicyNamespaced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PolicyMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: std::collections::BTreeMap<String, String>,
}

/// `TracingPolicySpec` mirrors Tetragon's spec block: kprobes, tracepoints,
/// uprobes, LSM hooks, and a pod/container selector.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TracingPolicySpec {
    #[serde(default)]
    pub kprobes: Vec<KProbeSpec>,
    #[serde(default)]
    pub uprobes: Vec<UProbeSpec>,
    #[serde(default)]
    pub tracepoints: Vec<TracepointSpec>,
    #[serde(default)]
    pub lsm_hooks: Vec<LsmHookSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pod_selector: Option<crate::selectors::PodSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_selector: Option<crate::selectors::ContainerSelector>,
    /// Policy-mode is one of `Enforce` or `Monitor`. Defaults to `Enforce`.
    #[serde(default)]
    pub mode: PolicyMode,
    #[serde(default)]
    pub options: Vec<KeyValue>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "PascalCase")]
pub enum PolicyMode {
    #[default]
    Enforce,
    Monitor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KProbeSpec {
    pub call: String,
    #[serde(default)]
    pub syscall: bool,
    #[serde(default)]
    pub return_: bool,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
    #[serde(default)]
    pub selectors: Vec<crate::filter::FilterGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UProbeSpec {
    pub path: String,
    pub symbols: Vec<String>,
    #[serde(default)]
    pub selectors: Vec<crate::filter::FilterGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TracepointSpec {
    pub subsystem: String,
    pub event: String,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
    #[serde(default)]
    pub selectors: Vec<crate::filter::FilterGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LsmHookSpec {
    pub hook: String,
    #[serde(default)]
    pub args: Vec<ArgSpec>,
    #[serde(default)]
    pub selectors: Vec<crate::filter::FilterGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArgSpec {
    pub index: u32,
    pub r#type: ArgType,
    #[serde(default)]
    pub size_arg_index: Option<u32>,
    #[serde(default)]
    pub return_copy: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArgType {
    Int,
    Uint64,
    #[serde(rename = "size_t")]
    SizeT,
    String,
    CharBuf,
    Path,
    Fd,
    File,
    Skb,
    Sock,
    NopReturn,
    CredEffective,
    LinuxBinprm,
    LoadInfo,
    KernelModule,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyValue {
    pub name: String,
    pub value: String,
}

impl TracingPolicy {
    /// Parse a YAML or JSON document into a `TracingPolicy`. The Go upstream
    /// supports YAML; we accept JSON natively (k8s YAML→JSON is upstream of
    /// us — out of scope here, see `cave-k8s` skipped subsystem).
    pub fn parse_json(text: &str) -> Result<Self> {
        let p: Self = serde_json::from_str(text)?;
        p.validate()?;
        Ok(p)
    }

    /// Validate structural invariants. Tetragon enforces these at admission
    /// time and we run the same checks before installing the policy into
    /// the [`PolicyStore`](crate::store::PolicyStore).
    pub fn validate(&self) -> Result<()> {
        if self.metadata.name.is_empty() {
            return Err(ForensicsError::InvalidPolicy(
                "metadata.name must not be empty".into(),
            ));
        }
        match self.kind {
            PolicyKind::TracingPolicy => {
                if self.metadata.namespace.is_some() {
                    return Err(ForensicsError::InvalidPolicy(
                        "cluster-scoped TracingPolicy must not set metadata.namespace".into(),
                    ));
                }
            }
            PolicyKind::TracingPolicyNamespaced => {
                if self.metadata.namespace.as_deref().unwrap_or("").is_empty() {
                    return Err(ForensicsError::InvalidPolicy(
                        "TracingPolicyNamespaced requires metadata.namespace".into(),
                    ));
                }
            }
        }
        if self.spec.kprobes.is_empty()
            && self.spec.uprobes.is_empty()
            && self.spec.tracepoints.is_empty()
            && self.spec.lsm_hooks.is_empty()
        {
            return Err(ForensicsError::InvalidPolicy(
                "spec must define at least one of kprobes / uprobes / tracepoints / lsm_hooks"
                    .into(),
            ));
        }
        for kp in &self.spec.kprobes {
            if kp.call.is_empty() {
                return Err(ForensicsError::InvalidPolicy(
                    "kprobe.call must not be empty".into(),
                ));
            }
        }
        Ok(())
    }

    /// True if the policy is in monitor-only mode (no enforcement actions
    /// are executed; events are still produced).
    pub fn is_monitor(&self) -> bool {
        matches!(self.spec.mode, PolicyMode::Monitor)
    }

    /// True if this policy is scoped to a single namespace.
    pub fn is_namespaced(&self) -> bool {
        matches!(self.kind, PolicyKind::TracingPolicyNamespaced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn min_spec() -> TracingPolicySpec {
        TracingPolicySpec {
            kprobes: vec![KProbeSpec {
                call: "sys_open".into(),
                syscall: true,
                return_: false,
                args: vec![],
                selectors: vec![],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_minimal_cluster_policy_validates() {
        let p = TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: PolicyKind::TracingPolicy,
            metadata: PolicyMeta {
                name: "policy-1".into(),
                ..Default::default()
            },
            spec: min_spec(),
        };
        assert!(p.validate().is_ok());
        assert!(!p.is_namespaced());
        assert!(!p.is_monitor());
    }

    #[test]
    fn test_namespaced_requires_namespace() {
        let mut p = TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: PolicyKind::TracingPolicyNamespaced,
            metadata: PolicyMeta {
                name: "ns-policy".into(),
                ..Default::default()
            },
            spec: min_spec(),
        };
        assert!(p.validate().is_err());
        p.metadata.namespace = Some("kube-system".into());
        assert!(p.validate().is_ok());
        assert!(p.is_namespaced());
    }

    #[test]
    fn test_cluster_policy_rejects_namespace() {
        let p = TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: PolicyKind::TracingPolicy,
            metadata: PolicyMeta {
                name: "p".into(),
                namespace: Some("default".into()),
                ..Default::default()
            },
            spec: min_spec(),
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_empty_name_rejected() {
        let p = TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: PolicyKind::TracingPolicy,
            metadata: PolicyMeta::default(),
            spec: min_spec(),
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_spec_must_have_hooks() {
        let p = TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: PolicyKind::TracingPolicy,
            metadata: PolicyMeta {
                name: "p".into(),
                ..Default::default()
            },
            spec: TracingPolicySpec::default(),
        };
        let err = p.validate().unwrap_err();
        assert!(format!("{err}").contains("kprobes"));
    }

    #[test]
    fn test_empty_kprobe_call_rejected() {
        let mut s = min_spec();
        s.kprobes[0].call = String::new();
        let p = TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: PolicyKind::TracingPolicy,
            metadata: PolicyMeta {
                name: "p".into(),
                ..Default::default()
            },
            spec: s,
        };
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_parse_json_roundtrip() {
        let p = TracingPolicy {
            api_version: "cilium.io/v1alpha1".into(),
            kind: PolicyKind::TracingPolicy,
            metadata: PolicyMeta {
                name: "json-p".into(),
                ..Default::default()
            },
            spec: min_spec(),
        };
        let j = serde_json::to_string(&p).unwrap();
        let back = TracingPolicy::parse_json(&j).unwrap();
        assert_eq!(back.metadata.name, "json-p");
    }

    #[test]
    fn test_parse_json_validates_after_parse() {
        let bad = r#"{
            "api_version":"cilium.io/v1alpha1",
            "kind":"TracingPolicy",
            "metadata":{"name":""},
            "spec":{}
        }"#;
        assert!(TracingPolicy::parse_json(bad).is_err());
    }

    #[test]
    fn test_arg_type_snake_case_serde() {
        let v = ArgType::CharBuf;
        let j = serde_json::to_string(&v).unwrap();
        assert_eq!(j, "\"char_buf\"");
    }

    #[test]
    fn test_mode_enforce_default() {
        let s = TracingPolicySpec::default();
        assert!(matches!(s.mode, PolicyMode::Enforce));
    }

    #[test]
    fn test_uprobe_spec_serde() {
        let u = UProbeSpec {
            path: "/usr/lib/libssl.so.3".into(),
            symbols: vec!["SSL_read".into(), "SSL_write".into()],
            selectors: vec![],
        };
        let j = serde_json::to_string(&u).unwrap();
        let back: UProbeSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(back.symbols.len(), 2);
    }

    #[test]
    fn test_tracepoint_spec_serde() {
        let t = TracepointSpec {
            subsystem: "syscalls".into(),
            event: "sys_enter_openat".into(),
            args: vec![],
            selectors: vec![],
        };
        let j = serde_json::to_string(&t).unwrap();
        let back: TracepointSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(back.event, "sys_enter_openat");
    }

    #[test]
    fn test_lsm_hook_spec_serde() {
        let l = LsmHookSpec {
            hook: "file_open".into(),
            args: vec![],
            selectors: vec![],
        };
        let j = serde_json::to_string(&l).unwrap();
        let back: LsmHookSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(back.hook, "file_open");
    }
}
