// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Datapath tab — Cilium eBPF userspace-simulation surfaces.
//!
//! cave-net mirrors Cilium's BPF hot path (`bpf/lib/*`) in a
//! deterministic userspace simulator (`ebpf_sim`). This tab catalogs
//! the load-balancer / DSR datapath probes exposed over `/api/net/*`
//! so an operator can replay the observable state machine.

use super::NetViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

/// One datapath probe surface: a human label, the upstream BPF
/// function it ports, and the live API endpoint backing it.
struct DatapathRow {
    feature: &'static str,
    upstream: &'static str,
    endpoint: &'static str,
}

const ROWS: &[DatapathRow] = &[
    DatapathRow {
        feature: "L4 port-range policy (LPM trie)",
        upstream: "policy/portrange.go PortRangeToMaskedPorts",
        endpoint: "GET /api/net/policy/port-range",
    },
    DatapathRow {
        feature: "EDT bandwidth scheduler",
        upstream: "bpf/lib/edt.h edt_sched_departure",
        endpoint: "POST /api/net/bandwidth/schedule",
    },
    DatapathRow {
        feature: "NAT46/64 address embedding",
        upstream: "bpf/lib/nat_46x64.h",
        endpoint: "GET /api/net/nat64/translate",
    },
    DatapathRow {
        feature: "DSR IPv4 option (set + extract)",
        upstream: "bpf/lib/nodeport.h dsr_set_opt4 / dsr_extract_opt4",
        endpoint: "GET /api/net/dsr/encode",
    },
    DatapathRow {
        feature: "LB session affinity (sticky backend)",
        upstream: "bpf/lib/lb.h __lb4_affinity_backend_id",
        endpoint: "POST /api/net/lb/affinity/simulate",
    },
    DatapathRow {
        feature: "LB source-range ACL (LoadBalancerSourceRanges)",
        upstream: "bpf/lib/lb.h lb4_src_range_ok",
        endpoint: "POST /api/net/lb/source-range/check",
    },
];

pub(crate) fn render_section(_state: &AdminState, _ctx: &RequestCtx) -> Result<String, NetViewError> {
    let table_rows: Vec<Vec<String>> = ROWS
        .iter()
        .map(|r| vec![r.feature.to_string(), r.upstream.to_string(), r.endpoint.to_string()])
        .collect();
    Ok(format!(
        r#"<section id="net-datapath" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">eBPF datapath sims ({n})</h2>
  <p class="text-sm text-gray-600 mb-2">
    Userspace simulation of Cilium's BPF hot path (<code>ebpf_sim</code>).
    Each surface replays the observable datapath state machine over <code>/api/net/*</code>.
  </p>
  {tbl}
</section>"#,
        n = ROWS.len(),
        tbl = table(&["feature", "upstream", "endpoint"], &table_rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;

    #[test]
    fn section_lists_new_datapath_surfaces() {
        let s = AdminState::seeded();
        let ctx = RequestCtx::developer("acme", &[Permission::NetRead]);
        let html = render_section(&s, &ctx).unwrap();
        assert!(html.contains("net-datapath"));
        assert!(html.contains("dsr_set_opt4"));
        assert!(html.contains("__lb4_affinity_backend_id"));
        assert!(html.contains("lb4_src_range_ok"));
        assert!(html.contains("/api/net/lb/source-range/check"));
    }
}
