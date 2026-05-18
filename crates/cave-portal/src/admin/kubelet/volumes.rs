// SPDX-License-Identifier: AGPL-3.0-or-later
//! Volumes tab — pod ↔ PVC bindings derived from the seeded pod set.
//!
//! Mirrors the upstream Kubernetes Dashboard's Volumes column on the
//! per-pod drawer. Today the derivation is shape-only (one synthetic
//! PVC per pod whose name follows the `<pod>-pvc` convention) — a real
//! cluster client would resolve via apiserver `/api/v1/persistentvolumeclaims`.

use super::KubeletViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeBinding {
    pub pod: String,
    pub claim_name: String,
    pub storage_class: String,
    pub size_gi: u32,
    pub access_mode: &'static str, // "RWO" | "RWX" | "ROX" | "RWOP"
    pub status: &'static str,      // "Bound" | "Pending" | "Lost"
}

pub fn list_volumes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VolumeBinding>, KubeletViewError> {
    let pods = super::pods::list_pods(state, ctx)?;
    Ok(pods
        .iter()
        .map(|p| VolumeBinding {
            pod: p.pod_name.clone(),
            claim_name: format!("{}-pvc", p.pod_name),
            storage_class: "fast-ssd".into(),
            size_gi: 10,
            access_mode: "RWO",
            status: if p.status == "Running" {
                "Bound"
            } else if p.status == "Pending" {
                "Pending"
            } else {
                "Lost"
            },
        })
        .collect())
}

pub fn bound_count(volumes: &[VolumeBinding]) -> usize {
    volumes.iter().filter(|v| v.status == "Bound").count()
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KubeletViewError> {
    let volumes = list_volumes(state, ctx)?;
    let rows: Vec<Vec<String>> = volumes
        .iter()
        .map(|v| {
            vec![
                v.pod.clone(),
                v.claim_name.clone(),
                v.storage_class.clone(),
                format!("{}Gi", v.size_gi),
                v.access_mode.into(),
                v.status.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kubelet-volumes" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Volumes ({n}, {bound} Bound)</h2>
  <p class="text-xs text-gray-500 mb-2">Synthetic PVC binding derived from pod status — a live cluster resolves these via apiserver <code>/api/v1/persistentvolumeclaims</code>.</p>
  {tbl}
</section>"#,
        n = volumes.len(),
        bound = bound_count(&volumes),
        tbl = table(
            &["pod", "claim", "storageClass", "size", "access", "status"],
            &rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_volumes_emits_one_per_pod() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Volumes/VolumeList.tsx",
            "VolumeList",
            "acme"
        );
        let s = AdminState::seeded();
        let volumes = list_volumes(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert_eq!(volumes.len(), 3, "acme has 3 pods");
        assert!(volumes.iter().all(|v| v.claim_name.ends_with("-pvc")));
    }

    #[test]
    fn list_volumes_status_mirrors_pod_status() {
        let s = AdminState::seeded();
        let volumes = list_volumes(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        // Every Bound row corresponds to a Running pod.
        let bound: Vec<_> = volumes.iter().filter(|v| v.status == "Bound").collect();
        let pods = super::super::pods::list_pods(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        let running_pod_count = pods.iter().filter(|p| p.status == "Running").count();
        assert_eq!(bound.len(), running_pod_count);
    }

    #[test]
    fn list_volumes_requires_kubelet_read() {
        let s = AdminState::seeded();
        assert!(list_volumes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_columns_match_dashboard() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        for col in ["pod", "claim", "storageClass", "size", "access", "status"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
        // Operator-readable caveat about synthetic PVC.
        assert!(html.contains("Synthetic PVC"));
    }
}
