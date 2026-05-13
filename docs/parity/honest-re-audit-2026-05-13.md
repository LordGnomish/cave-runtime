# Honest re-audit — 2026-05-13

This document describes the **honest re-audit** pass that ran on
2026-05-13 against the top-14 crates whose `parity_ratio > 0.6` and
whose source-of-truth was the on-disk `parity.manifest.toml::[parity]
fill_ratio` (i.e. manifest-sourced, not audit-doc-sourced).

The goal: surface the gap between **shape-level parity** (manifest
declares the upstream package as `[[mapped]]`) and **honest parity**
(the manifest author themselves admitted scope cuts in the `note`
field, or the local file is a shared idiom-mapping placeholder rather
than a real port).

## Method

Two passes, both mechanical and reproducible via
`scripts/honest-re-audit.py`:

### Pass A — Self-flagged partial detection

For every `[[mapped]]` block in a tier-1 manifest, the script scans
the `note` field for any of these self-flag patterns:

| pattern                       | meaning                                  |
|-------------------------------|------------------------------------------|
| `scope cut` / `honest scope`  | author admitted a feature was cut        |
| `deferred` / `not implemented` | feature is in scope but not done        |
| `caller bridges` / `caller drives` | local code is half the surface — caller must bring the rest |
| `no on-the-wire`              | pure state machine, no I/O               |
| `subset` / `MVP`              | partial coverage of the upstream API     |
| `placeholder` / `skeleton` / `stub-only` | not a real port at all         |
| `lacks` / `misses`            | enumerated gap                           |
| `TBD` / `WIP` / `first cut`   | explicit work-in-progress                |

A match demotes the block from `[[mapped]]` to a new `[[partial]]`
class. The body of the block is preserved verbatim — only the header
changes — so the `note` field reads as the same honest scope-cut
explanation it did before.

### Pass B — Shared-placeholder reclassification

Two files in `cave-net` host **dozens** of `[[mapped]]` entries each
that all share the same local file:

- `src/cilium/idiom_map.rs` (241 LOC, ~64 upstream pkgs) — a static
  `IdiomMapping[]` table mapping Cilium's tiny utility Go packages
  (`pkg/byteorder/`, `pkg/lock/`, `pkg/time/`, ...) to the Rust
  stdlib + well-known crates that replace them. There is no
  per-package port; the listed Rust replacement (`u32::to_be_bytes`,
  `std::net::IpAddr`, `tokio::sync::Mutex`, ...) is the runtime
  semantics. By the manifest schema's own classification, these are
  `[[skipped]]` blocks with reason `stdlib-analog`, not real ports.

- `src/cilium/binary_cites.rs` (141 LOC) — cross-binary citation
  table for Cilium's standalone-binary directories (`clustermesh-
  apiserver/`, `hubble-relay/`, etc.). Agent-side logic is ported
  separately; these entries are CLI-entrypoint skips by the charter.

The script reclassifies those entries as `[[skipped]]` with
`stdlib-analog` / `CLI` reason. `fill_ratio` is unchanged (skipped
counts in both numerator and denominator), but the manifest now
reflects the honest position.

## New schema fields

`crates/<name>/parity.manifest.toml`:

```toml
[parity]
mapped_count   = N    # fully ported (line-by-line behaviour)
partial_count  = M    # NEW — shape-only / scope-cut / MVP
skipped_count  = K
unmapped_count = U
total          = N+M+K+U
fill_ratio     = (N+M+K) / total
honest_ratio   = (N+K)   / total   # NEW — partial excluded
```

`docs/parity/parity-index.json` gains `honest_ratio`, `mapped_count`,
`partial_count`, `skipped_count`, `unmapped_count`, `total_count`
per crate. The compliance dashboard renders a new **Honest Parity**
card alongside Structural, Upstream Parity, and Portal UI Parity.

## Aggregate result — top 14 crates

| Crate                    | old fill | new fill | new honest | Δ honest | partials | reclass |
|--------------------------|---------:|---------:|-----------:|---------:|---------:|--------:|
| cave-cache               |   0.9474 |   0.9474 |     0.8421 |  -0.1053 |        4 |       0 |
| cave-cri                 |   0.9412 |   0.9412 |     0.8529 |  -0.0883 |        3 |       0 |
| cave-mesh                |   0.8571 |   0.8571 |     0.8286 |  -0.0285 |        1 |       0 |
| cave-etcd                |   0.9155 |   0.9155 |     0.8873 |  -0.0282 |        2 |       0 |
| cave-auth                |   0.7838 |   0.7838 |     0.7568 |  -0.0270 |        1 |       0 |
| cave-controller-manager  |   0.8000 |   0.8000 |     0.7778 |  -0.0222 |        1 |       0 |
| cave-apiserver           |   0.8824 |   0.8824 |     0.8627 |  -0.0197 |        1 |       0 |
| cave-net                 |   0.9179 |   0.9179 |     0.9179 |        0 |        0 |      64 |
| cave-scheduler           |   0.8966 |   0.8966 |     0.8966 |        0 |        0 |       0 |
| cave-kubelet             |   0.8684 |   0.8684 |     0.8684 |        0 |        0 |       0 |
| cave-rdbms-operator      |   0.8400 |   0.8400 |     0.8400 |        0 |        0 |       0 |
| cave-streams             |   0.8182 |   0.8182 |     0.8182 |        0 |        0 |       0 |
| cave-vault               |   0.7895 |   0.7895 |     0.7895 |        0 |        0 |       0 |
| cave-karpenter           |   0.6471 |   0.6471 |     0.6471 |        0 |        0 |       0 |

**Totals:** 13 mapped→partial demotions, 64 mapped→skipped (stdlib-analog/CLI)
reclassifications. Honest-parity score on the dashboard drops accordingly
on the seven affected crates.

### Footnote on cave-streams / cave-vault / cave-rdbms-operator

The mechanical pass finds **zero** self-flagged partials in these
manifests. Two possibilities:

1. The notes are genuinely thin (single-sentence shapes that don't
   admit scope cuts). The regex can't find what isn't there.
2. The implementations really are line-by-line ports.

Without a deeper manual port-by-port review (next audit iteration),
the script cannot distinguish (1) from (2). The honest position is
to leave these at `fill_ratio == honest_ratio` and revisit when a
human re-audit is available.

### Footnote on cave-net

The 64 reclassifications move entries from `[[mapped]]` to `[[skipped]]`
with `stdlib-analog` / `CLI` reason. `fill_ratio` and `honest_ratio`
are unchanged because the charter counts `[[skipped]]` the same way
as `[[mapped]]` — both are "not a gap". The manifest now classifies
correctly per the schema's own definition, which matters for any
future audit that wants to enumerate "what was actually ported" vs
"what's covered by stdlib".

## Per-crate detail

### cave-cache — 4 partials

| Upstream                       | Local files                            | Honest gap |
|--------------------------------|----------------------------------------|------------|
| `redis:src/cluster.c`          | `src/cluster/{mod,state,slots,gossip,migration,epoch}.rs` | "the on-wire bus serializer for upstream's port-16380 listener is not implemented" |
| `redis:src/sentinel.c`         | `src/sentinel.rs`                      | "this is the pure state machine — the gossip TCP socket + SENTINEL command set surface lives in the server crate" |
| `redis:src/replication.c`      | `src/replication.rs`                   | "no on-the-wire RDB encoder + TCP socket lifecycle — caller bridges this state to the wire" |
| `redis:src/tls.c`              | `src/tls_listener.rs`                  | "cryptographic key-cert match + handshake live in rustls server-config builder the caller drives" |

### cave-cri — 3 partials

| Upstream         | Local files                                                   | Honest gap |
|------------------|---------------------------------------------------------------|------------|
| `core/content/`  | `src/content/{mod,digest,store,writer}.rs`                    | "no boltdb metadata persistence (in-memory index rebuilds via directory walk on open) and no cross-process locking" |
| `core/diff/`     | `src/diff/{mod,walking_differ,compression}.rs`                | "double-tree Diff *production* path (snapshot Δ → tarball) is still left to overlayfs at mount time" |
| `core/leases/`   | `src/leases/{mod,manager,resource}.rs`                        | "no boltdb persistence — leases are in-memory and must be re-registered after restart" |

### cave-etcd — 2 partials

| Upstream                                  | Local files     | Honest gap |
|-------------------------------------------|-----------------|------------|
| `server/etcdserver/api/v3alarm/`          | `src/maintenance.rs` | "Basic alarm surface — full alarm storage subset" |
| `server/etcdserver/api/v3compactor/`      | `src/routes.rs` | "Compaction is operator-triggered; auto-compactor is a subset" |

### cave-apiserver — 1 partial

| Upstream                                          | Local files       | Honest gap |
|---------------------------------------------------|-------------------|------------|
| `staging/src/k8s.io/apiserver/pkg/cel/`           | `src/cel_eval.rs` | "Real CEL evaluator (CelInterpreterEvaluator) ... Supports the VAP CEL subset" |

### cave-mesh — 1 partial

| Upstream                                          | Local files            | Honest gap |
|---------------------------------------------------|------------------------|------------|
| `istio/istio:pilot/pkg/networking/extension/`     | `src/wasm_plugin.rs`   | "no embedded wasmtime/wasmer runtime — the actual bytecode execution stays in the bound envoy" |

### cave-controller-manager — 1 partial

| Upstream                                          | Local files            | Honest gap |
|---------------------------------------------------|------------------------|------------|
| `pkg/controller/cidrallocator/`                   | `src/cidrallocator.rs` | pod-CIDR slice allocator — MVP; no cloud-provider integration |

### cave-auth — 1 partial

| Upstream                                          | Local files                     | Honest gap |
|---------------------------------------------------|---------------------------------|------------|
| `keycloak:saml-core/ + services/.../protocol/saml/` | `src/saml/*.rs` (8 files)     | "Built-in exc-c14n canonicalization (rfc3741 subset) — caller-pluggable hook remains for strict-mode IdPs" |

### cave-net — 64 reclassifications, 0 partials

61 entries moved to `[[skipped]] reason = "stdlib-analog"` (Cilium
micro-pkgs covered by Rust stdlib / well-known crates per the
`idiom_map.rs` table), plus 3 moved to `[[skipped]] reason = "CLI"`
(Cilium standalone-binary entrypoints whose agent-side logic is
already ported in cave-net).

## Aggregate dashboard impact

Before this pass:
- Upstream Parity grade — driven by `fill_ratio` average across
  tier-1 measurable crates.
- No "honest" axis — the dashboard could not surface scope cuts
  the manifest authors had already admitted.

After this pass:
- Upstream Parity grade unchanged (fill_ratio unchanged for all 14).
- **NEW Honest Parity grade** — driven by `honest_ratio` average.
  For the seven crates with self-flagged partials, the honest grade
  is up to 11 points lower than the shape grade.

## Limitations / honest gaps in this audit

1. **Mechanical only.** The regex catches what authors explicitly
   admitted in `note`. It does NOT find shape-only ports whose notes
   are silent about scope cuts. A deeper manual port-by-port review
   would likely surface more partials in cave-scheduler, cave-kubelet,
   cave-streams, cave-vault, and cave-rdbms-operator — none of which
   the regex demoted. The honest position is "current pass is a
   floor on the partial count, not a ceiling".

2. **Audit-sourced ratios untouched.** Crates whose `parity_ratio` is
   audit-doc-sourced (`cave-cloud-controller-manager`, `cave-portal`,
   `cave-kube-proxy`, `cave-local-llm`) do not have manifest
   inventories to scrutinize. Their honest_ratio is unchanged at
   fill_ratio.

3. **Behavioral parity not in scope.** The "behavioral axis"
   (running upstream test suites against the cave implementation)
   is parallel-session work, not this audit. The honest_ratio is
   still structural — "did the author claim a real port?" — not
   functional — "does the port produce the same output as upstream?".

4. **No semantic LOC comparison.** A future pass could compare local
   file LOC to upstream LOC for each `[[mapped]]` entry and flag
   any with a ratio < 0.05 as suspect. The current pass does not.

## Reproducibility

```bash
# Re-run the demotion pass (idempotent — already-demoted entries are detected by upstream_pkg)
python3 scripts/honest-re-audit.py

# Regenerate the parity index with honest_ratio + breakdown
python3 scripts/build-parity-index.py

# Inspect the dashboard's new card
cargo run -p cave-runtime -- serve --data-dir /tmp/cave-data
# → http://localhost:6443/admin/compliance
```

## Charter alignment

This audit serves the **honest measurement** golden rule: never
self-report a percentage above what the code earns. Demoting
self-flagged scope-cuts to `[[partial]]` and reclassifying
shared-placeholder mappings to `[[skipped]]` makes the dashboard
honest about what's actually ported vs. what's authored but
incomplete.
