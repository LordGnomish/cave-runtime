// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/edge` view — edge node runtime status.
//!
//! Renders a live snapshot computed by the `cave-edge-runtime` crate itself
//! (KubeEdge + K3s edge-mode parity): the edged pod set with kubelet phases,
//! the device-twin delta, the local-autonomy connection state, and the
//! constrained-resource (256 MB) budget with the memory-pressure eviction
//! ranking. The numbers are produced by exercising the real ported logic, so
//! the page is a working demonstration of the subsystems, not a mock.

use cave_edge_runtime::autonomy::{ConnectionState, EdgeAutonomy};
use cave_edge_runtime::constrained::{ConstrainedMode, PodResource, ResourceBudget};
use cave_edge_runtime::devicetwin::{DeviceTwin, TwinVersion};
use cave_edge_runtime::edged::{ContainerState, ContainerStatus, Edged, Pod, PodWork, RestartPolicy};

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EdgeViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

fn demo_pod(name: &str, policy: RestartPolicy, states: &[ContainerState]) -> Pod {
    Pod {
        name: name.to_string(),
        namespace: "default".to_string(),
        uid: format!("uid-{name}"),
        restart_policy: policy,
        containers: states
            .iter()
            .enumerate()
            .map(|(i, s)| ContainerStatus {
                name: format!("c{i}"),
                state: s.clone(),
            })
            .collect(),
    }
}

/// Build the live snapshot by running the ported edge subsystems.
fn snapshot() -> (Edged, DeviceTwin, ConstrainedMode, EdgeAutonomy) {
    // edged: a couple of pods at different phases.
    let mut edged = Edged::new("edge-node-1");
    edged.dispatch(PodWork::Add(demo_pod(
        "web",
        RestartPolicy::Always,
        &[ContainerState::Running],
    )));
    edged.dispatch(PodWork::Add(demo_pod(
        "batch",
        RestartPolicy::Never,
        &[ContainerState::Terminated { exit_code: 0 }],
    )));
    edged.dispatch(PodWork::Add(demo_pod(
        "starting",
        RestartPolicy::Always,
        &[ContainerState::Waiting],
    )));

    // device twin: one attribute with a pending delta.
    let mut twin = DeviceTwin::new();
    twin.bind_device("thermostat-1");
    twin.update_actual("thermostat-1", "temp", "20");
    twin.update_expected("thermostat-1", "temp", "22", TwinVersion { cloud: 1, edge: 1 });

    // constrained mode: 256 MB node under load.
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 256,
        reserved_mb: 56,
    });
    cm.try_admit(&PodResource {
        name: "web".into(),
        request_mb: 64,
        limit_mb: 128,
        usage_mb: 110,
        priority: 100,
    });
    cm.try_admit(&PodResource {
        name: "batch".into(),
        request_mb: 32,
        limit_mb: 64,
        usage_mb: 20,
        priority: 10,
    });

    // autonomy: connected node.
    let autonomy = EdgeAutonomy::new(0);

    (edged, twin, cm, autonomy)
}

pub fn render(_state: &(), ctx: &RequestCtx) -> Result<String, EdgeViewError> {
    ctx.authorise(Permission::EdgeRead)?;
    let (edged, twin, cm, autonomy) = snapshot();

    // Pods table.
    let pod_rows: Vec<Vec<String>> = ["web", "batch", "starting"]
        .iter()
        .filter_map(|name| edged.phase_of(name).map(|p| (name, p)))
        .map(|(name, phase)| vec![name.to_string(), format!("{phase:?}")])
        .collect();

    // Device-twin delta table.
    let twin_rows: Vec<Vec<String>> = twin
        .delta("thermostat-1")
        .into_iter()
        .map(|d| {
            vec![
                "thermostat-1".to_string(),
                d.attr.clone(),
                twin.actual("thermostat-1", &d.attr).unwrap_or_default(),
                d.expected,
            ]
        })
        .collect();

    // Eviction ranking table.
    let evict_rows: Vec<Vec<String>> = cm
        .eviction_order()
        .into_iter()
        .map(|c| vec![c.name, c.usage_above_request_mb.to_string()])
        .collect();

    let conn = match autonomy.state() {
        ConnectionState::Connected => "Connected",
        ConnectionState::Disconnected => "Disconnected",
    };

    let body = format!(
        r#"<section class="space-y-6">
  <div>
    <h2 class="text-lg font-semibold mb-1">Edge node · {node}</h2>
    <p class="text-sm text-gray-500">cloud link: <strong>{conn}</strong> ·
       pods: {npods} · allocatable: {alloc} MB · used (req): {used} MB ·
       memory pressure (&lt;50 MB free): <strong>{pressure}</strong></p>
  </div>
  <div>
    <h3 class="text-md font-semibold mb-2">Pods (kubelet phase)</h3>{pods}
  </div>
  <div>
    <h3 class="text-md font-semibold mb-2">Device twin delta</h3>{twin}
  </div>
  <div>
    <h3 class="text-md font-semibold mb-2">Eviction ranking (memory pressure)</h3>{evict}
  </div>
</section>"#,
        node = escape(edged.node_name()),
        conn = conn,
        npods = edged.pod_count(),
        alloc = cm.budget().allocatable_mb(),
        used = cm.used_request_mb(),
        pressure = cm.under_memory_pressure(50),
        pods = table(&["pod", "phase"], &pod_rows),
        twin = table(&["device", "attr", "actual", "expected"], &twin_rows),
        evict = table(&["pod", "MB over request"], &evict_rows),
    );

    Ok(page_shell_full(
        ctx,
        "/admin/edge",
        &format!("edge · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("edge/pkg/edged/edged.go", "edged");

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_edge_read_permission() {
        let err = render(&(), &ctx(&[])).unwrap_err();
        assert!(matches!(err, EdgeViewError::Auth(_)));
    }

    #[test]
    fn render_shows_live_subsystem_snapshot() {
        let html = render(&(), &ctx(&[Permission::EdgeRead])).unwrap();
        // edged: phases computed by the real getPhase machine.
        assert!(html.contains("edge-node-1"));
        assert!(html.contains("Running")); // web
        assert!(html.contains("Succeeded")); // batch (Never, exit 0)
        assert!(html.contains("Pending")); // starting (Waiting)
        // device twin: the pending delta (expected 22 vs actual 20).
        assert!(html.contains("thermostat-1"));
        assert!(html.contains("22"));
        // autonomy + budget.
        assert!(html.contains("Connected"));
        assert!(html.contains("200")); // allocatable = 256 - 56
    }

    #[test]
    fn snapshot_reports_memory_pressure_under_load() {
        // web uses 110 + batch 20 = 130 of 200 → 70 free, not under 50.
        let (_e, _t, cm, _a) = snapshot();
        assert!(!cm.under_memory_pressure(50));
        assert!(cm.under_memory_pressure(80));
        // web exceeds its 64 MB request (110) → ranked first for eviction.
        assert_eq!(cm.eviction_order()[0].name, "web");
    }
}
