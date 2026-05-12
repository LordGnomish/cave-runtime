# cave-scheduler parity — 2026-05-12 measured audit

**Upstream pin:** `kubernetes/kubernetes` `pkg/scheduler/*` (v1.31.x; the manifest's pinned `v1.36.0` claim is preserved).

## Why this exists

2026-05-01 audit placed cave-scheduler at **tier 100, parity_ratio = 1.0** via the wave3 mechanical metric (33 declared files, 19 fns, 107 tests, 3 surfaces — all matched). That number does not reflect upstream-package coverage; it reflects the local manifest's own narrow declarations.

This pass replaces it with a measured `fill_ratio` over the enumerated `pkg/scheduler/*` package tree.

## Methodology

Same shape as cave-etcd (2026-05-12). Each entry is `[[mapped]]` (cave has impl) / `[[skipped]]` (out-of-scope with enumerated reason: `go-bootstrap` | `stdlib-analog` | `test-harness` | `parallel-track`) / `[[unmapped]]` (real gap, acknowledged).

## Counts

| Bucket | Count |
|---|---:|
| `[[mapped]]` | 18 |
| `[[skipped]]` | 7 |
| `[[unmapped]]` | 4 |
| **Total** | **29** |
| **fill_ratio** | **0.8621** |

The previous self-reported `parity_ratio = 1.0` is replaced by `fill_ratio = 0.8621`.

## Mapped highlights

- **Framework + plugins** — `framework.rs` (54 KB) + `plugins.rs` (55 KB) cover PreFilter / Filter / PostFilter / PreScore / Score / Reserve / Permit / PreBind / Bind / PostBind extension points and all in-tree score plugins.
- **DRA scheduler hooks** — `dra.rs` + `dra_scheduler.rs` already wired (KEP-4381, v1.32 beta).
- **Preemption** — `preempt.rs` + `default_preemption.rs` implement the standard graceful-eviction path.
- **Multi-profile** — `profiles.rs` handles multiple scheduler instances in one binary.

## Unmapped (real gaps)

1. **imagelocality scorer** — emits 0 because cave-kubelet's image cache is not exposed to the scheduler. Needs an image-presence API on cave-cri.
2. **interpodaffinity soft scoring** — Filter works; PreScore/Score paths return neutral 0, so soft anti-affinity preferences are no-ops.
3. **Preemption-victim restoration** — failed eviction during the window leaks victim state instead of re-queuing.
4. **volumezone enforcement** — topology-aware provisioning exists but per-zone restrictions are not enforced at schedule time.

## Out of scope for this audit

- No new plugin ports landed in this pass; the audit is documentation-only. The 4 unmapped items are individually small (~100–300 LOC each), suitable for follow-up sweeps.
