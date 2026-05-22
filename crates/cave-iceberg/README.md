# cave-iceberg

Sovereign Apache Iceberg table-format reimplementation. Upstream:
[`apache/iceberg-rust`](https://github.com/apache/iceberg-rust) v0.9.1.

## Status

Read-side MVP — catalog, table metadata, manifests, snapshots, and
scan planning. The write path (transaction commits, data-file writer)
and the Avro on-disk format for manifests are deferred to
lakehouse-ray-2. See `parity.manifest.toml` for the full inventory.

## Quick start

```rust,no_run
use cave_iceberg::*;

# async fn run() -> Result<()> {
let cat = MemoryCatalog::new();
cat.create_namespace(&Namespace::new(NamespaceIdent::from_dot("analytics"))).await?;

let schema = Schema::builder()
    .with_field(NestedField::required(1, "id", Type::Primitive(PrimitiveType::Long)))
    .with_field(NestedField::optional(2, "name", Type::Primitive(PrimitiveType::String)))
    .build()?;
let metadata = TableMetadataBuilder::new()
    .location("s3://lake/analytics/events")
    .schema(schema)
    .build()?;

let table = cat.create_table(&TableIdent::from_dot("analytics.events"), metadata).await?;
let _scan = table.scan().select(["id", "name"]);
# Ok(()) }
```
