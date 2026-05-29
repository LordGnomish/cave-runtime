# cave-datafusion — fresh-implementation coverage audit

- **Crate:** `cave-datafusion` (`/Users/gnomish/Code/cave-runtime-main/crates/data/cave-datafusion`)
- **Upstream:** [apache/datafusion](https://github.com/apache/datafusion)
- **Target tag:** `53.1.0`
- **Commit SHA:** `eae7bf4fa1c037c0a065d1f36d0669f5bb97a9cf`
- **Upstream license:** Apache-2.0 (line-port compatible with AGPL-3.0-or-later)
- **Port policy:** line-port
- **Audit date:** 2026-05-29

## Summary

cave-datafusion is a small, honestly-scoped MVP that carries the *shape* of the
DataFusion engine: a `LogicalPlan`/`LogicalExpr` AST, a hand-rolled SQL-subset
parser, a `PhysicalPlan`/`PhysicalExpr` AST, and a **row-at-a-time** executor
over a tiny `Value` enum (no Arrow). It genuinely executes
SELECT/WHERE/projection/GROUP-BY/ORDER-BY/LIMIT/inner-hash-join/cross-join
end-to-end against in-memory `MemTable`/`CsvSource` providers.

DataFusion proper is a ~40-crate workspace (common, expr, optimizer,
physical-expr, physical-plan, physical-optimizer, sql, sql-unparser, catalog,
datasource-{csv,json,parquet,avro,arrow}, functions{,-aggregate,-nested,-window,
-table}, substrait, proto, pruning, …). The cave crate covers the *plan-AST and
naive-execution* slice and explicitly defers everything else via `scope_cuts`.
The biggest honest gaps: **no optimizer of any kind** (no logical rule
framework, no physical optimizer, no expression simplification/CSE), **no Arrow
vectorization**, **window functions exist but are not wired into the plan**,
**DML has no execution path**, joins beyond inner/cross are declared but not
executed, and no serialization (proto/substrait) or `information_schema`.

## Coverage matrix

| Upstream module | Capability | Cave module | Status | Notes |
|---|---|---|---|---|
| `datafusion-common/src/{datatype,column,dfschema}.rs` (re-exports arrow) | Schema / DataType / Field model | `schema.rs` | PARTIAL | 10 flat primitive types only; no nested/list/struct/decimal-precision/timestamp-tz; no `DFSchema` qualified columns, no functional dependencies. |
| `datafusion-common/src/scalar/` | ScalarValue (full Arrow type lattice) | `row.rs` (`Value`) | PARTIAL | 6 variants (Null/Bool/Int32/Int64/Float64/Utf8). No Int8/16/UInt*/Date/Time/Timestamp/Interval/Decimal/List/Struct/Binary; sum/avg force Float64. |
| `datafusion-expr/src/expr.rs` | Logical expression AST | `logical_expr.rs` | PARTIAL | Column/Literal/Binary/Not/IsNull/Cast/Function/Alias only. Missing: Between, InList, Like/ILike, Case, ScalarSubquery, exists, negative, GROUPING SETS, window-expr, sort-expr, placeholders, wildcards-with-qualifier. |
| `datafusion-expr/src/logical_plan/plan.rs` | Logical plan AST | `logical_plan.rs` | PARTIAL | 9 nodes (Scan/Projection/Filter/Aggregate/Sort/Limit/Join/Union/Empty). Missing: Window, Distinct, SubqueryAlias, Subquery, RecursiveQuery, Values, Explain, Analyze, Repartition, Unnest, Statement/DDL nodes, CrossJoin-as-node. |
| `datafusion-expr/src/type_coercion/` | Type-coercion rules for binary/agg/scalar | (none) | MISSING | No coercion engine; arithmetic just calls `as_f64()`, comparisons coerce ad-hoc via `cmp_nulls_first`. |
| `datafusion-expr/src/window_frame.rs`, `window_state.rs` | Window frame spec (ROWS/RANGE/GROUPS, bounds) | `window.rs` (`Frame`) | MISSING | `Frame{start,end}` is a bare struct never used; no UNBOUNDED/PRECEDING/FOLLOWING/CURRENT-ROW, no RANGE vs ROWS semantics. |
| `datafusion-functions-window/` | Window functions (row_number, rank, lead/lag, ntile, nth_value, cume_dist) | `window.rs` | PARTIAL | row_number/rank/dense_rank/lag/lead/first_value/last_value/ntile implemented as pure slice fns, **but not wired into any plan node or executor** — unreachable from SQL/DataFrame. Missing cume_dist, percent_rank, nth_value. |
| `datafusion-sql/src/parser.rs` + `planner.rs` (wraps sqlparser-rs) | SQL → LogicalPlan | `sql_parser.rs` + `context.rs::sql_to_plan` | PARTIAL | Single-table SELECT subset (DISTINCT flag parsed but ignored; WHERE/GROUP BY/ORDER BY/LIMIT/OFFSET). No JOIN, no subquery, no CTE/WITH, no UNION/INTERSECT/EXCEPT, no HAVING, no qualified `t.col`, no `*` expansion (carried as sentinel), no DDL, no multi-statement, no parameters. |
| `datafusion-sql/src/statement.rs` (DML/DDL planning) | INSERT/UPDATE/DELETE/CREATE planning | `dml.rs` | PARTIAL | `DmlPlan` builder types + arity validation only; **no SQL parsing into DML and no execution** — the row engine cannot mutate a `MemTable`. No CREATE TABLE/VIEW/SCHEMA, no COPY. |
| `datafusion-sql/src/unparser/` | LogicalPlan → SQL string | (none) | MISSING | No unparser at all. |
| `datafusion-optimizer/src/optimizer.rs` + rules | Logical optimizer (push-down filter/limit, eliminate joins, decorrelate, propagate empty, single-distinct-to-groupby, etc.) | (none) | MISSING | No `OptimizerRule` trait, no rule pipeline, zero rewrite passes. `LogicalPlan` is executed as-built. |
| `datafusion-optimizer/src/analyzer/` | Analyzer pass (type coercion, count-wildcard, inline-tablescan) | (none) | MISSING | No analyzer phase. |
| `datafusion-optimizer/src/common_subexpr_eliminate.rs`, `simplify_expressions/` | CSE + constant-folding / expr simplification | `context.rs::lower_expr` (const-eval of scalar fns w/ literal args) | MISSING | Only eager constant-fold of a scalar function whose args are all literals during lowering; no general simplification, no CSE, no const-folding of arithmetic. |
| `datafusion-physical-expr/src/expressions/` | Physical expression eval | `physical_expr.rs` | PARTIAL | Column/Literal/Binary/Not/IsNull/IsNotNull/Cast eval row-at-a-time. No Case/InList/Like/Between/scalar-fn-call/window/coercion; no columnar (Arrow) evaluation. |
| `datafusion-physical-expr/src/equivalence/`, `sort_properties` | Equivalence classes / sort properties / ordering | `physical_plan.rs::SortPhysical` | MISSING | No equivalence-class or ordering-property tracking; sort just re-sorts. |
| `datafusion-physical-plan/src/aggregates/` | Hash + streaming aggregation, grouping sets, spilling | `physical_plan.rs::Aggregate` + `Accumulator` | PARTIAL | Single hash-aggregate over BTreeMap with 5 hard-coded accumulators (count/sum/avg/min/max). No partial/final split, no grouping sets, no DISTINCT-agg, no spill, no streaming-agg, no UDAF; sum/avg coerce to Float64. |
| `datafusion-physical-plan/src/joins/` (hash, sort-merge, nested-loop, symmetric, outer, semi/anti, mark) | Join operators | `physical_plan.rs::{HashJoin,CrossJoin}` | PARTIAL | Inner single-key hash join + cross join only. `JoinKind` enum lists Left/Right/Full/Semi/Anti but the lowerer/executor only ever emit Inner-hash or Cross — outer/semi/anti silently become inner. No sort-merge, no multi-key, no join filter. |
| `datafusion-physical-plan/src/sorts/` | External merge sort, TopK | `physical_plan.rs::Sort` | PARTIAL | In-memory `Vec::sort_by` with nulls-first/last handling. No external/spilling sort, no TopK fast-path, no merge of pre-sorted streams. |
| `datafusion-physical-plan/src/{limit,projection,filter,coalesce_batches}.rs` | Limit/Projection/Filter operators | `physical_plan.rs::{Limit,Projection,Filter}` | COVERED | Row-at-a-time skip/take, project expr list, predicate filter all genuinely execute. (Note: no batch coalescing since no Arrow.) |
| `datafusion-physical-plan/src/repartition/`, `coalesce_partitions.rs` | Partitioning / parallel exec | (none) | MISSING | Engine is single-partition; no `Partitioning`, no repartition, no parallelism. |
| `datafusion-physical-plan/src/streaming.rs`, `stream.rs` (SendableRecordBatchStream) | Streaming/async execution model | `physical_plan.rs::ExecutionPlan::execute -> Vec<Row>` | MISSING | Execution is synchronous-collect-to-Vec; no stream, no backpressure, no async pull. |
| `datafusion-physical-optimizer/` (enforce_distribution, enforce_sorting, join_selection, filter/limit/projection pushdown) | Physical optimizer | (none) | MISSING | No physical optimizer pipeline at all. |
| `datafusion-catalog/src/{catalog,schema}.rs` | Catalog/Schema provider hierarchy | `catalog.rs` (`SessionCatalog`) | PARTIAL | Single flat `name→TableProvider` map with register/deregister/list. No CatalogProviderList → CatalogProvider → SchemaProvider hierarchy, no default catalog/schema namespacing. |
| `datafusion-catalog/src/information_schema.rs` | `information_schema` virtual tables | (none) | MISSING | No information_schema (tables/columns/df_settings/views/routines). |
| `datafusion-catalog/src/{view,cte_worktable,listing_schema}.rs` | Views / CTE work table / listing | (none) | MISSING | No view registration, no recursive-CTE work table. |
| `datafusion-catalog/src/table.rs` (`TableProvider`) | Storage adapter trait | `data_source.rs::TableProvider` | PARTIAL | `schema()` + `async scan() -> Vec<Row>`. Missing supports_filters_pushdown, scan-with-projection/limit, statistics, insert_into, table_type. |
| `datafusion-datasource-csv/` | CSV source (split, schema-infer, compression, streaming) | `data_source.rs::CsvSource` | PARTIAL | `from_str` only: header-name check, naive `split(',')`, per-cell coerce-or-null. No quoting/escaping/RFC-4180, no delimiter config, no schema inference, no file/object-store IO, no streaming. |
| `datafusion-datasource-json/` | NDJSON source | (none) | MISSING | lib.rs doc mentions JSON provider but none exists in `data_source.rs`. |
| `datafusion-datasource-parquet/`, `-avro/`, `-arrow/` | Parquet/Avro/Arrow file sources | (none) | MISSING | Deferred to "lakehouse-ray-2". |
| `datafusion-common/src/{stats,pruning}.rs`, `datafusion-pruning` | Statistics + predicate pruning | (none) | MISSING | No `Statistics`, no `PruningPredicate`, no row-group/partition pruning. |
| `datafusion/src/dataframe/mod.rs` | DataFrame fluent builder | `dataframe.rs` | PARTIAL | select/filter/aggregate/sort/limit/join/union builders. No `collect`/`show`/`write_*`, no `with_column`, no `distinct`, no `explain`, no `cache`; `schema()` returns input schema (output shape only known after exec). |
| `datafusion/src/execution/context/mod.rs` + `session_state.rs` | SessionContext / SessionState / config | `context.rs::SessionContext` | PARTIAL | new/register_table/table/sql/sql_to_plan/execute_plan. No SessionConfig/runtime-env/memory-pool/disk-manager, no register_csv/parquet/listing, no `read_*`, no UDF/UDAF/UDWF registration surface, no `state()`. |
| `datafusion-functions/` (scalar built-ins: math, string, datetime, regex, crypto, encoding) | Scalar function library | `functions.rs` | PARTIAL | 9 scalars (abs/coalesce/length/lower/upper/concat/round/least/greatest). Missing the hundreds of upstream scalars; `round` ignores its precision arg; no datetime/regex/crypto/array. |
| `datafusion-functions-aggregate/` | Aggregate function library (sum/avg/count/min/max/stddev/var/corr/approx_*/array_agg/first/last/string_agg) | `functions.rs::AggregateKind` + `physical_plan.rs::Accumulator` | PARTIAL | 5 aggregates hard-coded as an enum (no `AggregateUDF` trait, no extensibility). Missing stddev/variance/covariance/correlation/approx_*/array_agg/string_agg/bit_and_or_xor/bool_and_or/median/percentile/grouping. No DISTINCT, no FILTER, no ORDER-BY-within-agg. |
| `datafusion-functions-nested/` | Array/map/struct functions | (none) | MISSING | No nested types, so no nested functions. |
| `datafusion-expr/src/{udf,udaf,udwf}.rs` + `registry.rs` | User-defined function framework | `functions.rs::FunctionRegistry` (scalar only) | PARTIAL | Can register a scalar `Fn(&[Value])->Value`; no UDAF/UDWF traits, no return-type/signature/volatility metadata, no async UDF. |
| `datafusion-proto/`, `-proto-common/` | Protobuf plan/expr serialization | (none) | MISSING | No proto serde. (Plan/Expr derive serde::Serialize for JSON but no wire-format.) |
| `datafusion-substrait/` | Substrait producer/consumer | (none) | MISSING | No Substrait. |
| `datafusion-ffi/` | C ABI / FFI table & plan | (none) | MISSING | No FFI. |
| `datafusion/src/physical_planner.rs` (PhysicalPlanner) | LogicalPlan → PhysicalPlan lowering | `context.rs::lower_inner` + `lower_expr` | PARTIAL | Direct 1:1 lowering of the 9 supported nodes. No ExtensionPlanner, no expr-to-physical for Case/InList/window, no aggregate partial/final placement, no repartition insertion; Union/EmptyRelation lower to an error. |
| `datafusion-spark/` | Spark-compatible function semantics | (none) | MISSING | Out of scope. |

## Actionable gaps for strict-TDD

Ordered roughly lowest-effort / highest-value first.

### 1. Wire window functions into execution (PARTIAL → COVERED)
- **Upstream ref:** `datafusion/physical-plan/src/windows/`, `datafusion-functions-window/src/row_number.rs`, `rank.rs`
- The `window.rs` slice functions are correct but unreachable: no `LogicalPlan::Window`, no `PhysicalPlan::Window`, and `parse_sql` cannot parse `OVER (...)`.
- **Failing test idea:** `sql_window_row_number_over_partition`
  - `ctx.register_mem_table` with a `(g, v)` table, run `SELECT g, row_number() OVER (PARTITION BY g ORDER BY v) FROM t`, assert per-group sequence `[1,2,...]`. Currently `parse_sql` errors on `OVER`.

### 2. Execute outer / semi / anti joins (PARTIAL → COVERED)
- **Upstream ref:** `datafusion/physical-plan/src/joins/hash_join.rs` (JoinType handling), `nested_loop_join.rs`
- `JoinKind::{Left,Right,Full,Semi,Anti}` exist but `context.rs::lower_inner` only ever emits `HashJoin`(inner) or `CrossJoin`; outer/semi/anti silently produce inner-join rows.
- **Failing test idea:** `left_join_keeps_unmatched_left_rows_with_nulls`
  - Build a left table with a key absent from the right; lower a `Join{kind:Left,...}`; assert the unmatched left row is emitted with right-side columns NULL. Currently the row is dropped (inner semantics).

### 3. Correct numeric typing for SUM / fix round() precision arg (PARTIAL)
- **Upstream ref:** `datafusion-functions-aggregate/src/sum.rs`, `datafusion-functions/src/math/round.rs`
- `Accumulator::finalize` returns `Float64` for `Sum`/`Avg` even when all inputs are `Int64`; `round` ignores a second precision argument.
- **Failing test idea:** `sum_of_int64_column_stays_int64`
  - Aggregate `Sum` over `[Int64(10),Int64(20)]`; assert result is `Value::Int64(30)`, not `Value::Float64(30.0)`. Also `round_with_precision_arg`: `round(3.14159, 2)` → `Value::Float64(3.14)`.

### 4. Add a logical filter-pushdown optimizer rule (MISSING)
- **Upstream ref:** `datafusion-optimizer/src/push_down_filter.rs`, `optimizer.rs` (`OptimizerRule` trait)
- There is no optimizer phase whatsoever; introduce an `OptimizerRule` trait + a `push_down_filter` pass that moves a `Filter` below a `Projection`.
- **Failing test idea:** `push_down_filter_moves_filter_below_projection`
  - Build `Filter(predicate, Projection(exprs, Scan))`; run optimizer; assert the result is `Projection(exprs, Filter(predicate, Scan))`. No optimizer entry point exists yet, so this won't even compile against the public API.

### 5. Constant-folding / expression simplification (MISSING)
- **Upstream ref:** `datafusion-optimizer/src/simplify_expressions/expr_simplifier.rs`
- Only scalar-fn-with-literal-args is folded during lowering; arithmetic literals like `1 + 1` and boolean identities (`x AND true`) are not simplified.
- **Failing test idea:** `simplify_folds_constant_arithmetic`
  - Simplify `LogicalExpr` `lit(1) + lit(2)` → `lit(3)`, and `col(a) AND lit(true)` → `col(a)`. Requires a new `simplify(expr) -> expr` entry point.

### 6. Parse + execute DML (INSERT/UPDATE/DELETE) end-to-end (PARTIAL)
- **Upstream ref:** `datafusion-sql/src/statement.rs` (`sql_statement_to_plan`), `datafusion-catalog/src/memory/` (insert_into)
- `dml.rs` only builds/validates `DmlPlan`; `parse_sql` rejects non-SELECT, and the executor cannot mutate a `MemTable`.
- **Failing test idea:** `sql_insert_then_select_roundtrip`
  - `ctx.sql("INSERT INTO t (a,b) VALUES (1,'x')")` then `ctx.sql("SELECT * FROM t")` returns the inserted row. Currently `parse_sql` errors on the `INSERT` keyword and `TableProvider` has no insert path.
