# ADR-147 — Data Persistence Crate Naming + Lakehouse Consolidation

**Status:** Proposed — pending Burak approval
**Scope:** Cave Runtime
**Category:** Naming / Architecture
**Date:** 2026-05-02
**Related ADRs:** ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 (Kong+Gravitee→cave-gateway pattern), ADR-RUNTIME-STREAMING-CONSOLIDATION-001 (Kafka+Pulsar→cave-streams pattern), ADR-RUNTIME-UPSTREAM-MIRROR-001 (Platform OSS → Runtime crate mapping)

## Context

The data-persistence layer of `cave-runtime` has accumulated naming and topology debt that no longer matches the architectural pattern established by ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 and the streaming consolidation ADR. Three concrete misalignments:

### 1. `cave-pg` overloaded

The `cave-pg` crate carries two responsibilities that share a name but not a domain:

- **Connection pooling / operator concerns** — PgBouncer-style pool, instance registry, replication topology, backup/restore orchestration. The "operator" surface that a CNPG-style controller would expose.
- **(implicitly) Postgres engine surface** — but the actual Postgres TCP wire + planner + MVCC + WAL implementation already lives in `cave-rdbms` (5073 LOC, 80 tests, `protocol/` + `executor/` + `sql/` + `storage/`).

Today `cave-pg` (2184 LOC, 0 lib tests) is misnamed: it is *not* the Postgres engine. It is the Postgres operator. A reader looking for "where does the Postgres wire live?" lands in the wrong crate.

### 2. `cave-iceberg` and `cave-datafusion` solve one problem in two crates

A modern lakehouse stack is a single composed surface:

- A **table format** (Apache Iceberg, Delta Lake, Apache Hudi) that defines partition layout, schema evolution, snapshot isolation, time-travel.
- A **query engine** (DataFusion, Trino, Spark) that reads that format and serves SQL/DataFrame queries.
- A **columnar IO layer** (Parquet, ORC) that the format and engine share.
- An **object store** (MinIO, S3) under it all.

`cave-iceberg` (1986 LOC, 117 tests) implements the table-format layer. `cave-datafusion` (1921 LOC, 92 tests) implements the query-engine layer. They are not independently useful — every realistic lakehouse query path crosses both. Today they are two separate crates with no canonical aggregator, the way callers have to reach for both is opaque, and there is no place to put cross-cutting concerns (write paths, ACID across both, time-travel queries that need engine + format, MinIO-aware planning).

### 3. Pattern asymmetry with already-consolidated crates

`cave-streams` already collapsed Kafka + Pulsar into one Rust impl (ADR-RUNTIME-STREAMING-CONSOLIDATION-001). `cave-gateway` already collapsed Kong + Gravitee (ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001). Both ADRs cite the same justification: "two strong upstreams in the same architectural slot belong in one cave-native crate." The lakehouse stack — Iceberg + DataFusion + (future Delta/Hudi optional, Parquet IO, MinIO awareness) — is the same pattern, currently un-applied.

## Decision (proposed)

Rename and consolidate to two changes, both bookkeeping-only at the source level:

### 3.1 — `cave-pg` → `cave-rdbms-operator`

`cave-pg` is renamed to `cave-rdbms-operator`. The crate's surface is unchanged; its name now reflects the actual responsibility (CNPG-style operator + pool + topology) and its sibling relationship to `cave-rdbms` (the engine).

**Upstream re-target**: `cave-pg`'s parity manifest currently points at `pgbouncer/pgbouncer @ 1.21.0`. The rename should re-target to **`cloudnative-pg/cloudnative-pg`** (the actual operator the crate mirrors) plus a secondary `[[upstreams]]` entry for `pgbouncer/pgbouncer` (the connection-pooler subset).

### 3.2 — `cave-iceberg` + `cave-datafusion` → `cave-lakehouse`

A new `cave-lakehouse` crate absorbs both, mirroring the cave-streams / cave-gateway multi-upstream pattern:

```toml
[[upstreams]]
org = "apache"
repo = "iceberg-rust"
version = "0.4.0"
notes = "Table format — partition layout, snapshot isolation, time-travel."

[[upstreams]]
org = "apache"
repo = "datafusion"
version = "44.0.0"
notes = "Query engine — SQL planner, DataFrame, vectorized executor."

[[upstreams]]
org = "delta-io"
repo = "delta-rs"
version = "0.18.0"
notes = "Optional second table format. Iceberg is the primary; Delta is opt-in for Databricks-aligned workflows."

[[upstreams]]
org = "apache"
repo = "hudi-rs"
version = "0.2.0"
notes = "Optional third table format. Defer concrete impl until Iceberg + Delta are 100% parity."

[[upstreams]]
org = "apache"
repo = "arrow-rs"
version = "53.0.0"
notes = "Parquet columnar IO + Arrow in-memory representation. Shared between table-format read path and engine vectorized executor."

[[upstreams]]
org = "minio"
repo = "minio"
version = "RELEASE.2026-04-22"
notes = "Object store substrate. Lakehouse calls go through cave-store which mirrors MinIO; this entry exists so the lakehouse audit trail names MinIO explicitly."
```

Source layout (single crate, internal modules per upstream):

```
cave-lakehouse/
├── src/
│   ├── lib.rs
│   ├── table_format/      # Iceberg primary + Delta secondary
│   │   ├── iceberg/       # ex-cave-iceberg src/* moves here verbatim
│   │   └── delta/         # placeholder, opt-in
│   ├── engine/            # DataFusion-based planner + executor
│   │   └── datafusion/    # ex-cave-datafusion src/* moves here verbatim
│   ├── parquet_io/        # shared Arrow/Parquet read+write helpers
│   └── time_travel.rs     # cross-cutting: engine consumes format snapshots
└── parity.manifest.toml   # multi-upstream form per ADR-RUNTIME-UPSTREAM-MIRROR-001
```

The internal modules keep their existing 117 + 92 = **209 tests** intact; nothing functional moves. Only the crate boundary changes.

## Why these two and not more

Other data crates were considered and explicitly held out of this ADR:

- **`cave-rdbms`** stays — it is the Postgres engine, distinct domain from the operator. Keeping engine and operator in separate crates is *correct* per the ADR-RUNTIME-UPSTREAM-MIRROR-001 default ("1 OSS → 1 crate"). The rename of `cave-pg → cave-rdbms-operator` makes that separation explicit, it does not collapse it.
- **`cave-docdb`** stays — single-upstream (mongodb/mongo), no consolidation case yet.
- **`cave-cache`** stays — single-upstream (valkey-io/valkey), no consolidation case.
- **`cave-store`** stays — already the MinIO mirror; lakehouse calls *into* it, doesn't absorb it.

## Migration steps (mechanical)

1. **`cave-pg → cave-rdbms-operator`** (one rename):
   - `git mv crates/cave-pg crates/cave-rdbms-operator`
   - Update `Cargo.toml` `name = "cave-rdbms-operator"`, root workspace member path
   - One reverse-dep crate (per `grep -rln cave-pg crates/*/Cargo.toml`); update its dep + `use` imports
   - Re-target `parity.manifest.toml` upstream to `cloudnative-pg/cloudnative-pg`
   - Optional: leave a 2-line `crates/cave-pg/` shim that re-exports `pub use cave_rdbms_operator::*;` for one release cycle, then delete

2. **`cave-iceberg + cave-datafusion → cave-lakehouse`** (consolidate):
   - Create `crates/cave-lakehouse/` workspace member
   - `git mv crates/cave-iceberg/src/* crates/cave-lakehouse/src/table_format/iceberg/`
   - `git mv crates/cave-datafusion/src/* crates/cave-lakehouse/src/engine/datafusion/`
   - Combine `Cargo.toml` deps; expose multi-upstream form
   - Combine `parity.manifest.toml` (file mappings get `local = "src/table_format/iceberg/X.rs"` prefix update)
   - Remove the two old crates from workspace (their source moved, no orphan)
   - Update zero reverse-deps (both crates currently have 0 reverse-dep)

`cargo check --workspace` + `cargo test --workspace` verify each rename atomically. No source semantics change.

## Risks accepted

- **Rename churn** in any in-flight branches that touch `cave-pg` / `cave-iceberg` / `cave-datafusion` will conflict on path. Mitigation: time the rename for a quiet window; the audit (2026-05-01) already shows these three crates not in active sprint touch.
- **Parity dashboard** numbers reset for the consolidated crate — `cave-lakehouse` starts at 0 file/fn/test/surface entries until a manifest-fill pass. The 209 underlying tests still run; only the dashboard score blanks.
- **Out-of-scope**: this ADR does NOT touch public HTTP/gRPC routes, on-disk data layout, wire protocol, multi-tenant headers. It is a source-layer naming change.

## Decision (Burak)

- [ ] Approve `cave-pg → cave-rdbms-operator`
- [ ] Approve `cave-iceberg + cave-datafusion → cave-lakehouse`
- [ ] Approve both as a single landed PR
- [ ] Reject — keep current layout

Once any of these is checked, the agent that executes the rename references this ADR in the commit message:

```
refactor(workspace): cave-pg → cave-rdbms-operator (ADR-147)
refactor(workspace): consolidate cave-iceberg + cave-datafusion → cave-lakehouse (ADR-147)
```
