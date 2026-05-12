# cave-streams parity — 2026-05-12 audit

**Upstreams:** `apache/kafka 4.2.0` (Apache-2.0, primary) +
`apache/pulsar v4.2.0` (Apache-2.0, secondary). Per
ADR-RUNTIME-STREAMING-CONSOLIDATION-001, one Rust crate exposes
both wire protocols.

## Methodology

Standard cave-etcd pattern, dual-upstream. Inventory enumerates
the top-level Kafka packages (server/, controller/, common/,
clients/, log/, coordinator/, network/, raft/, streams/, connect/,
storage/, tools/) plus Pulsar's pulsar-broker / pulsar-common /
pulsar-client / pulsar-functions / pulsar-io. Each entry counts
once across both upstreams — when a cave file serves both wire
protocols, it's noted in the `note` field.

## Counts

| Bucket   | Count |
|----------|------:|
| Mapped   | 18 |
| Skipped  | 16 |
| Unmapped | 9 |
| **Total** | **43** |
| **fill_ratio** | **0.7907** |

## What lands in the inventory

* **Mapped (18)** covers every wire protocol surface (Kafka
  request/response + Pulsar binary framing), the broker dispatch,
  segment-log storage, log compaction, producer + idempotent
  producer, consumer + group coordinator + incremental rebalance,
  EOS transactions, topic partition metadata, ACLs + quotas, the
  Kafka Streams DSL stub, Connect REST API, schema registry,
  Pulsar admin REST, Pulsar dispatcher (Exclusive / Shared /
  Failover / Key_Shared), and cave-side shared concerns (tenant,
  unified cursor, mirror, compression).
* **Skipped (16)** covers Kafka CLI (kafka-topics.sh etc.),
  Gradle build, tests + benchmarks, docs + examples, Trogdor soak,
  Java-stdlib helpers, Pulsar non-Java client libraries, tiered
  storage backends, Pulsar Functions (maps to cave-pipelines),
  Bouncy Castle crypto.
* **Unmapped (9)** covers honest gaps: KRaft consensus (KIP-500),
  tiered storage (KIP-405), Connect worker runtime (REST-only
  today), Streams Processor API (DSL-stub-only), Share groups
  (KIP-932), Pulsar geo-replication, Pulsar transactions, the
  BookKeeper-backed managed ledger (cave-streams uses its own
  segment log), and Pulsar IO connectors.

## What this PR does NOT claim

* `fill_ratio = 0.7907` does NOT claim cave-streams is a drop-in
  replacement for Kafka 4.2 + Pulsar 4.2. The two upstreams add up
  to ~370K LOC of Java/Scala; the cave crate is 18K LOC of Rust.
  The ratio describes how many top-level packages are accounted
  for (mapped or honestly skipped), not feature coverage by LOC.
* The 9 unmapped entries — particularly KRaft and tiered storage —
  are real production-feature gaps. Single-node only today.
