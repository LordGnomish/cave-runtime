# Data Persistence Track — Phase-A Baseline

**Date:** 2026-04-29
**Branch:** `feat/data-persistence-phase-a-baseline` (parent: `feat/data-persistence-100-pct-sprint`)
**Goal:** Establish a real, auditable parity baseline for the 7 data-persistence
crates against their upstream sources. **No source code edits; manifests + audit + report only.**

This is the *baseline* — the inputs needed for an honest Phase-B sprint plan.
No "REAL %100" claim is made here; the table below names exactly how far each
crate is from that goal.

---

## TL;DR

| Metric | Value |
|---|---|
| Crates audited | 7 |
| Total upstream surface items enumerated | **2 198** |
| Mapped | 619 (28.2%) |
| Partial (Iceberg, Search, DataFusion only) | 72 (3.3%) |
| Unmapped / missing | 1 292 (58.8%) |
| Skipped (UI / CLI / backcompat / out-of-scope) | 215 (9.8%) |
| **Strict fill** = (mapped + skipped) / total | **0.379** |
| **Lenient fill** = (mapped + partial + skipped) / total | **0.412** |
| Cargo tests passing (per-crate) | **1 318 / 1 318** ✓ |

**Honest verdict:** Phase-A confirms the existing 1 318 tests are real and green,
**and** that the manifest fill across all 7 crates is **far** from 1.0 — the
prior `M*: %100` commit messages reflect *internal* feature scope, not parity
against upstream public surface.

---

## Per-crate baseline

| Crate | Upstream | Total | Mapped | Partial | Unmapped | Skipped | Strict fill | Tests |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| cave-pg | postgres@`ca9807d` + cnpg@`f8283b0` | 255 | 80 | 0 | 140 | 35 | **0.451** | 384 ✓ |
| cave-docdb | mongo@`d5cfd3d` | 271 | 72 | 0 | 164 | 35 | **0.395** | 113 ✓ |
| cave-cache | valkey@`16ed690` | 499 | 351 | 0 | 110 | 38 | **0.780** | 220 ✓ |
| cave-blob | minio@`7aac2a2` | 243 | 32 | 0 | 207 | 4 | **0.148** | 168 ✓ |
| cave-iceberg | iceberg@`c81534f` | 167 | 45 | 19 | 95 | 8 | **0.317** | 199 ✓ |
| cave-search | opensearch@`96bb22d` | 397 | 18 | 27 | 264 | 88 | **0.267** | 84 ✓ |
| cave-datafusion | datafusion@`3aefba7` | 366 | 21 | 26 | 312 | 7 | **0.077** | 150 ✓ |
| **TOTAL** | | **2 198** | **619** | **72** | **1 292** | **215** | **0.379** | **1 318** ✓ |

`Strict fill` counts only `mapped` + `skipped`; `partial` items are *not* counted
as parity. Cave-iceberg, cave-search, and cave-datafusion agents introduced a
`partial` status to flag wire-incompatible-but-reachable surface; treat partial
as an unmapped variant for planning.

Per-crate manifests live at `crates/<name>/parity.manifest.toml` and pin the
exact upstream commit SHA used for enumeration.

---

## Skipped breakdown — what's *intentionally* out of scope

The skipped column counts items the manifest explicitly excludes per Burak's
rule (UI / CLI / backcompat). Numbers below are agent-reported.

| Crate | UI | CLI | backcompat | other (engine, sentinel, platform) | Total |
|---|---:|---:|---:|---|---:|
| cave-pg | 0 | 22 | 9 (auth-legacy + protocol-v2 + deprecated catalogs) | 4 (PL bindings, sepgsql, Windows event-log) | 35 |
| cave-docdb | 13 (Atlas-only stages) | 9 | 13 (OP_QUERY/OP_INSERT/OP_UPDATE/OP_DELETE/OP_KILL_CURSORS pre-3.6, MONGODB-CR, mmapv1) | 0 | 35 |
| cave-cache | 0 | 5 | 10 (RDB v9/v10, deprecated cmds, AOFv1) | 23 (sentinel-* family ×21 + AE/kqueue/IO-threads ×2) | 38 |
| cave-blob | 0 | 1 (mc) | 3 (SigV2, presigned-V2, ListObjectsV1) | 0 | 4 |
| cave-iceberg | 3 (engine integrations: Spark/Flink/Hive) | 1 | 0 | 4 (storage engine internals) | 8 |
| cave-search | 21 (plugins: security/k-NN/ml-commons/neural-search/alerting/sql/geospatial/repo-*) | 0 | 0 | 67 (no-snapshot ×12, no-scripting/Painless ×12, single-node ×7, no-parent-child ×6, no-geo ×7, no-ingest ×5, no-data-streams ×4, no-on-disk ×3, no-search-pipelines ×3, no-remote-store ×2, misc ×6) | 88 |
| cave-datafusion | 0 | 1 (datafusion-cli) | 0 | 6 (sqllogictest, substrait, proto, ffi, spark, wasmtest companion crates) | 7 |
| **Total** | **37** | **38** | **35** | **104** | **215** |

The "other" column carries reasons that aren't strictly UI/CLI/backcompat per
Burak's three categories — they were promoted by individual agents because
including them would require dependencies/architecture that the embedded crate
deliberately doesn't have (e.g., Lucene segment storage in cave-search, Sentinel
HA daemon in cave-cache). **Phase-B should re-decide each `other` skip:** is it
genuinely out-of-scope, or is the agent over-eagerly excluding work?

---

## Phase-B priority queue — top 5 highest-leverage gaps per crate

These are the unmapped items whose absence breaks real upstream clients
hardest. Sourced verbatim from the per-crate agent reports.

### cave-pg
1. **`NegotiateProtocolVersion` (be-message)** — every modern psql/libpq client probes v3.1/v3.2 on connect; missing it forces hard-error or fallback.
2. **`AuthenticationRequest::SASL_PLUS` / `scram-sha-256-plus`** — channel-binding default for psql ≥17 over TLS.
3. **`pg_database` + `pg_authid` system catalogs** — `\l`, `\du`, ORM bootstrap probes (sqlx/pgx/JDBC) all hit these.
4. **`IDENTIFY_SYSTEM` / `START_REPLICATION`** — every CDC consumer (Debezium, pglogical, pg_dump --logical) opens with these. XLogData wire is mapped but unreachable without the verbs.
5. **`FunctionCall` + `FunctionCallResponse`** — legacy fastpath; absence makes unknown msg byte trip clients into protocol-error mode.

### cave-docdb
1. **`findAndModify`** — atomic upsert/replace; PyMongo / Mongoose / Spring Data uses it heavily.
2. **`bulkWrite`** — primary write path for modern drivers (4.4+); replaces sequential insert/update/delete.
3. **`explain`** — silent breakage in any ORM that calls explain on warmup or query-plan probing.
4. **`saslStart` / `saslContinue`** + SCRAM-SHA-256 mechanism — without SASL no driver with auth enabled connects.
5. **`$lookup` + `$unwind`** (agg-stage) — backbone of "joins" in real Mongo apps; absence breaks any non-trivial pipeline.

### cave-cache
1. **`FCALL` / `FCALL_RO` / `FUNCTION-LOAD`** — Valkey 7+ stored-procedures path; modern SDKs prefer this over EVAL.
2. **`SORT` / `SORT_RO`** — extremely common in Sidekiq-like clients; hard break.
3. **`TIME` / `ROLE`** — cheap heartbeat probes (redis-rb, lettuce health checks).
4. **`TOUCH`** — caching wrappers bump LRU without reading.
5. **`MIGRATE`** — required for cluster reshard tooling (`redis-cli --cluster`).

### cave-blob
1. **`DeleteObjects`** (multi-delete) — `aws-sdk` and `s3cmd sync --delete` rely on this for batch removal.
2. **`CopyObject` / `UploadPartCopy`** — server-side copy used by aws-sdk `Copy`, `mc cp` server-to-server, rename emulation.
3. **`ListBuckets`** — first call most SDKs make to verify creds/region.
4. **SignatureV4 streaming + `aws-chunked` parser** — chunked uploads are how official AWS SDKs PUT large objects by default.
5. **`GetBucketLocation` + `GetBucketVersioning`** — SDKs probe these on init for region routing and versioning awareness.

### cave-iceberg
1. **`POST .../tables/{table}/plan`** — server-side scan planning; modern PyIceberg/Spark fall back to local manifest planning without it.
2. **`GET .../tables/{table}/credentials`** — vended credentials; AWS/GCS/Azure clients require this when catalog brokers STS.
3. **`POST .../tables/{table}/sign`** — S3 request signer; clients with `s3.signer-uri` set fail without it.
4. **`POST .../register`** — registerTable, the standard catalog-migration pathway.
5. **`POST /v1/{prefix}/transactions/commit`** — multi-table atomic commit, required for engine CTAS+INSERT and overwrite-with-side-table flows.

> Honourable mention: existing `POST .../tables/{table}` commit handler is `partial` — accepts `(expected_metadata_location, new_metadata_location)` instead of upstream's `CommitTableRequest{requirements[], updates[]}`. Wire-incompatible.

### cave-search
1. **`bulk`** — every loader/exporter relies on `_bulk` ndjson; absence forces single-doc fallback.
2. **`msearch`** — multi-search batch path; dashboards and federated tools call it heavily.
3. **`info` (GET /) and `ping` (HEAD /)** — clients hit these on connect; missing them makes the client constructor error.
4. **`indices.get_mapping` / `indices.put_mapping`** — Kibana-style probes hit `/_mapping`; current crate only exposes `GET /:index`.
5. **`field_caps`** — Kibana/dashboards/federated discovery flows fail without it.

### cave-datafusion
1. **All Join executors** (HashJoinExec / NestedLoopJoinExec / SortMergeJoinExec / CrossJoinExec / SymmetricHashJoinExec) — local has zero join support; SQL parser also rejects multi-table FROM.
2. **`SortExec` + `ORDER BY`** — parser accepts but plan ignores; no `LogicalPlan::Sort` variant.
3. **`count_distinct` + `median` + `stddev`/`variance` + `percentile` aggregates** — common analytics queries fail; only COUNT/SUM/MIN/MAX/AVG (Int64-only) work.
4. **All scalar functions** (~93) — no UDF registry, no `coalesce`, `lower`, `substr`, `date_trunc`, `cast`, `now`, `concat`. Even `Expr::Cast` is missing.
5. **DDL/DML + CTEs + UNION** — no `CREATE TABLE` (only programmatic `register_table`), no INSERT/UPDATE/DELETE, no UNION/INTERSECT/EXCEPT, no WITH.

---

## Verifications of original prompt claims

| Claim | Result |
|---|---|
| 1 318 tests | ✅ verified — exactly 1 318 across 7 crates, all passing |
| 7 data-persistence crates green & buildable in isolation | ✅ verified per-crate `cargo test -p X` |
| `cargo test --workspace` clean | ❌ **FAILED** — pre-existing compile error in `cave-runtime` (missing `cave_portal_api` import in dep closure). Not introduced by this branch; not in the 7 target crates. |
| Existing manifests filled to 1.0 | ❌ **NOT TRUE** — prior to this audit, 3 manifests were 35-line stubs with all entries commented out, 4 didn't exist. Phase-A is the first time real items have been enumerated. |
| Scheduler conflict blocking merge | ❌ **NOT REPRODUCIBLE** — `git ls-files --unmerged` in main worktree (`hopeful-poincare-b0ede9`) returned empty. The main worktree has staged observability/qwen test changes (separate sprint), not a UU merge state. |
| `rg "todo!\|unimplemented!\|TODO\|FIXME"` in 7 crates | ✅ 0 occurrences across `crates/cave-{pg,docdb,cache,blob,iceberg,search,datafusion}/src` |

The two negative results matter for any future merge plan: the workspace
compile error must be fixed independently, and the assumed scheduler conflict
needs to be re-checked at merge time (it was not present today).

---

## Manifest schema notes (divergences to fix in Phase-B)

Each agent followed its own brief, but four subtle divergences crept in:

1. **Status vocabulary.** Four crates use `status = "mapped" | "unmapped" | "skipped"`. Three add `"partial"` (cave-iceberg, cave-search, cave-datafusion). Cave-search uses `"missing"` instead of `"unmapped"`.
2. **Field name.** Cave-datafusion uses `mapped = "yes" | "no" | "partial"` instead of `status = "..."`.
3. **Skipped reason structure.** Some manifests use a single `reason = "<UI|CLI|backcompat>: ..."` string; iceberg/datafusion split the `[summary]` block into typed counters (`skipped_cli`, `skipped_ui`, `skipped_engine`).
4. **Summary counter format.** Cave-search omits `total/mapped/partial/missing/skipped/fill` keys entirely from `[summary]`, putting only `scope`/`goal`/`non_goals` text. Counts are inferable from `[[items]]` rows but not from a top-level summary.

**Phase-B Step 0** should normalize these to a single TOML schema before any
downstream tool tries to compute fill ratios programmatically. Suggested:
```toml
[summary]
total    = N
mapped   = N
partial  = N      # 0 if not used
unmapped = N      # standardize on "unmapped" (not "missing")
skipped  = N
fill     = (mapped + skipped) / total
[summary.skipped]
ui         = N
cli        = N
backcompat = N
other      = N    # with explanation per-item
[[items]]
upstream = "<repo>"
kind     = "<...>"
name     = "<symbol>"
ref      = "<upstream/path>"
status   = "mapped|partial|unmapped|skipped"
local    = "<path::symbol>"   # only if mapped/partial
reason   = "<text>"           # only if skipped
```

---

## What this baseline is *not*

- It is **not** a green light to claim "REAL %100" on any crate. cave-cache at 0.78 is closest; cave-datafusion at 0.077 is furthest.
- It is **not** an attempt to merge `feat/data-persistence-100-pct-sprint` to main. The merge premise (scheduler conflict) wasn't reproducible today and the workspace-wide build is independently broken.
- It does **not** edit any source under `crates/cave-*/src/`. Phase-B is a separate sprint.

## What this baseline *is*

- An honest, upstream-pinned, item-level enumeration of the 7 crates' parity gap.
- A reproducible artifact: every manifest names its upstream commit SHA, so re-running enumeration is deterministic.
- A prioritized work queue: 35 highest-leverage gaps (5 × 7 crates) ready to drive Phase-B sprint planning.
- A tested-green baseline (1 318 / 1 318) that any Phase-B work must keep green.

---

## Recommended Phase-B sequencing

Ordered by **(upstream commit-ready + highest-leverage / smallest-effort) ratio**:

1. **cave-pg**: `NegotiateProtocolVersion` (be-message), `pg_database` + `pg_authid` shim — fastest path to make psql `\l` / `\du` / ORMs happy. (~1–2 days)
2. **cave-blob**: `DeleteObjects`, `CopyObject`, `ListBuckets` — three concrete S3 ops that complete the aws-sdk smoke-test suite. (~1–2 days)
3. **cave-cache**: SORT/SORT_RO + TIME/ROLE/TOUCH — five very short Rust functions, large client-compat unlock. (~½ day)
4. **cave-search**: `_bulk` + `_msearch` + `info`/`ping` + `indices.get_mapping` — Kibana smoke-tests start passing. (~2–3 days)
5. **cave-iceberg**: schema-fix existing partial commit handler to upstream `CommitTableRequest{requirements[], updates[]}` — wire compatibility for *existing* clients before adding new endpoints. (~½ day)
6. **cave-docdb**: `bulkWrite` + SASL + `findAndModify` — substantial work, biggest driver-compat unlock. (~3–5 days)
7. **cave-datafusion**: HashJoinExec + ORDER BY + `count_distinct` — the SQL-engine block. (~1+ week, biggest scope of any single crate.)

Total: ~3 calendar weeks of focused engineering for Phase-B's first pass —
which still wouldn't push fill ratios anywhere near 1.0, but would close the
client-compatibility cliff for all 7 crates simultaneously.

The full path to a defensible "REAL %100" remains a multi-month effort. This
document is the honest map.
