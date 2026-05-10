# cave-streams

Kafka-compatible streaming engine with full wire protocol, Schema Registry, and Kafka Connect integration.

## Status

This crate is currently in a pre-open-source-launch phase. Feature parity with Apache Kafka is tracked via internal issue trackers and is not yet complete for production-grade deployments.

## Upstream

- (internal — no external upstream)

## Surface ported

- Full Kafka wire protocol implementation supporting v0.10.0 through v3.6.0.
- Binary and JSON serialization formats for efficient data transmission.
- Schema Registry client with Avro, JSON Schema, and Protobuf support.
- Kafka Connect framework for building source and sink connectors.
- Consumer group management with automatic rebalancing strategies.
- Producer batching and compression support for gzip, snappy, and lz4.
- Topic partition management with configurable replication factors.
- Offset tracking and commit log persistence for reliable delivery.
- Metadata caching and broker discovery for high availability.
- Integration with the cave-runtime sovereign Cloud OS infrastructure.

## Public API

- `KafkaClient`: Main entry point for connecting to Kafka clusters.
- `Producer`: Handles message publishing with configurable acks and retries.
- `Consumer`: Manages subscription, polling, and offset commits.
- `SchemaRegistryClient`: Interacts with the schema registry for validation.
- `ConnectWorker`: Base class for implementing Kafka Connect connectors.

## Tests

Unit tests cover protocol encoding and decoding logic extensively. Integration tests require a running Kafka cluster and are disabled by default in CI environments until launch.

## License

Apache-2.0

## See also

- [../cave-network](../cave-network)
- [../cave-storage](../cave-storage)
- [../cave-config](../cave-config)
