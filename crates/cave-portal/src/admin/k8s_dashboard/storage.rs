// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Storage tab — PersistentVolumes / PersistentVolumeClaims / StorageClasses.

use super::K8sDashboardViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageRow {
    pub kind: &'static str, // "PersistentVolume" | "PersistentVolumeClaim" | "StorageClass"
    pub name: String,
    pub namespace: String,
    pub size_gi: u32,
    pub status: &'static str,
}

pub fn list_storage(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StorageRow>, K8sDashboardViewError> {
    let workloads = super::workloads::list_workloads(state, ctx)?;
    let mut out: Vec<StorageRow> = Vec::new();
    // StorageClasses — cluster-wide, just 2.
    out.push(StorageRow {
        kind: "StorageClass",
        name: "fast-ssd".into(),
        namespace: "".into(),
        size_gi: 0,
        status: "Default",
    });
    out.push(StorageRow {
        kind: "StorageClass",
        name: "standard".into(),
        namespace: "".into(),
        size_gi: 0,
        status: "Available",
    });
    // PV + PVC per pod with a real workload.
    for w in &workloads {
        if w.pod_name.is_empty() {
            continue;
        }
        let bound = matches!(w.status, "Running" | "Pending");
        out.push(StorageRow {
            kind: "PersistentVolumeClaim",
            name: format!("{}-pvc", w.pod_name),
            namespace: "default".into(),
            size_gi: 10,
            status: if w.status == "Running" {
                "Bound"
            } else {
                "Pending"
            },
        });
        if bound {
            out.push(StorageRow {
                kind: "PersistentVolume",
                name: format!("pv-{}", w.pod_name),
                namespace: "".into(),
                size_gi: 10,
                status: "Bound",
            });
        }
    }
    Ok(out)
}

pub fn count_by_kind(rows: &[StorageRow], kind: &str) -> usize {
    rows.iter().filter(|r| r.kind == kind).count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, K8sDashboardViewError> {
    let rows = list_storage(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.kind.into(),
                r.name.clone(),
                r.namespace.clone(),
                if r.size_gi == 0 {
                    "—".into()
                } else {
                    format!("{}Gi", r.size_gi)
                },
                r.status.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="k8s-dashboard-storage" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Storage ({n})</h2>
  <div class="text-xs text-gray-500 mb-2">
    {pv} PV · {pvc} PVC · {sc} StorageClass
  </div>
  {tbl}
</section>"#,
        n = rows.len(),
        pv = count_by_kind(&rows, "PersistentVolume"),
        pvc = count_by_kind(&rows, "PersistentVolumeClaim"),
        sc = count_by_kind(&rows, "StorageClass"),
        tbl = table(
            &["kind", "name", "namespace", "size", "status"],
            &table_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_storage_includes_all_three_kinds() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Storage.tsx",
            "Storage",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_storage(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let kinds: std::collections::HashSet<_> = rows.iter().map(|r| r.kind).collect();
        for k in ["PersistentVolume", "PersistentVolumeClaim", "StorageClass"] {
            assert!(kinds.contains(k), "missing kind {k}");
        }
    }

    #[test]
    fn list_storage_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_storage(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn pv_pvc_counts_correlate() {
        let s = AdminState::seeded();
        let rows = list_storage(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        let pv = count_by_kind(&rows, "PersistentVolume");
        let pvc = count_by_kind(&rows, "PersistentVolumeClaim");
        // PV ≤ PVC (only Running pods get a bound PV).
        assert!(pv <= pvc);
    }

    #[test]
    fn render_section_emits_kind_legend() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::K8sDashboardRead])).unwrap();
        assert!(html.contains("PV"));
        assert!(html.contains("PVC"));
        assert!(html.contains("StorageClass"));
    }
}
