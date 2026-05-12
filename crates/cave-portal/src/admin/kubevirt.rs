//! `/admin/kubevirt` — KubeVirt VM lifecycle parity. Mirrors the
//! upstream UI's VirtualMachine list with phase-coloured cards and a
//! per-phase summary. The cave-side `VirtualMachine` struct doubles
//! as our `VirtualMachineInstance` view since the seed only carries
//! the merged shape.
//!
//! Upstream UI: <https://kubevirt.io/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, VirtualMachine};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KubevirtViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VirtualMachine>, KubevirtViewError> {
    ctx.authorise(Permission::KubevirtRead)?;
    let mut rows: Vec<VirtualMachine> = scope(
        &state.virtual_machines.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

/// Per-phase summary card values. KubeVirt's phases: `Pending`,
/// `Scheduling`, `Scheduled`, `Running`, `Succeeded`, `Failed`,
/// `Unknown`. We surface a subset relevant to the seeded data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VmSummary {
    pub total: u32,
    pub running: u32,
    pub pending: u32,
    pub failed: u32,
    pub total_cpu: u32,
    pub total_mem_mib: u64,
}

pub fn vm_summary(rows: &[VirtualMachine]) -> VmSummary {
    let mut s = VmSummary {
        total: rows.len() as u32,
        ..Default::default()
    };
    for v in rows {
        match v.phase {
            "Running" => s.running += 1,
            "Pending" => s.pending += 1,
            "Failed" => s.failed += 1,
            _ => {}
        }
        s.total_cpu += v.cpu;
        s.total_mem_mib += v.memory_mib;
    }
    s
}

/// Filter VMs by phase. Mirrors the upstream UI's phase-pill filter.
pub fn by_phase<'a>(rows: &'a [VirtualMachine], phase: &str) -> Vec<&'a VirtualMachine> {
    rows.iter().filter(|v| v.phase == phase).collect()
}

/// Look up one VM by name.
pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<Option<VirtualMachine>, KubevirtViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|v| v.name == name))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KubevirtViewError> {
    let rows = list_records(state, ctx)?;
    let summary = vm_summary(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                r.phase.into(),
                r.cpu.to_string(),
                r.memory_mib.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    KubeVirt VM lifecycle (cave-kubevirt).
    Upstream: <a class="text-blue-700 underline" href="https://kubevirt.io/">kubevirt.io</a>.
  </p>
  <div class="mb-4 grid grid-cols-5 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL</div><div class="text-2xl font-bold">{total}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">RUNNING</div><div class="text-2xl font-bold text-green-700">{running}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">PENDING</div><div class="text-2xl font-bold text-yellow-700">{pending}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">FAILED</div><div class="text-2xl font-bold text-red-700">{failed}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">CPU / RAM</div><div class="text-base font-semibold">{cpu} / {mem}M</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Virtual machines ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total = summary.total,
        running = summary.running,
        pending = summary.pending,
        failed = summary.failed,
        cpu = summary.total_cpu,
        mem = summary.total_mem_mib,
        tbl = table(&["name", "phase", "cpu", "memory_mib"], &table_rows),
    );
    Ok(page_shell(
        &format!("kubevirt · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/kubevirt/src/components/VmsList.tsx", "VmsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubevirt/src/components/VmsList.tsx",
            "VmsList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::KubevirtRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubevirt/src/components/VmsList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        assert!(html.contains("vm-1"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubevirt/src/components/VmsList.tsx",
            "RenderEvil",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        assert!(!html.contains("evil-vm"));
    }

    #[test]
    fn vm_summary_aggregates_phase_counts_and_resources() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        let s = vm_summary(&rows);
        assert_eq!(s.total, rows.len() as u32);
        assert!(s.total_cpu > 0);
        assert!(s.total_mem_mib > 0);
    }

    #[test]
    fn by_phase_filters_correctly() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        let running = by_phase(&rows, "Running");
        assert!(running.iter().all(|v| v.phase == "Running"));
        let zombie = by_phase(&rows, "Zombie");
        assert!(zombie.is_empty());
    }

    #[test]
    fn detail_returns_vm_by_name() {
        let s = AdminState::seeded();
        let rows = list_records(&s, &ctx(&[Permission::KubevirtRead])).unwrap();
        if let Some(first) = rows.first() {
            let name = first.name.clone();
            let d = detail(&s, &ctx(&[Permission::KubevirtRead]), &name).unwrap();
            assert!(d.is_some());
        }
        let missing = detail(&s, &ctx(&[Permission::KubevirtRead]), "no-such-vm").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn render_includes_summary_cards_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        assert!(html.contains("TOTAL"));
        assert!(html.contains("RUNNING"));
        assert!(html.contains("kubevirt.io"));
    }
}
