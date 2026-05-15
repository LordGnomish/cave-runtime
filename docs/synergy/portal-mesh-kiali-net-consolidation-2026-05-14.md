# Portal mesh / kiali / net consolidation

**Date:** 2026-05-14
**Status:** Landed. `/admin/mesh` is the canonical service-mesh +
network dashboard; `/admin/kiali` and `/admin/net` 308-redirect into
it with anchor hashes preserving every legacy deep-link.

## Audit (before)

| URL | Handler | LOC | Sections |
|---|---|---:|---|
| `/admin/mesh` | `mesh::render` (flat) | 484 | Workloads, Services, AuthZ, Mesh Flows |
| `/admin/kiali` | `kiali::render` (composite) | 842 across 6 files | Topology, Workloads, Services, Traffic, Validations |
| `/admin/net` | `net::render` (composite) | 780 across 6 files | Flows, Policies, Services, Nodes, Identities |

**Overlap surfaced:** Workloads (mesh + kiali), Services (mesh + kiali + net), Flows (mesh + net). Operator had three top-nav entries painting overlapping data with different shapes.

## Decision

`/admin/mesh` is canonical because it's already the
AuthorizationPolicy + flow-log editor (mesh-write surface), it ships
with the `MeshState` runtime client, and it was already
COMPLETE in the previous parity sweep. Promotion strategy:

* `mesh::render` composes its own 4 sections PLUS:
  * Topology / Traffic / Validations from `kiali::*::render_section`
  * NetworkPolicies / Nodes / Identities + L3/L4/L7 Flows from `net::*::render_section`
* Tab nav inside the mesh page links by anchor hash (`#kiali-topology`, `#net-flows`, etc.).
* `kiali_handler` and `net_handler` become 308 `Redirect::permanent(...)`.

Legacy anchor IDs preserved verbatim so external bookmarks /
documentation links still scroll-into-view on the unified page:

| Legacy anchor | Survives where |
|---|---|
| `#kiali-topology` | new `Topology` section header |
| `#kiali-workloads` | shadows `#mesh-workloads` (Workloads section) |
| `#kiali-services` | shadows `#mesh-services` |
| `#kiali-traffic` | new `Traffic` section |
| `#kiali-validations` | new `Validations` section |
| `#net-flows` | new `Network Flows` section |
| `#net-policies` | new `NetworkPolicies` section |
| `#net-services` | shadows `#mesh-services` |
| `#net-nodes` | new `Nodes` section |
| `#net-identities` | new `Identities` section |

## Permission gating

Each composed section is gated on its own permission via
`render_section()` returning `Auth(Permission::*Read)` on miss. The
unified `mesh::render` swallows those errors so a caller with only
`MeshRead` still gets the mesh-native sections; kiali/net sections
silently drop their bodies. The nav anchors stay because they
point at sections that *would* render with broader grants.

## Visibility tweak

The 7 `render_section` helpers in `kiali/` + `net/` were promoted
from `pub(super)` to `pub(crate)` so the sibling `mesh::render` can
call them. No other change to those modules.

## Live HTTPS smoke

```
==> redirect status table
  /admin/mesh     status=200
  /admin/kiali    status=308  location: /admin/mesh?tenant_id=dev#kiali-topology
  /admin/net      status=308  location: /admin/mesh?tenant_id=dev#net-flows

==> Anchor presence in /admin/mesh body
  kiali-topology         ✓     kiali-workloads        ✓
  kiali-services         ✓     kiali-traffic          ✓
  kiali-validations      ✓     net-flows              ✓
  net-policies           ✓     net-services           ✓
  net-nodes              ✓     net-identities         ✓
  mesh-workloads         ✓     mesh-services          ✓
  mesh-authz             ✓     mesh-flows             ✓

==> curl follow-redirect resolves to:
  /admin/kiali  → http://127.0.0.1:18491/admin/mesh?tenant_id=dev#kiali-topology
  /admin/net    → http://127.0.0.1:18491/admin/mesh?tenant_id=dev#net-flows
```

## Feature parity

Every exclusive feature from `kiali/*` and `net/*` is now reachable
from the unified `/admin/mesh` page. The non-exclusive features
(Workloads, Services) were already in `/admin/mesh` with richer
shapes (per-service health badge, bytes-dropped counter); we kept
the mesh-native render.

| Feature | Source | Surfaced on /admin/mesh? |
|---|---|---|
| Kiali topology graph | kiali/topology.rs | ✓ (`#kiali-topology`) |
| Kiali workload list | kiali/workloads.rs | redundant — mesh has its own |
| Kiali service list | kiali/services.rs | redundant — mesh has its own |
| Kiali traffic (VS/DR/Gateway) | kiali/traffic.rs | ✓ (`#kiali-traffic`) |
| Kiali validations | kiali/validations.rs | ✓ (`#kiali-validations`) |
| Cilium Hubble flows (L3/L4/L7) | net/flows.rs | ✓ (`#net-flows`) |
| Cilium NetworkPolicy | net/policies.rs | ✓ (`#net-policies`) |
| K8s Service list | net/services.rs | redundant — mesh has its own |
| Cilium node + endpoint browser | net/nodes.rs | ✓ (`#net-nodes`) |
| Cilium security identity catalog | net/identities.rs | ✓ (`#net-identities`) |
| Mesh AuthorizationPolicy editor | mesh.rs | ✓ (`#mesh-authz`) |
| Mesh flow log | mesh.rs | ✓ (`#mesh-flows`) |

## Workspace impact

* `crates/cave-portal/src/admin/mesh.rs`: +89 LOC (composition + 5 new tests).
* `crates/cave-portal/src/admin/mod.rs`: handler `net_handler` reworked as 308 + helper `urlencode_query` added; `kiali_handler` reworked as 308.
* 7 files in `kiali/` + `net/`: `pub(super)` → `pub(crate)` (single-line bump each).
* No new tests broken. `cargo test -p cave-portal --lib`: 1853 → **1858** (+5 consolidation tests).
* `cargo check --workspace` clean.

## What didn't change

* Old kiali/ and net/ modules are NOT deleted — they continue to
  expose `list_endpoints`, `list_policies`, `create_policy`,
  `delete_policy`, `list_edges`, `edge_health`, `GraphNode`,
  `TopologyEdge` etc. as the legacy public surface for any
  consumer that imports them programmatically (cavectl, tests,
  in-process callers). Only the URL handlers redirect.
* The sidebar nav (`crates/cave-portal/src/admin/layout/nav.rs`)
  keeps its "Kiali" + "Networking" entries pointing at
  `/admin/kiali` and `/admin/net`. Clicks 308 through to the
  unified page; cleaner UX (direct-target the nav at mesh anchors)
  is a follow-up.
* No data migration — both modules continue reading the same
  `AdminState` fields they already did.

## Honest deferrals

* **Sidebar entry consolidation** — kiali + networking entries
  could point straight at `/admin/mesh#kiali-topology` and
  `/admin/mesh#net-flows` instead of relying on the 308. Trivial,
  not strictly necessary, deferred.
* **kiali/net file deletion** — the modules still exist for their
  programmatic API. Once cavectl + admission tests adopt the
  unified mesh surface, the entire `kiali/` + `net/` directories
  could be flattened back into a single helper module. Out of
  scope; that's a refactor sweep.
* **Section ordering tuning** — the current order is
  topology → workloads → services → traffic → authz → mesh-flows →
  net-flows → policies → validations → nodes → identities. An
  operator playtest may swap a few; left as-is for now.
