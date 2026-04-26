# ADR-DATA-LAKE-PATTERN-001 — Data Lake: ADLS Gen2 (Azure) / MinIO + Iceberg + DataFusion (Hetzner)

**Status:** Accepted
**Scope:** Universal+Hetzner+Azure (Platform)
**Category:** Data Lake / Analytics
**Decided:** 2026-04-25 (Burak Tartan)
**Related ADRs:** ADR-050 (Object Storage), ADR-008 (Cache), ADR-021 (Streaming), ADR-049 (Search), ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001

## Context

ADR-050 sadece **object storage** (S3 wire) düzeyinde MinIO + ADLS Gen2 ikilisini kararladı. Ancak **Data Lake** kavramı 3 katmanı kapsıyor:

1. **Object storage** — basic S3-uyumlu blob CRUD
2. **Hierarchical Namespace (HNS)** — atomic rename + posix permissions + nested directories
3. **Hadoop FileSystem driver** — Spark/Synapse/Databricks için `abfs://` veya `s3a://`

ADLS Gen2 üçünü de native sağlar; MinIO yalnızca object storage tarafını sağlıyor. Hetzner profilinde "ADLS Gen2 muadili" tam stack tanımlanmamıştı — bu ADR onu kapatır.

## Decision

CAVE Data Lake mimarisi **Iceberg-centric** olarak tanımlanır. Tenant uygulamaları her iki cloud'da da **Apache Iceberg API** üzerinden veri yazar/okur; underlying storage (MinIO veya ADLS Gen2) abstract edilir. Hierarchical namespace ihtiyacı Iceberg metadata layer'da karşılanır (table partitioning + nested namespaces).

### Stack katmanları

```
┌─────────────────────────────────────────────────┐
│ Tenant query (SQL / DataFrame)                  │
│   - SELECT … FROM warehouse.sales WHERE …       │
└────────────────────────┬────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────┐
│ Analytics engine                                │
│   Hetzner: DataFusion (cave-datafusion reimpl)  │
│   Azure:   Databricks Delta veya Synapse Spark  │
└────────────────────────┬────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────┐
│ Table format (Iceberg)                          │
│   Hetzner: Apache Iceberg (cave-iceberg reimpl) │
│   Azure:   Apache Iceberg (Databricks / Trino)  │
│   Common API: ACID + snapshots + schema evol.   │
└────────────────────────┬────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────┐
│ Object storage                                  │
│   Hetzner: MinIO (S3 wire)  → S3A driver        │
│   Azure:   ADLS Gen2 (HNS)  → ABFS driver       │
└─────────────────────────────────────────────────┘
```

### Hetzner stack özet

| Katman | Çözüm | Wire / API |
|---|---|---|
| Object storage | MinIO | S3 |
| Hierarchical organization | Iceberg metadata + partition spec | Iceberg API |
| Hadoop FS driver | S3A | hadoop://s3a/ |
| Analytics engine | DataFusion | SQL + Arrow |

### Azure stack özet

| Katman | Çözüm | Wire / API |
|---|---|---|
| Object storage | ADLS Gen2 | Blob + Data Lake API |
| Hierarchical namespace | ADLS HNS (native) | POSIX-like |
| Hadoop FS driver | ABFS | abfs:// |
| Analytics engine | Databricks Delta veya Synapse Spark | SQL + Spark |

### Tenant developer experience

- Tenant uygulamaları **Iceberg API** kullanır — underlying storage cloud-agnostic.
- Aynı SQL query her iki cloud'da çalışır.
- Cloud farkı sadece deployment YAML'ında: `storage.url = s3://...` (Hetzner) veya `storage.url = abfs://...` (Azure). Iceberg üstünde transparent.

## Rejected Alternatives

### MinIO Enterprise (HNS + Hadoop driver) — Rejected
Commercial license, sovereign zero-vendor-lock-in (ADR-066) ile çelişir. OSS muadiller yetersizse değerlendirilir; şu an ihtiyaç yok (Iceberg yeterli).

### JuiceFS — Considered, Deferred
Apache 2.0, S3 backend üzerine POSIX FS sağlıyor. ADLS HNS'e en yakın muadil. Iceberg+DataFusion stack'i tam analytics ihtiyacını karşıladığından şimdilik gerek yok. Eğer CAVE'e tenant olarak Hadoop ecosystem (HDFS API gerektiren) gelirse JuiceFS değerlendirilir. **Watch:** Q3 2026.

### Apache Ozone — Rejected
Hadoop ecosystem object storage. CAVE'in K8s-native + S3-uyumlu yaklaşımıyla zayıf uyum. MinIO daha hafif ve k8s-native.

### HDFS — Rejected
Eski school. Multi-node, complex ops, GRPC + Java stack. CAVE'in cloud-native vizyonuyla çelişir.

### SeaweedFS — Considered, Deferred
Distributed FS + S3 + POSIX. MinIO'dan daha çok feature ama daha az production-tested. Phase 2 değerlendirme.

## Consequences

### Positive
- Tek developer experience (Iceberg API her iki cloud'da)
- Iceberg ACID + schema evolution + time travel — modern data lake özellikleri
- DataFusion sayesinde Hetzner'da Spark cluster gereksiz (Rust-native, K8s-friendly)
- Cloud migration zero-friction (sadece storage URL değişiyor)
- Analytics workload sovereignty (Hetzner) ve enterprise (Azure) profillerinde uniform

### Negative
- Hetzner'da Hadoop ecosystem zayıf (Spark MapReduce gerektiren legacy workload'lar zorlanır — DataFusion modern alternative ama eski Spark code'u port etmek gerekebilir)
- ADLS Gen2'nin native HNS özelliklerinden (atomic rename, posix perms) Iceberg yararlanmaz — bazı ops faydaları kaçırılır
- Iceberg metadata maintenance (compaction, snapshot expiry) ek operasyonel iş

### Risks
| Risk | Mitigation |
|---|---|
| Iceberg upstream divergence (1.x → 2.x breaking) | Pin Iceberg version. cave-iceberg upstream parity testleri. |
| MinIO AGPL license drift (2024 sonrası) | License monitoring quarterly. SeaweedFS hazır backup. |
| DataFusion performance gap vs Spark on big workloads | Benchmark per-tenant. Spark fallback option Hetzner profilinde mümkün. |
| Tenant uygulaması abfs://-only (cloud-specific) | Document as "tenant code uses Iceberg API only, no abfs:// hardcode". |

## Mirror — Runtime tarafı

Bu pattern Mirror prensibi gereği Runtime'da otomatik:
- `cave-blobs` (MinIO upstream-reimpl) — S3 wire
- `cave-iceberg` (Apache Iceberg upstream-reimpl) — Iceberg API
- `cave-datafusion` (DataFusion upstream-reimpl) — Arrow + SQL

Azure tarafı (ADLS Gen2, Databricks, Synapse) Cave Runtime tarafında **reimpl edilmez** (SaaS managed). Cave Runtime self-contained — kendi data lake stack'ini sağlar (cave-blobs + cave-iceberg + cave-datafusion).

## Compliance Mapping
SOC2 CC6.1 (data access controls — Iceberg ACL + bucket IAM). ISO A.5.12 (data classification — Iceberg partition strategy). GDPR Art.5 (data minimization — Iceberg snapshot expiry). NIS2 Art.21 (data sovereignty — Hetzner self-host alternative).

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-25 ADR review session.*
