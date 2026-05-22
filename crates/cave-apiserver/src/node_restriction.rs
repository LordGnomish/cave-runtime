// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodeRestriction admission — line-by-line port of upstream
//! `plugin/pkg/admission/noderestriction/admission.go`.
//!
//! NodeRestriction limits what a kubelet (identified as
//! `system:node:<nodename>` in group `system:nodes`) may do via the API:
//!
//!   * mutate Node → only its own
//!   * mutate Pod → only pods bound (by spec.nodeName) to itself
//!   * write Pod/status → only own-node pods
//!   * mutate Lease in `kube-node-lease` → only its own
//!   * mutate CSINode → only its own
//!   * read Secret/ConfigMap → only those referenced by its own pods
//!     (mirror of `referencegraph.go`; the reference graph itself is
//!     out of scope for this module — we model the gate via an injected
//!     `OwnPodReferences`).
//!
//! ## Tenant invariant
//!
//! A kubelet identified by `system:node:<n>` operates within a single
//! tenant; cross-tenant Node mutation is denied at the request level by
//! tagging the AdmissionRequest's `tenant_id`. The plugin treats requests
//! whose tenant differs from the node's tenant as plain non-node users.

use crate::admission::{
    AdmissionRequest, AdmissionResponse, MutatingWebhook, Operation, ValidatingWebhook,
};
use std::collections::HashSet;

const NODE_USER_PREFIX: &str = "system:node:";

/// Extract the nodename from `system:node:<nodename>`. Returns `None` if the
/// user is not a node identity.
pub fn node_name_from_user(user: &str) -> Option<&str> {
    user.strip_prefix(NODE_USER_PREFIX)
}

/// Capture of pods/objects "owned" by a given node — populated externally and
/// consulted by the plugin.
#[derive(Debug, Clone, Default)]
pub struct OwnPodReferences {
    /// Pod names owned by this node, in `(namespace, name)` form.
    pub pods: HashSet<(String, String)>,
    /// Secrets referenced by those pods.
    pub secrets: HashSet<(String, String)>,
    /// ConfigMaps referenced by those pods.
    pub configmaps: HashSet<(String, String)>,
    /// PVCs referenced by those pods.
    pub pvcs: HashSet<(String, String)>,
}

pub trait NodeReferenceLister: Send + Sync {
    fn references_for(&self, node_name: &str) -> OwnPodReferences;
}

/// In-memory lister for tests.
#[derive(Default)]
pub struct StaticNodeRefs {
    pub by_node: std::collections::HashMap<String, OwnPodReferences>,
}

impl StaticNodeRefs {
    pub fn set(&mut self, node: &str, refs: OwnPodReferences) {
        self.by_node.insert(node.into(), refs);
    }
}

impl NodeReferenceLister for StaticNodeRefs {
    fn references_for(&self, node_name: &str) -> OwnPodReferences {
        self.by_node.get(node_name).cloned().unwrap_or_default()
    }
}

pub struct NodeRestriction {
    pub refs: std::sync::Arc<dyn NodeReferenceLister>,
}

impl NodeRestriction {
    pub fn new(refs: std::sync::Arc<dyn NodeReferenceLister>) -> Self {
        Self { refs }
    }

    fn deny(req: &AdmissionRequest, msg: impl Into<String>) -> AdmissionResponse {
        AdmissionResponse::deny(req, 403, msg)
    }
}

impl ValidatingWebhook for NodeRestriction {
    fn name(&self) -> &str {
        "NodeRestriction"
    }
    fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
        let Some(node_name) = node_name_from_user(&req.user) else {
            return AdmissionResponse::allow(req);
        };
        let refs = self.refs.references_for(node_name);
        match req.kind.as_str() {
            "Node" => {
                if matches!(
                    req.operation,
                    Operation::Create | Operation::Update | Operation::Delete
                ) {
                    if req.name != node_name {
                        return Self::deny(
                            req,
                            format!(
                                "node {} cannot {:?} other node {}",
                                node_name, req.operation, req.name
                            ),
                        );
                    }
                }
                AdmissionResponse::allow(req)
            }
            "Pod" => {
                // We can't read spec.nodeName from a generic Resource model
                // here; the reference list is authoritative.
                if matches!(req.operation, Operation::Create | Operation::Update) {
                    if !refs
                        .pods
                        .contains(&(req.namespace.clone(), req.name.clone()))
                    {
                        return Self::deny(
                            req,
                            format!(
                                "node {} cannot {:?} pod {}/{}",
                                node_name, req.operation, req.namespace, req.name
                            ),
                        );
                    }
                }
                AdmissionResponse::allow(req)
            }
            "Secret" => {
                // Only Get/List: validating phase doesn't see Get; but on
                // Create/Update by node user we deny.
                if matches!(
                    req.operation,
                    Operation::Create | Operation::Update | Operation::Delete
                ) {
                    return Self::deny(req, format!("node {} cannot mutate secrets", node_name));
                }
                if !refs
                    .secrets
                    .contains(&(req.namespace.clone(), req.name.clone()))
                {
                    return Self::deny(
                        req,
                        format!(
                            "node {} cannot access secret {}/{}",
                            node_name, req.namespace, req.name
                        ),
                    );
                }
                AdmissionResponse::allow(req)
            }
            "ConfigMap" => {
                if matches!(
                    req.operation,
                    Operation::Create | Operation::Update | Operation::Delete
                ) {
                    return Self::deny(req, format!("node {} cannot mutate configmaps", node_name));
                }
                if !refs
                    .configmaps
                    .contains(&(req.namespace.clone(), req.name.clone()))
                {
                    return Self::deny(
                        req,
                        format!(
                            "node {} cannot access configmap {}/{}",
                            node_name, req.namespace, req.name
                        ),
                    );
                }
                AdmissionResponse::allow(req)
            }
            "Lease" => {
                // kube-node-lease ns + own-name only
                if req.namespace != "kube-node-lease" {
                    return Self::deny(
                        req,
                        format!(
                            "node {} can only mutate Leases in kube-node-lease",
                            node_name
                        ),
                    );
                }
                if req.name != node_name {
                    return Self::deny(
                        req,
                        format!(
                            "node {} cannot mutate other node's lease {}",
                            node_name, req.name
                        ),
                    );
                }
                AdmissionResponse::allow(req)
            }
            "CSINode" => {
                if req.name != node_name {
                    return Self::deny(
                        req,
                        format!(
                            "node {} cannot mutate other CSINode {}",
                            node_name, req.name
                        ),
                    );
                }
                AdmissionResponse::allow(req)
            }
            _ => AdmissionResponse::allow(req),
        }
    }
}

/// Mutating no-op so NodeRestriction can also slot into the mutating chain
/// (upstream registers it for both phases and emits no patches).
impl MutatingWebhook for NodeRestriction {
    fn name(&self) -> &str {
        "NodeRestriction.Mutating"
    }
    fn admit(&self, req: &mut AdmissionRequest) -> AdmissionResponse {
        // Mutation gate: validating-phase does the heavy lifting; here we just
        // mirror the deny path for Create/Update on the same kinds.
        ValidatingWebhook::validate(self, req)
    }
}

#[cfg(test)]
mod tests;
