# cave-rdbms — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-19
**Upstream pin**: `postgres/postgres @ REL_16_0` (PostgreSQL 16.0)
**Crate root**: `crates/cave-rdbms/`

## Scope

cave-rdbms implements a **minimal Postgres-compatible RDBMS** server:

- PostgreSQL wire-protocol v3 (startup, simple-query, extended-query, error response, parameter status)
- SQL grammar — lexer / parser / planner / optimizer / executor
- In-memory storage engine with transactions + savepoints
- `information_schema` + `pg_catalog` system views
- Side HTTP admin API under `/api/rdbms/*` for cave-portal

It is **not** a full Postgres reimplementation. WAL, MVCC, replication,
autovacuum, triggers, FDW, extensions, full-text search, jsonb operators,
GIN/GiST/SP-GiST/BRIN indexes, COPY, LISTEN/NOTIFY, SSL+SCRAM,
row-level security, window functions, CTEs, materialised views, prepared
statement cache, and the plpgsql / plpython execution engines are all
explicitly **out of scope** for this crate and counted as `skipped` in the
manifest. They live (or will live) in `cave-rdbms-operator` (the CNPG +
PgBouncer control plane crate) or are part of the long-term roadmap.

## Inventory measurement

Hand-curated against the postgres `REL_16_0` source tree.

| Bucket   | Count | Examples                                                                           |
|----------|------:|------------------------------------------------------------------------------------|
| Mapped   |    31 | wire (pqcomm, pqformat, auth, elog), SQL (scan, gram, planner, optimizer),         |
|          |       | executor (execMain, nodeSeqscan, nodeModifyTable, nodeAgg, execExpr, spi),         |
|          |       | storage (xact, heap, pg_namespace, bufmgr), catalog (pg_type, pg_class)            |
| Partial  |     4 | planner (rule-based, no cost), optimizer (constant-folding only),                  |
|          |       | transaction (single-thread), executor (interpreted, no JIT)                        |
| Skipped  |    30 | WAL, replication, MVCC, autovacuum, partitioning, triggers, FDW, extensions,       |
|          |       | FTS, jsonb operators, GIN/GiST/SP-GiST/BRIN indexes, COPY, LISTEN/NOTIFY,          |
|          |       | SSL/SCRAM, RLS, window functions, CTEs, materialised views, VACUUM/CLUSTER/        |
|          |       | ANALYZE, plpgsql, plpython, foreign tables, generated columns, range/enum types,   |
|          |       | array operators, inheritance, archive_mode, bgwriter, checkpointer                 |
| Unmapped |     4 | extended-query prepared-statement cache, role/grant, `ON CONFLICT` (UPSERT),       |
|          |       | `INSERT … RETURNING`                                                               |
| **Total**|  **69** | |

- **fill_ratio  = (mapped + partial + skipped) / total = 65 / 69 = 0.9420**
- **honest_ratio = (mapped + skipped) / total             = 61 / 69 = 0.8841**

## 8-gate close-out

| # | Gate                              | Result | Evidence                                  |
|---|-----------------------------------|--------|-------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | 31/31 `src/**/*.rs` carry AGPL-3.0-or-later |
| 2 | `source_sha` pinned in manifest   | PASS   | `source_sha = "REL_16_0"`                 |
| 3 | `last_audit = "2026-05-19"`       | PASS   | `[parity].last_audit`                     |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly  |
| 5 | `fill_ratio >= 0.90`              | PASS   | 0.9420                                    |
| 6 | mapped + partial + skipped + unmapped == total | PASS | 31 + 4 + 30 + 4 = 69       |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/`                |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                 |

**Charter v2 verdict: 8/8 PASS.**

## Test coverage

`cargo test -p cave-rdbms --lib --tests` exercises:

- 49 upstream-named test mappings (lexer/parser/planner/executor/protocol/error)
- 9 close-out self-audit assertions (`tests/parity_self_audit.rs`)

## Next sweep (out of this close-out)

- `INSERT … RETURNING` + `ON CONFLICT (UPSERT)` — small executor extension
- Role/grant minimal viable (sufficient for cave-portal RBAC integration)
- Extended-query prepared-statement cache (KEEP-ALIVE for cave-rdbms-operator
  connection pooler)

These are tracked as `unmapped_count = 4` and will lift `honest_ratio`
to ~0.9420 when landed (parity with `fill_ratio`).
