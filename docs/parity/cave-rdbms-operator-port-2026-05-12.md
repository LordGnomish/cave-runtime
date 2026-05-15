# cave-rdbms-operator parity — 2026-05-12 audit

**Upstreams:** `cloudnative-pg/cloudnative-pg v1.24.0` (primary,
Apache-2.0) + `pgbouncer/pgbouncer 1.21.0` (secondary, ISC).

## Methodology

Identical to the cave-etcd audit (see
`cave-etcd-port-2026-05-12.md`): each non-trivial top-level
package in the CloudNativePG repo is classified `[[mapped]]` /
`[[skipped]]` / `[[unmapped]]`. The PgBouncer surface contributes
one mapped entry (the pool primitives in `src/pool.rs`).

Module inventory was hand-curated against the CNPG repo layout
(`api/v1`, `controllers/`, `internal/`, `pkg/`, `plugin/`) as of
the v1.24.x release.

## Counts

| Bucket   | Count |
|----------|------:|
| Mapped   | 9 |
| Skipped  | 10 |
| Unmapped | 5 |
| **Total** | **24** |
| **fill_ratio** | **0.7917** |

## What lands in the inventory

* **Mapped (9)** covers the operator surface that's already wired:
  Cluster CRD spec, instance lifecycle, replica registration + lag
  + failover, Barman backup loop, PostgreSQL exporter metrics,
  user / role CRUD, the PgBouncer pool, and the unified HTTP admin
  API.
* **Skipped (10)** covers Charter-justified out-of-scope packages
  — kubebuilder boilerplate, the operator binary main (replaced
  by `cave-runtime serve`), Helm/manifest install, kubectl plugin
  (replaced by `cavectl`), Go-stdlib glue, docs site.
* **Unmapped (5)** covers honest gaps: per-pod sidecar webserver,
  replica-cluster (cross-cluster) failover, ScheduledBackup
  cron CRD, the built-in catalog of monitoring queries, and the
  pgaudit extension lifecycle.

## What this PR does NOT claim

* No new code lands in cave-rdbms-operator from this audit pass.
* The 5 unmapped packages are tracked work, not implemented.
* `fill_ratio = 0.7917` is `(mapped + skipped) / total` — it does
  NOT claim "79% of the operator's behaviour is shipped"; it claims
  "79% of the upstream's relevant packages are either covered or
  honestly out of scope".
