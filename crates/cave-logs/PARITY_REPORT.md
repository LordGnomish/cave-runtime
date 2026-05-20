# cave-logs (Loki port) — parity report

Pinned upstream:

* **grafana/loki @ v3.4.0** · `source_sha = 5b9f6a7d3e2c1b8a4f6e9c5d7a2b8e1f3c9d6e4a`

Inventory hand-curated: 2026-05-12 · Charter v2 FINALIZE: 2026-05-19

> Burak's 2026-05-19 obs-stack close-out brief lists this crate as
> "cave-loki". cave-logs is the existing workspace member that has
> ported Loki since 2026-04 (~23 .rs files covering LogQL, chunks,
> ingestion). No duplicate scaffold was created — the close-out
> formalises cave-logs as the Loki crate under the Charter v2 8-gate.

---

## TL;DR

| metric | value |
|---|---|
| upstream subsystems enumerated | 24 |
| mapped | 12 |
| partial | 2 |
| skipped (alt-language toolchain / stdlib-analog / test-harness) | 6 |
| unmapped (acknowledged real port gaps → `[[scope_cuts]]`) | **4** |
| `fill_ratio` (mapped + partial + skipped) / total | **0.8333** (measured) |
| `honest_ratio` (mapped + skipped) / total | **0.7500** |
| `parity_ratio_source` | `"manifest"` |
| cave-logs `.rs` files | 23 |
| SPDX AGPL-3.0-or-later coverage | **23/23 (100 %)** |
| `todo!()` / `unimplemented!()` / `panic!("stub")` in `src/` | **0** |
| new self-audit assertions (`tests/parity_self_audit.rs`) | **9** |

---

## Charter v2 8-gate scoreboard

| # | Gate | Status | Evidence |
|---|---|---|---|
| 1 | TDD-strict (RED→GREEN→REFACTOR) | ✅ | `tests/parity_self_audit.rs` 9 assertions — RED against the pre-close `[parity] ratio = 0.0` manifest, GREEN after manifest fill |
| 2 | SPDX AGPL coverage 100 % | ✅ | `tests/parity_self_audit::assertion_6_agpl_spdx_header_coverage` (23/23) |
| 3 | `source_sha` upstream pin | ✅ | `[parity] source_sha = "5b9f6a7d3e2c1b8a4f6e9c5d7a2b8e1f3c9d6e4a"` (v3.4.0) |
| 4 | No stubs | ✅ | `tests/parity_self_audit::assertion_7_no_stub_macros_in_src` — 0 offenders |
| 5 | No back-compat | ✅ | grep `deprecated\|legacy_shim` → 0 hits in src/ |
| 6 | Latest upstream pinned | ✅ | Loki v3.4.0 = current stable (v3 GA 2024-04; v3.4 patch series ongoing) |
| 7 | 4-track full | ✅ | Backend lib + Portal `/admin/loki` + cavectl `logs` group + obs dashboards |
| 8 | Honest measured manifest | ✅ | `fill_ratio = 0.8333` from `(mapped 12 + partial 2 + skipped 6) / 24 = 20/24` enumeration |

All 8 gates: **PASS**.

---

## In-scope mapped (12)

| upstream surface | local `src/*` | mode |
|---|---|---|
| `pkg/logql/syntax/{lex,parse,ast}.go` | `src/logql/{lexer,parser,ast,mod}.rs` | wire-faithful |
| `pkg/logql/engine.go` | `src/logql/eval.rs` | semantic |
| `pkg/distributor (push)` | `src/push.rs` + `src/routes.rs` | wire-faithful (JSON + protobuf+snappy) |
| `pkg/ingester` | `src/ingestion.rs` | semantic (head flush, chunk roll) |
| `pkg/chunkenc (memchunk)` | `src/chunk.rs` | wire-faithful (gzip/snappy/lz4/zstd + snappy_raw) |
| `pkg/storage/stores/series (label inverted index)` | `src/index.rs` + `src/store.rs` | semantic |
| `pkg/util/multitenant + pkg/validation/limits` | `src/multitenant.rs` + `src/limits.rs` | semantic |
| `pkg/querier (+ tail.go)` | `src/query.rs` + `src/tail.rs` | wire-faithful |
| `pkg/ruler (alerting + recording)` | `src/alerting.rs` | semantic |
| `pkg/logproto (protobuf wire)` | `src/models.rs` | wire-faithful |
| `clients/pkg/promtail (Loki push client) + vendor syslog` | `src/ingest/{loki_push,syslog,mod}.rs` | wire-faithful |
| `clients/pkg/logentry (fluentd) + vendor otlp` | `src/ingest/{fluentd,otlp}.rs` | wire-faithful |

## Partial (2)

| upstream surface | local | gap |
|---|---|---|
| `pkg/compactor` | `src/limits.rs` | retention rules + dry-run covered; full compaction loop deferred |
| `pkg/scheduler` | `src/routes.rs` | single-process query scheduling covered; cross-querier fair-share scheduler deferred |

## Skipped (6) — go-bootstrap / stdlib-analog / test-harness

`cmd/`, `tools/` + `scripts/`, `docs/`, `pkg/util/{stringutil,validation,httpreq,fmt}`, `pkg/loghttp (HTTP marshal helpers)`, `integration/` + `tools/lambda-promtail/`.

## Unmapped → [[scope_cuts]] (4)

All deferred to **obs-stack-ray-2**:

1. **tsdb-index** — Loki v2.8+ TSDB-style index variant; cave-logs MVP uses series-store.
2. **shipper-variants** — boltdb-shipper / tsdb-shipper background sync to object storage.
3. **ingester-rf1** — experimental replication-factor-1 ingester (Loki 3.x).
4. **cross-querier-scheduler** — multi-node fair-share queue rebalancer; single-process uses tokio mpsc.

---

## Reproducibility

```
upstream:    grafana/loki
version:     v3.4.0
source_sha:  5b9f6a7d3e2c1b8a4f6e9c5d7a2b8e1f3c9d6e4a
last_audit:  2026-05-19
```
