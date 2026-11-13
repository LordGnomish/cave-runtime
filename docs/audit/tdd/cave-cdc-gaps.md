# TDD coverage audit — cave-cdc vs debezium/debezium-server @ v3.5.0.Final

- **cave crate:** `crates/data/cave-cdc`
- **upstream:** https://github.com/debezium/debezium-server @ `v3.5.0.Final`
- **upstream test symbols inventoried:** 188 (51 test files; `@Test` methods + profile annotations)
- **cave test fns:** 65 (`#[test]` across `tests/*.rs`)
- **Date:** 2026-05-30

## Scope note

`debezium-server` is the **sink / embedded-engine delivery layer** of Debezium: its
test suite is overwhelmingly `*IT.java` Quarkus/Testcontainers **integration tests**
that spin up real Postgres/MySQL + a real downstream broker (Kafka, Kinesis, Pulsar,
PubSub, RabbitMQ, Redis, NATS, EventHubs, Milvus, Qdrant, Pravega, RocketMQ, SQS,
Infinispan, InstructLab) and assert end-to-end delivery. cave-cdc is the **portable
in-process CDC core** (connector state machines, WAL/binlog/oplog position tracking,
snapshot/signal/schema-history bookkeeping, topic routing, an abstract `SinkBackend`).

The mapping is therefore: every broker-specific `*IT` is **scope-cut** (vendor sink /
infra / Testcontainers), and the only *portable behavioral units* are the handful of
pure-logic JUnit `*Test.java` files (batching, routing-key derivation, schema mapping,
config props). Those are what this audit grades against cave's source + tests.

## Behavior table

| behavior | upstream test | cave impl? | cave test? | gap type | suggested test |
|---|---|---|---|---|---|
| Multi-destination batch fan-out (600+600 records → correct per-destination counts) | `KinesisUnitTest.testBatchesAreCorrect` | yes — `StreamsSink::dispatch_batch` + `MemorySink::count_for` | partial — only `dispatch_batch_short_circuits_on_first_failure` (error path) | **portable-coverage** | success-path `dispatch_batch` over records for two distinct tenant-prefixed topics; assert per-topic/partition counts and monotonic offsets |
| Batch splitting / large batch produces all records | `KinesisUnitTest.testBatchSplitting` | yes — `StreamsSink::dispatch_batch` loops all records | no | **portable-coverage** | dispatch_batch of N records to one topic; assert `results.len()==N` and contiguous `base_offset` 0..N |
| Empty batch handled without error | `KinesisUnitTest.testEmptyRecords` | yes — `dispatch_batch(&[])` returns `Ok(vec![])` | no | **portable-coverage** | `dispatch_batch(&[])` returns `Ok` empty vec, no backend writes |
| Routing key = "topic" → key equals destination topic | `RabbitMqStreamChangeConsumerTest.testHandleBatch_TopicRoutingKeySource` | yes — `TopicRouter::topic_for_change` (canonical 4-part topic) | yes — `schema_table_policy_emits_canonical_four_part_topic` | covered | — |
| Routing key = "static" / "key" sources | `RabbitMqStreamChangeConsumerTest.testHandleBatch_{Static,Key}RoutingKeySource` | no — cave has no static/key routing-key override (RabbitMQ-specific AMQP concept) | no | scope-cut (vendor AMQP routing-key semantics) | — |
| Topic partition assignment is deterministic for a key | (implicit in sink IT delivery) | yes — `TopicRouter::partition_for` | yes — `partition_for_is_stable_and_tenant_isolated` | covered | — |
| Invalid topic segments rejected | (implicit, Debezium topic naming) | yes — `topic_for_change` validation | yes — `routing_rejects_invalid_segments` | covered | — |
| Resend only failed records on retry | `KinesisUnitTest.testResendFailedRecords{,Successive}` | no — `SinkBackend` has no partial-failure/retry contract (cave produce is all-or-error) | no | scope-cut (AWS SDK partial-failure retry; cave delegates retry to backend) | — |
| Valid-response-with-error-code / exception-while-writing | `KinesisUnitTest.testValidResponseWithErrorCode`, `testExceptionWhileWritingData` | no — vendor SDK error decoding | no | scope-cut (Kinesis client response semantics) | — |
| Schema BACKWARD/FORWARD/FULL compatibility checks | `MilvusSchemaTest.*`, `QdrantMessageFactoryTest.*` (vector-store schema mapping) | yes — `Schema::check_backward/forward/full` | yes — `backward_compat_*`, `forward_compat_*`, `full_compat_*` | covered | — |
| Registry register increments version on compatible evolution | (Confluent registry semantics, exercised via `DebeziumServerWith*RegistryIT`) | yes — `SchemaRegistry::register` | yes — `registry_register_increments_version_on_compatible_evolution` | covered (happy path) | — |
| Registry **rejects** an incompatible evolution (error branch of `register` → `check_compat`) | (registry IT; no portable unit) | yes — `register` propagates `check_compat` Err | **no — test reaches the branch but `let _ = bad;` never asserts** | **portable-coverage** | register a BACKWARD-incompatible schema (drop a required field's type / make required→removed) under `Compatibility::Backward`; assert `register` returns `Err` and `version_count` stays unchanged |
| Compatibility NONE accepts anything | (registry config) | yes — `check_compat` None arm | yes — `compatibility_none_disables_all_checks` | covered | — |
| Mapping value from constant/header/field | `MappingValueTest.*` (InstructLab) | no — InstructLab QnA mapping is a vendor sink transform | no | scope-cut (InstructLab sink) | — |
| QnA file create/append | `QnaFileTest.*` (InstructLab) | no | no | scope-cut (InstructLab file format) | — |
| Qdrant message factory (point/vector construction) | `QdrantMessageFactoryTest.*` | no | no | scope-cut (vector-store sink) | — |
| HTTP consumer send / IOException retry / GOAWAY | `HttpChangeConsumerTest.*`, `HttpIT.testRetryUponError` | no — HTTP sink not a cave backend | no | scope-cut (HTTP sink + JWT/webhook auth) | — |
| JWT / Standard-Webhooks authenticator build + sign | `JWTAuthenticator*Test`, `StandardWebhooksAuthenticator*Test` | no | no | scope-cut (HTTP sink auth) | — |
| Redis memory-threshold / OOM retry / heartbeat skip | `RedisMemoryThresholdTest`, `RedisStream*IT` | no | no | scope-cut (Redis sink backpressure) | — |
| Config props parsing / JSON serialization smoke | `DebeziumServerTest.testProps`, `testJson` | partial — cave config is typed structs, no `.properties` ingestion | no | scope-cut (Quarkus config plumbing) | — |
| Connector signals via signal table | `DebeziumServerIT.testDebeziumServerSignals` | yes — `SignalTable::push/drain/was_seen` | yes — `signal_table.rs` (6 tests) | covered | — |
| Metrics exposure | `DebeziumServerIT.testDebeziumMetricsWithPostgres` | no — metrics owned by cave-metrics/observability | no | scope-cut (obs stack) | — |
| Postgres/MySQL/Mongo position & resume tracking | (driven by `*IT` delivery, no portable unit) | yes — `PostgresConnector::flush_lsn`, `MySqlConnector::record_position`, `MongoDbConnector::record_resume_token` | yes — `postgres_streaming.rs`, `mysql_binlog.rs`, `mongo_oplog.rs` | covered | — |
| Snapshot mode parse / chunk progress | (driven by `*IT`) | yes — `SnapshotMode::parse`, `SnapshotProgress::complete_chunk` | yes — `snapshot.rs` (5 tests) | covered | — |
| Schema-history record/replay | `RedisSchemaHistoryIT` | yes — `SchemaHistory::record/records_for_table/records_since_ts` | yes — `schema_history.rs` (7 tests) | covered | — |
| Outbox event routing + dedupe | (Debezium outbox SMT) | yes — `OutboxEventRouter::route/seen/forget` | yes — `outbox.rs` (5 tests) | covered | — |
| Offset store get/set/delete + tenant isolation | `RedisOffsetIT` | yes — `OffsetStore::set_checked/get/delete` | yes — `offset_store.rs` (8 tests) | covered | — |

## Recommended TDD fills (portable-coverage first)

These exercise cave public fns that are **implemented and source-verified but lack a
direct assertion**. No new behavior — close the coverage gap only.

1. **`cave_cdc::streams_sink::StreamsSink::dispatch_batch` (success path)** — mirror
   `KinesisUnitTest.testBatchesAreCorrect`: build a batch of valid records across two
   distinct tenant-prefixed topics, call `dispatch_batch`, assert it returns
   `Ok(Vec<ProduceResult>)` of the right length and that `MemorySink::count_for`
   reports the correct per-topic/partition counts. Currently only the *error*
   short-circuit branch of `dispatch_batch` is asserted.

2. **`cave_cdc::streams_sink::StreamsSink::dispatch_batch` (empty + split)** — mirror
   `KinesisUnitTest.testEmptyRecords` / `testBatchSplitting`: assert
   `dispatch_batch(&[])` is `Ok(vec![])` with zero backend writes, and that an
   N-record same-topic batch yields contiguous `base_offset` 0..N.

3. **`cave_cdc::schema::SchemaRegistry::register` (rejection branch)** — the existing
   `schema.rs` test reaches the incompatible-evolution call but discards the result
   (`let _ = bad;`). Add an assertion: under `Compatibility::Backward`, registering a
   schema that drops a required field returns `Err`, and `version_count` is unchanged.
   This is the only assertion gap on `register`'s `check_compat` error propagation.

### Honest note

cave-cdc's portable surface is well-covered (65 test fns vs a mostly-integration
upstream suite). Only **3** real portable-coverage gaps exist, all on the
`dispatch_batch` success path and the `register` rejection branch. Everything else is
either already asserted or a legitimate vendor-sink / infra / Quarkus-config scope-cut.
