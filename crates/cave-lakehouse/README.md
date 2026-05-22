# cave-lakehouse

High-performance lakehouse implementation using Apache Iceberg and DataFusion.

## Status

This crate is currently in the pre-open-source-launch phase. Feature parity with the upstream Apache Iceberg specification is actively tracked and implemented incrementally. Core functionality is stable, but edge cases in schema evolution and snapshot management are still under rigorous testing.

## Upstream

- [Apache Iceberg](https://iceberg.apache.org/)

## Surface ported

- Full support for Apache Iceberg table format v1 and v2 specifications.
- Integration with Apache DataFusion for SQL query execution and optimization.
- Efficient metadata management including snapshot, manifest, and schema tracking.
- Support for various storage backends via the `cave-storage` abstraction layer.
- Transactional guarantees for concurrent reads and writes using optimistic concurrency control.
- Automatic compaction of small manifest files to maintain query performance.
- Support for time-travel queries to access historical data states.
- Schema evolution capabilities including column addition, removal, and type changes.
- Partitioning strategies such as bucketing, range, and hash partitioning.
- Integration with the `cave-runtime` cloud OS for seamless deployment on sovereign cloud infrastructure.

## Public API

- `IcebergTable`: The primary struct representing an Iceberg table with methods for reading and writing data.
- `Snapshot`: Represents a specific point in time for the table, allowing for time-travel queries.
- `Schema`: Manages the table schema, including field definitions and evolution logic.
- `Transaction`: Handles atomic updates to table metadata and data files.
- `ManifestReader`: Reads manifest files to determine which data files are part of a snapshot.
- `QueryEngine`: Wraps DataFusion to execute SQL queries against Iceberg tables.

## Tests

Comprehensive unit tests cover metadata parsing, schema evolution, and transaction logic. Integration tests validate end-to-end workflows with real storage backends and DataFusion query execution. Coverage is maintained at a high level to ensure reliability in production environments.

## License

Apache-2.0

## See also

- [../cave-runtime](../cave-runtime)
- [../cave-storage](../cave-storage)
- [../cave-ql](../cave-ql)
