# ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001 — Data Persistence Multi-Upstream Konsolide Reimpl

**Status:** Proposed (Burak'ın "sanki bir şeyler daha vardı" notu — tam liste kesinleşmemiş, draft)
**Scope:** Cave Runtime (independent override; multi-upstream consolidation)
**Category:** Charter / Architecture (Layer 4 persistence)
**Decided draft:** 2026-04-25 (Burak Tartan)

## Context

`ADR-RUNTIME-UPSTREAM-MIRROR-001` default mirror = 1 Platform OSS → 1 Runtime crate. Persistence katmanı bu defaulttan **sapan** özel durum: Cave Runtime'da data persistence için **birden fazla upstream konsolide** reimpl ediliyor.

Mevcut crate'ler:
- `cave-pg` — PostgreSQL upstream (relational, ACID, OLTP)
- `cave-docdb` — MongoDB upstream (document, schemaless)
- `cave-cache` — Valkey upstream (in-memory, key-value, pub/sub)

Genişleme adayları (Burak'ın "sanki bir şeyler daha vardı" referansı için aday liste — kararlanacak):
- `cave-distsql` — **CockroachDB** (distributed SQL, Postgres wire-compatible) veya **TiDB** (MySQL+Postgres wire) veya **YugabyteDB** (Postgres wire) — multi-region OLTP
- `cave-tsdb` — **InfluxDB** veya **TimescaleDB** veya **VictoriaMetrics** — time series
- `cave-graph` — **Neo4j** veya **Dgraph** veya **JanusGraph** — graph
- `cave-search` — **Elasticsearch/OpenSearch** veya **Quickwit** veya **Tantivy** — full-text search
- `cave-iceberg` + `cave-datafusion` — **Apache Iceberg + DataFusion** (zaten kararlandı, data warehouse query layer)
- `cave-blobs` — **MinIO** veya **SeaweedFS** — object storage (S3-compatible)

## Decision (taslak — Burak finalize edecek)

Cave Runtime persistence katmanı çoklu upstream'i koruyarak konsolide edilir. Her workload tipi için ayrı crate ama **tüm crate'ler ortak primitive'leri (cave-kernel WAL, Raft, snapshot, transaction) paylaşır**.

### Mimari

```
┌─────────────────────────────────────────────────────────────────┐
│ Tenant uygulamaları                                             │
└─────────────────────────────────────────────────────────────────┘
         │ Wire protocols (Postgres / MongoDB / Redis / S3 / ...) │
         ▼
┌──────────────┬───────────────┬──────────────┬─────────────────┐
│ cave-pg      │ cave-docdb    │ cave-cache   │ cave-distsql    │
│ (PG wire)    │ (Mongo wire)  │ (Redis wire) │ (PG wire,       │
│              │               │              │  multi-region)  │
├──────────────┼───────────────┼──────────────┼─────────────────┤
│ cave-tsdb    │ cave-search   │ cave-blobs   │ cave-iceberg +  │
│ (Influx/TS)  │ (ES/Quickwit) │ (S3 wire)    │ cave-datafusion │
│              │               │              │ (analytics OLAP)│
└──────────────┴───────────────┴──────────────┴─────────────────┘
                          │ Shared primitives
                          ▼
┌─────────────────────────────────────────────────────────────────┐
│ cave-kernel: WAL + Raft + Snapshot + Transaction + Encryption  │
│              (PQC-ready, multi-tenant invariant)                │
└─────────────────────────────────────────────────────────────────┘
```

### Workload-data mapping (tipik)

| Workload | Crate | Wire |
|---|---|---|
| Tenant OLTP (small) | cave-pg | Postgres |
| Tenant OLTP (multi-region, high-scale) | cave-distsql | Postgres (CockroachDB-compat) |
| Tenant document store | cave-docdb | MongoDB |
| Tenant cache / session | cave-cache | Redis |
| Tenant time series (metrics, IoT) | cave-tsdb | InfluxQL veya Flux |
| Tenant search (logs, full-text) | cave-search | Elasticsearch |
| Tenant blob (images, files) | cave-blobs | S3 |
| Tenant analytics (warehouse, ML features) | cave-iceberg + cave-datafusion | SQL (Iceberg + Arrow) |
| Tenant graph (social, knowledge) | cave-graph | Cypher / Gremlin |

Tenant deployment'ında Crossplane XR ile gerekli olan crate'ler aktive edilir (her tenant'ın workload profiline göre).

## Açık sorular (Burak finalize edecek)

1. **`cave-distsql` upstream**: CockroachDB / TiDB / YugabyteDB — hangisi?
2. **`cave-tsdb` upstream**: InfluxDB / TimescaleDB / VictoriaMetrics — hangisi? (TimescaleDB Postgres extension olduğu için cave-pg'ye gömülebilir mi?)
3. **`cave-search` upstream**: Elasticsearch (Apache 2.0 ama Elastic license drift) / OpenSearch (Apache 2.0) / Quickwit (AGPL) / Tantivy (MIT, library)
4. **`cave-graph` upstream**: Neo4j (proprietary core) / Dgraph (Apache 2.0 ama Hypermode acquisition belirsiz) / JanusGraph (Apache 2.0)
5. **`cave-blobs` upstream**: MinIO (AGPL drift) / SeaweedFS (Apache 2.0) / Garage (AGPL) — license analizi gerek
6. **`cave-distsql` ile `cave-pg` arasındaki sınır**: Hetzner single-region'da cave-pg yeterli mi, yoksa her zaman cave-distsql mi?

## Consequences

### Positive
- Tek runtime, tüm persistence ihtiyaçları (zero-vendor-lock-in for tenants)
- Shared primitives (WAL, Raft, encryption) tek bakım hattı
- Wire compatibility: tenant migration zero-friction (mevcut PG/Mongo/Redis/S3 clients çalışır)
- Multi-tenant invariant her crate'te (cave-kernel layer)

### Negative
- 8+ crate bakım yükü
- Her wire protocol için sürekli upstream izleme
- Test surface çok büyük

### Risks
| Risk | Mitigation |
|---|---|
| Upstream license drift (MinIO AGPL gibi) | License monitoring quarterly; permissive alternatif backup'lı seçim |
| Wire protocol divergence (PG 17→18 breaking) | Adapter layer; pinned upstream test corpus |
| Workload-crate mapping yanlış (tenant cave-pg ihtiyacında cave-distsql kullanır) | Backstage Scaffolder template'ler ile rehberli seçim |

## Related
- ADR-RUNTIME-UPSTREAM-MIRROR-001 — Mirror prensibinin multi-upstream consolidation istisnası
- ADR-RUNTIME-STACK-001 — Layer 4 ekosistem
- ADR-MULTI-TENANT-001 — tenant_id her data layer'da
- ADR-RUNTIME-STREAMING-CONSOLIDATION-001 — kardeş ADR (streaming için aynı pattern)
- ADR-DATA-LAYER-001 (varsa) — cave-iceberg + cave-datafusion analytics

---
*Drafted by Sonnet, finalize ediyor Burak — 2026-04-25 ADR review session. Açık sorular yukarıda.*
