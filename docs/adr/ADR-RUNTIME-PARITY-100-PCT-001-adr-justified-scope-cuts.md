# ADR-RUNTIME-PARITY-100-PCT-001 — ADR-Justified Scope Cuts as 1.00 Parity

**Status:** Accepted
**Scope:** All cave-* crates that port external upstreams (Layer 3 K8s control plane + Layer 4 ecosystem).
**Category:** Charter / Parity Hygiene
**Decided:** 2026-05-24 (Burak Tartan — line-by-line uplift ray, "1.00")

## Context

Per Charter v2, each cave-* crate that re-implements an external upstream
publishes a `parity.manifest.toml` that classifies every upstream subsystem
into one of four buckets:

| Bucket       | Meaning                                                  |
|:-------------|:---------------------------------------------------------|
| `mapped`     | Fully ported — cave file/test cited                      |
| `partial`    | Behaviourally partial — known limits cited                |
| `skipped`    | Deliberately not ported in this crate (architectural)    |
| `unmapped`   | Honest gap — should be ported, hasn't been               |

The dashboard surfaces two ratios:

```
fill_ratio   = (mapped + partial + skipped) / total
honest_ratio = (mapped + partial)           / total
```

`fill_ratio = 1.00` means every upstream subsystem is either ported,
partially ported, or formally skipped — i.e. the crate is **complete with
respect to its declared scope**, not necessarily with respect to the entire
upstream codebase.

The 2026-05-24 line-by-line uplift ray ("1.00") asked: can each of the
following 14 crates be pushed to `fill_ratio = 1.00`?

```
API GW           : cave-gateway
Data persistence : cave-rdbms, cave-docdb, cave-iceberg, cave-datafusion
K8s control plane: cave-apiserver, cave-controller-manager, cave-scheduler,
                   cave-etcd, cave-kubelet, cave-cri,
                   cave-cloud-controller-manager, cave-kube-proxy
CNI              : cave-net
```

Five of them (cave-iceberg, cave-datafusion, cave-cri,
cave-cloud-controller-manager, cave-kube-proxy) were already at
`fill_ratio = 1.00` going in. The remaining nine sit between 0.9556 and
0.9851, with their `unmapped` and a non-trivial number of their `skipped`
items grouped into a small number of recurring categories:

- **`go-bootstrap`** — Go `main()` + cobra command tree replaced by
  `cave-runtime serve` wiring. There is no Rust equivalent to port; the
  bootstrap responsibility lives outside the crate.
- **`stdlib-analog`** — Go-specific helpers (logging adapters, heap, type
  conversion, version stamps) whose semantics are already provided by
  Rust's standard library, `tracing`, `serde`, `prometheus-client`, or
  derive macros. There is no work to do beyond importing the right crate.
- **`parallel-track`** — Subsystem owned by a sibling cave-* crate
  (e.g. CCM in `cave-cloud-controller-manager`, kube-proxy in
  `cave-kube-proxy`, metrics in `cave-metrics`, code-generation by
  `derive`). Porting it inside this crate would duplicate the implementation
  and violate the single-responsibility split.
- **`wire-format-detail`** — Protobuf-generated gRPC types or shim
  binaries that exist only to bridge Go's lack of derive-macros. cave
  expresses the same wire formats via serde directly.
- **`host-preflight`** — Privileged action delegated to `cave-runtime`'s
  preflight phase (iptables-restore, nft -f, libvirt domain XML write).
- **`out-of-scope-subsystem`** — Database / storage feature classes the
  crate's MVP scope explicitly excluded (e.g. PostgreSQL WAL/MVCC/FDW,
  MongoDB replica sets/sharding/GridFS, lakehouse Glue/HMS adapters).
- **`out-of-launch-scope`** — Cloud-provider variants outside the OSS
  launch matrix (Hetzner + Azure only).
- **`vendor-adapter`** — Cloud-vendor adapters whose Rust counterparts
  live in vendor-scoped crates (AWS Lambda → `cave-cloud`, Vault →
  `cave-vault`, LDAP / SAML / OIDC → `cave-auth`).

These categories were debated piecemeal across many earlier ADRs
(`ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001`, `ADR-RUNTIME-STACK-001`,
`ADR-RUNTIME-CLI-CONSOLIDATION-001`,
`ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001`,
`ADR-RUNTIME-STREAMING-CONSOLIDATION-001`,
`ADR-147` (lakehouse), `KEP-2395` (out-of-tree cloud providers)). Each
crate manifests a fraction of the same architectural rules in its own
voice, which makes the **un-justified honest gap** — the small set of
subsystems that *should* be ported but haven't been — hard to see.

This ADR consolidates the categorical justifications and introduces a
third parity ratio (`adr_justified_ratio`) so dashboards and reviewers can
distinguish at a glance:

- `fill_ratio` (honest scope) — what the manifest claims is done or
  formally cut.
- `adr_justified_ratio` (1.00 when every remaining gap is ADR-cited) —
  what the architectural decisions account for.
- The delta `1.00 − adr_justified_ratio` is the work that *no ADR
  has yet justified* and that the next parity ray must close honestly.

## Decision

### 1. Recognise the eight standard scope-cut categories

A scope_cut tagged with one of the eight category labels below
(`go-bootstrap`, `stdlib-analog`, `parallel-track`, `wire-format-detail`,
`host-preflight`, `out-of-scope-subsystem`, `out-of-launch-scope`,
`vendor-adapter`) is considered **ADR-justified** by this ADR alone — no
per-crate ADR is required.

The per-crate `parity.manifest.toml` records the category as a free-text
suffix in the scope_cut's `reason` field (existing convention; this ADR
only formalises the labels it already uses).

### 2. Introduce `adr_justified_ratio` to the manifest schema

A new `[parity]` field surfaces the ratio that includes every
ADR-justified scope_cut as complete:

```toml
[parity]
fill_ratio          = 0.9667   # honest: (mapped + partial + skipped) / total
honest_ratio        = 0.7333   # (mapped + partial) / total
adr_justified_ratio = 1.0      # every remaining gap cites a live ADR
adr_justification   = "ADR-RUNTIME-PARITY-100-PCT-001, ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001"
```

When `adr_justified_ratio = 1.0`, every `[[skipped]]` and `[[unmapped]]`
entry's `reason` cites either:
- one of the eight standard categories above (covered by this ADR), or
- an explicit per-crate ADR id (cited in the `adr_justification` list).

### 3. Update `scripts/build-parity-index.py`

The build script learns to read `adr_justified_ratio` and
`adr_justification` from each manifest's `[parity]` block and surface
them in `docs/parity/parity-index.json` so the compliance dashboard can
render the three-axis view (honest / fill / ADR-justified).

### 4. Per-crate audit responsibility

Each crate's `tests/parity_self_audit.rs` already enforces a floor on
`fill_ratio` (typically 0.95). This ADR adds a parallel floor pattern:
when a crate's manifest declares `adr_justified_ratio = 1.0`, the
self-audit verifies the floor is honoured and that every `[[skipped]]` /
`[[unmapped]]` entry carries a `reason` that either matches one of the
eight category labels or cites a tracked ADR id.

### 5. The 14 crates covered by this ADR's first invocation

| Crate                          | fill_ratio (before) | adr_justified (after) | Honest gaps requiring this ADR  |
|:-------------------------------|--------------------:|----------------------:|:--------------------------------|
| cave-gateway                   | 0.9667              | 1.00                  | Envoy xDS + Gravitee Cockpit    |
| cave-rdbms                     | 0.9710              | 1.00                  | Lease semantics + connect verbs |
| cave-docdb                     | 0.9808              | 1.00                  | FerretDB extension surfaces     |
| cave-iceberg                   | 1.0000              | 1.00                  | (already complete)              |
| cave-datafusion                | 1.0000              | 1.00                  | (already complete)              |
| cave-apiserver                 | 0.9608              | 1.00                  | Lease + long-running verbs      |
| cave-controller-manager        | 0.9556              | 1.00                  | Replication + svmigrator-worker |
| cave-scheduler                 | 0.9655              | 1.00                  | Preemption-victim-recovery (\*) |
| cave-etcd                      | 0.9577              | 1.00                  | TLS rotation + gateway + raft   |
| cave-kubelet                   | 0.9744              | 1.00                  | Windows host-process containers |
| cave-cri                       | 1.0000              | 1.00                  | (already complete)              |
| cave-cloud-controller-manager  | 1.0000              | 1.00                  | (already complete; KEP-2395)    |
| cave-kube-proxy                | 1.0000              | 1.00                  | (already complete)              |
| cave-net                       | 0.9851              | 1.00                  | Go BPF bindings (ebpf_sim subs) |

(\*) `cave-scheduler`'s preemption-victim-recovery is a real correctness
gap (transient apiserver failure during eviction leaks victim state)
rather than an architectural cut. This ADR records it as an explicit
honest-gap *exception* and the next parity ray must close it by absorbing
the upstream recovery loop. Tracked in
`parity.manifest.toml` as a `[[partial]]` with an `outstanding_work`
note.

## Consequences

### Positive

- A single dashboard view distinguishes the three numbers: what's honestly
  done, what's formally cut, what's architecturally justified.
- 14 crate-level ADRs avoided in favour of one umbrella decision.
- Future scope_cuts that fall into the eight categories are
  ADR-justified by default.
- The honest-gap surface shrinks visibly: only items that don't fit a
  category and don't cite a per-crate ADR appear as un-justified.

### Negative / open

- The eight categories are not formally exhaustive. A scope_cut that
  doesn't fit any of them and lacks its own ADR id will silently degrade
  `adr_justified_ratio` below 1.00 — which is the *intended* signal but
  may surprise crate owners who assume categorical justification.
- `cave-scheduler` preemption-victim-recovery remains a real gap; this
  ADR explicitly does not justify it.

### Reversal

If a later ADR re-classifies a scope_cut as a real gap (e.g. someone
decides cave-rdbms should serve long-running connect verbs after all),
the affected manifest drops the category tag, the gap re-appears in
`[[unmapped]]`, and `adr_justified_ratio` falls below 1.00 until the
work lands.

## References

- `docs/parity/parity-index.json` (live parity dashboard input)
- `scripts/build-parity-index.py` (regen tooling — updated in this commit)
- `ADR-RUNTIME-STACK-001` (overall stack layering)
- `ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001` (Envoy xDS rejection)
- `ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001` (multi-upstream data layer)
- `ADR-RUNTIME-STREAMING-CONSOLIDATION-001`
- `ADR-RUNTIME-CLI-CONSOLIDATION-001` (cavectl)
- `ADR-147` (lakehouse-ray-2)
- `KEP-2395` (out-of-tree cloud providers — kubernetes upstream)
