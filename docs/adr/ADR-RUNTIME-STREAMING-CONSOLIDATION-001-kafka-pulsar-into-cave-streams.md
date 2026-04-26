# ADR-RUNTIME-STREAMING-CONSOLIDATION-001 — Kafka + Pulsar Konsolide Reimpl

**Status:** Accepted
**Scope:** Cave Runtime (independent override; multi-upstream consolidation)
**Category:** Charter / Architecture (Layer 4 streaming)
**Decided:** 2026-04-25 (Burak Tartan)

## Context

`ADR-RUNTIME-UPSTREAM-MIRROR-001` default mirror = 1 Platform OSS → 1 Runtime crate (örn. Cilium → cave-net). Streaming katmanı bu defaulttan **sapan** bir özel durum: Cave Runtime'da streaming için **Kafka ve Pulsar tek bir crate'te (cave-streams) konsolide reimpl** ediliyor.

İki upstream tek crate'te birleştirilmesinin sebepleri:

- Kafka ekosistemi geniş ve yerleşik (en yaygın streaming protokolü)
- Pulsar multi-tenancy ve geo-replication mimarisi Kafka'dan üstün (broker-decoupled storage, namespace-tier hiyerarşisi)
- İki ayrı engine bakımı zor; tek konsolide implementasyon her iki wire-protokolünü servis ederse tenant'ların migration ihtiyacı sıfır
- Cave Runtime'ın multi-tenant invariant'ı (ADR-MULTI-TENANT-001) Pulsar'ın namespace modeline yakın, Kafka'ya wire-uyumluluğu da gerek

## Decision

Cave Runtime streaming katmanı **`cave-streams`** crate'i altında tek Rust implementasyon olarak yazılır. **Kafka** ve **Pulsar** her ikisi upstream referansıdır; her ikisinin wire protokolü desteklenir.

### Mimari

```
┌────────────────────────────────────────────────────┐
│  cave-streams (single Rust crate)                  │
│                                                     │
│  ┌──────────────┐   ┌──────────────┐                │
│  │ Kafka wire   │   │ Pulsar wire  │ ← protocol     │
│  │ adapter      │   │ adapter      │   adapters     │
│  └──────┬───────┘   └──────┬───────┘                │
│         └──────┬───────────┘                        │
│                ▼                                    │
│  ┌────────────────────────────────┐                 │
│  │ Cave streaming engine (core)   │ ← shared logic  │
│  │  • Topic / namespace           │                 │
│  │  • Partition / segment         │                 │
│  │  • Consumer group / sub        │                 │
│  │  • Tenant isolation (built-in) │                 │
│  │  • Geo-replication             │                 │
│  │  • WAL → cave-kernel           │                 │
│  └────────────┬───────────────────┘                 │
│               ▼                                     │
│      cave-kernel WAL + Raft primitives              │
└────────────────────────────────────────────────────┘
```

### Wire compatibility

- **Kafka clients** (Java, Python, Go, librdkafka tabanlı tüm istemciler) cave-streams'e bağlanır, kod değişikliği yok.
- **Pulsar clients** (Java, Python, Go, C++) cave-streams'e bağlanır, kod değişikliği yok.
- İki wire protokolü aynı topic'lere okuma/yazma yapabilir (transparent bridge).

### Multi-tenant invariant

Pulsar'ın `tenant/namespace/topic` üçlü hiyerarşisi Cave default modelidir. Kafka wire `topic`'leri `tenant/namespace/<topic>` formatında map edilir. Default-deny cross-tenant traversal.

## Consequences

### Positive
- Tek Rust binary, iki ekosistem desteği
- Tenant migration zero-friction (mevcut Kafka/Pulsar uygulamaları aynen çalışır)
- Pulsar'ın multi-tenancy avantajı + Kafka'nın ecosystem genişliği
- WAL ve Raft cave-kernel paylaşımı (sweep-001/002)
- Cilium-inspired eBPF observability ile flow visibility

### Negative
- İki wire protokol bakımı tek codebase'te (her upgrade'de iki API uyumluluğu)
- Kafka ve Pulsar semantic farkları (örn. exactly-once, transaction model) bridge'de care gerek
- Test surface 2× (Kafka tests + Pulsar tests + interop tests)

### Risks
| Risk | Mitigation |
|---|---|
| Wire protocol divergence (Kafka 4.0 / Pulsar 4.0 breaking) | Pin upstream test corpus version. Adapter layer abstrakte protocol changes. |
| Performance gap vs native Kafka (libkafka C++) | Rust async runtime + zero-copy buffers. Benchmark target: ≥80% native Kafka throughput. |
| Pulsar exclusive features (delayed delivery, key-shared sub) Kafka tarafında map'lenemez | Document as Pulsar-wire-only features. Kafka clients'a 501 Not Implemented döndür. |

## Mirror inheritance
- Platform `ADR-streaming-001` (varsa) Kafka veya Pulsar deployment kararını kapsar.
- Runtime bu ADR ile **iki upstream'i konsolide** ediyor — Mirror'ın özel hali.

## Related
- ADR-RUNTIME-UPSTREAM-MIRROR-001 — bu ADR onun istisnası (multi-upstream consolidation)
- ADR-MULTI-TENANT-001 — namespace = tenant boundary
- ADR-RUNTIME-STACK-001 — Layer 4 ekosistem reimpl
- cave-kernel sweep-001/002 — WAL/Raft shared primitives

---
*Decided by Burak Tartan, recorded by Sonnet, 2026-04-25 ADR review session.*
