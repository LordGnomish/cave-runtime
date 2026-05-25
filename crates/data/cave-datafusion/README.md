# cave-datafusion

Sovereign Apache DataFusion query-engine reimplementation. Upstream:
[`apache/datafusion`](https://github.com/apache/datafusion) 53.1.0.

## Status

Read-side query MVP — LogicalPlan + LogicalExpr + DataFrame builder +
SQL parser/planner subset + row-at-a-time physical executor with
scan/filter/project/aggregate/sort/limit/join. CSV + in-memory
TableProviders. See `parity.manifest.toml` for the full inventory.

## Quick start

```rust,no_run
use cave_datafusion::*;
use std::sync::Arc;

# async fn run() -> Result<()> {
let ctx = SessionContext::new();

let schema: SchemaRef = Arc::new(TableSchema::new(vec![
    Field::new("g", DataType::Utf8, false),
    Field::new("v", DataType::Int64, false),
]));
ctx.register_mem_table(
    "t",
    schema,
    vec![
        Row::new(vec![Value::Utf8("x".into()), Value::Int64(10)]),
        Row::new(vec![Value::Utf8("x".into()), Value::Int64(20)]),
        Row::new(vec![Value::Utf8("y".into()), Value::Int64(5)]),
    ],
).await?;

let rows = ctx.sql("SELECT g, count(v) FROM t GROUP BY g ORDER BY g").await?;
# Ok(()) }
```
