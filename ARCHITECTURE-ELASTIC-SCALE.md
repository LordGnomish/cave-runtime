# CAVE Unified Runtime — Elastic Scale Architecture (1 → 1M Tenants)

**Document Version:** 1.0  
**Date:** April 2026  
**Scope:** Single binary, multi-tenant, elastic horizontal scaling from edge to hyperscale SaaS.

## Executive Summary

The CAVE Unified Runtime today is a monolithic binary with in-memory state management, designed for single-deployment or small multi-tenant scenarios. To serve 1 million tenants at scale while maintaining the same binary, codebase, and deployment model requires a **true elastic architecture** — not just horizontal pod replication.

This document outlines:
1. Current architecture assessment (bottlenecks, state patterns, module landscape)
2. A three-tier elastic scaling strategy with automatic transitions
3. HA/DR architecture patterns for each tier
4. Module-level isolation and tenant sharding strategies
5. Concrete implementation roadmap with effort estimates

**Key principle:** The runtime must detect deployment scale and **automatically reconfigure** itself. A development team with 10 tenants should not pay the operational cost of a 1M-tenant hyperscale system.

---

## A. Current Architecture Assessment

### A.1 State Management Patterns

The runtime uses **three distinct state management approaches**:

#### In-Memory State (Majority)
- **Modules:** `cave-flags`, `cave-secrets`, `cave-lint`, `cave-tracker`, `cave-portal`, `cave-scaffold`, `cave-incidents`, `cave-slo`, `cave-profiler`, `cave-policy`, `cave-pam`
- **Pattern:** `Arc<RwLock<State>>` or `Arc<State>` with no persistence
- **Bottleneck:** Lost on restart; no replication; memory-bound per instance
- **Example:** `TrackerState` holds `HashMap<Uuid, Issue>` — scales linearly with issue count

#### Single PostgreSQL Schema-Per-Module
- **Modules:** `cave-flags` (with read-through cache), `cave-vulns`, `cave-sbom`, etc.
- **Pattern:** Each module owns a schema (e.g., `flags.features`, `vulns.vulnerabilities`)
- **Bottleneck:** Single PostgreSQL instance; no sharding; write-through cache helps but not distributed
- **Persistence:** Good; queryable
- **Example:** `FlagsState` has `Arc<CavePool>` + `Arc<RwLock<FeatureCache>>`

#### Distributed Storage Engines
- **Modules:** `cave-store` (etcd + S3), `cave-cache` (Redis), `cave-streams` (Kafka)
- **Pattern:** These ARE distributed but not plugged into core modules yet
- **Bottleneck:** Isolated; core modules don't use them for multi-tenant isolation
- **Example:** `StoreState` implements full MVCC + WAL but only used for KV/objects, not module data

#### HA/Raft (Present but Dormant)
- **Crate:** `cave-ha` — full Raft consensus, leader election, DR
- **Status:** Implemented; not integrated into module state machine
- **Bottleneck:** Modules must opt-in; no automatic consensus wrapping

### A.2 Scaling Bottlenecks

| Tier | Current Bottleneck | Impact |
|------|-------------------|--------|
| **1-10 tenants (edge)** | In-memory state lost on restart | Dev friction; state recreation needed |
| **10-100 tenants** | Single PostgreSQL + memory cache invalidation | Cache coherency problems; no tenant isolation |
| **100-10K tenants** | No sharding; all data in one schema | Queries slow (O(n) scans); memory explodes |
| **10K-1M tenants** | Monolithic module state; no cell architecture | Single failure cascades; no disaster recovery |

### A.3 Module Classification

#### **Stateless Modules** (can scale horizontally with zero coordination)
- `cave-docs`, `cave-status`, `cave-changelog`, `cave-certs` — read from git/files
- `cave-lint`, `cave-security` — compute on request
- `cave-dast` — scan-on-demand
- `cave-admission` — validation rules (config-driven)
- Scaling: Unlimited replicas, no shared state

#### **Weakly Stateful Modules** (state is cached; strong consistency not required)
- `cave-flags` — write-through cache; consistency is acceptable at millisecond level
- `cave-chat`, `cave-ai-obs` — LLM state; ephemeral
- Scaling: Replicas with eventual consistency caching

#### **Strongly Stateful Modules** (must maintain consistency; local state critical)
- `cave-tracker` — issue graph, workflow state, sprint state
- `cave-incidents` — alert state, escalation tree
- `cave-slo` — SLO calculations, burn rate tracking
- `cave-workflows` — execution state, variables, retry logic
- Scaling: Must use consensus or primary-replica replication

#### **Event-Driven Modules** (append-only, shardable)
- `cave-streams` (Kafka) — by partition
- `cave-metrics` (TSDB) — by time + tenant + label combo
- `cave-logs` — by tenant + time
- `cave-trace` — by tenant + trace ID
- Scaling: Horizontal shard per partition; no consistency issues

#### **External Dependency Modules** (heavy lifting; offload to SaaS)
- `cave-llm-gateway` — talks to OpenAI, Claude, etc.
- `cave-devlake`, `cave-vault`, `cave-pam` — integrate with external systems
- Scaling: Stateless; reply on upstream rate limits

### A.4 Current Deployment Model

**Single Binary:** All 66 crates compiled into one ~50MB Alpine binary.

**Monolithic State Initialization** (main.rs):
```rust
let secrets_state = Arc::new(cave_secrets::SecretsState::default());  // in-memory
let flags_state = Arc::new(cave_flags::FlagsState::default());        // needs DB pool
let tracker_state = Arc::new(cave_tracker::TrackerState::default()); // full HashMap
let store_state = cave_store::StoreState::in_memory();               // in-memory KV
```

**No multi-tenancy frame:** Tenants exist at the HTTP middleware level (Okta/Keycloak groups), not at the state machine level.

**Config is static:** `cave-runtime.yaml` has per-module toggles (`modules.flags: true/false`) but no per-tenant or per-tier config.

---

## B. Elastic Scale Architecture (1 → 1M Tenants)

### B.1 Design Principles

1. **Same binary, same codebase** — no feature gating, no edition splits
2. **Auto-detection:** Runtime reads deployment scale from environment and reconfigures state layer
3. **Pluggable persistence:** Modules declare what they need (memory, SQLite, PostgreSQL, Raft)
4. **Explicit multi-tenancy:** Tenants are first-class in state management, not middleware
5. **Failover transparency:** Same API regardless of replication mode (leader-elected, primary-replica, sharded)

### B.2 Architecture Overview

```
┌────────────────────────────────────────────────────────────────────────────┐
│                    CAVE Unified Runtime (Single Binary)                     │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐ │
│  │  Scale Detection & Configuration Layer (NEW)                         │ │
│  │  • Reads CAVE_SCALE env (edge/mid/hyperscale)                       │ │
│  │  • Auto-configures persistence, HA mode, sharding                   │ │
│  │  • Initializes module state with correct backends                   │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐ │
│  │  Module State Layer (Abstracted)                                    │ │
│  │  • Stateless modules → always in-memory                             │ │
│  │  • Weakly stateful → memory + eventual-consistency backend          │ │
│  │  • Strongly stateful → Raft consensus OR primary-replica           │ │
│  │  • Event-driven → streams + sharding by partition/tenant           │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐ │
│  │  Data & Persistence Layer                                            │ │
│  │  • Tier 1: SQLite (embedded)                                         │ │
│  │  • Tier 2: PostgreSQL (single replica)                              │ │
│  │  • Tier 3: PostgreSQL + Redis + Kafka (distributed)                │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐ │
│  │  HA/DR & Observability                                               │ │
│  │  • cave-ha: Leader election, Raft, split-brain prevention          │ │
│  │  • cave-metrics/cave-logs/cave-trace: Distributed ingestion        │ │
│  │  • cave-vault: Encrypted secrets management                        │ │
│  └──────────────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## B.3 Tier 1: Single Tenant / Edge (1-10 tenants)

**Deployment:** Bare metal, single server, or small VM. No external dependencies.

**Configuration:**
```yaml
scale:
  tier: "edge"
  tenants_estimate: 5
  
database:
  engine: "sqlite"
  path: "./data/cave.db"
  
storage:
  engine: "embedded"
  backend: "memory"  # All state in-process
  
ha:
  enabled: false      # No replication needed
  
modules:
  # All modules use in-memory state
  tracker:
    persistence: "sqlite"  # Snapshot to disk for durability
    replication: "none"
```

**State Management:**
- **Persistent:** TrackerState, FlagsState, etc. use SQLite via `cave-db` persistence layer
- **In-memory:** Keep read-through cache (FeatureCache in FlagsState)
- **Durability:** SQLite transactions + WAL
- **Init:** Load all state from SQLite on startup

**Module Wiring Example:**
```rust
// In main.rs, during scale detection:
let store = if cfg_scale == "edge" {
    Arc::new(cave_db::persistence::DiskStorage::new("./data/flags.db").await?)
} else {
    Arc::new(cave_db::persistence::PostgresStorage::new(pool).await?)
};

let flags_state = Arc::new(FlagsState::new(store));
```

**Scaling Limit:** ~10 tenants per node; ~100K features, ~50K flags per tenant.

**HA/DR:**
- Single point of failure = complete outage
- **Mitigation:** Automated SQLite backups to S3 every hour (via `cave-backup`)
- **Recovery:** RTO = 5 min (restore from S3 + restart); RPO = 1 hour

**Pros:**
- Zero external infrastructure
- Instant startup
- No networking overhead
- Perfect for dev teams, startups, edge deployments

**Cons:**
- No redundancy
- Memory bound to single node
- Restart loses in-flight requests

### B.4 Tier 2: Mid-Scale (10-10K tenants)

**Deployment:** Kubernetes cluster with 3+ nodes; PostgreSQL managed service; Redis cache.

**Configuration:**
```yaml
scale:
  tier: "mid"
  tenants_estimate: 1000
  expected_growth: "12mo"  # Auto-upgrade based on time/metrics
  
database:
  engine: "postgres"
  url: "postgres://cave:pwd@postgres.default:5432/cave"
  pool_size: 20
  
cache:
  engine: "redis"
  url: "redis://redis.default:6379"
  invalidation_strategy: "publish-subscribe"
  
ha:
  enabled: true
  mode: "active-passive"  # Single leader
  
modules:
  tracker:
    persistence: "postgres"
    sharding: "none"        # All tenants in single schema
    replication: "read-only"  # Hot standby
```

**State Management:**

**Strongly Stateful Modules (TrackerState):**
```
┌─────────────────────┐     ┌──────────────────┐
│  Primary   (L)      │     │  Standby   (F)   │
│  Tracker Instance   │────▶│  Tracker Instance│
│  ┌─────────────────┐│     │ ┌──────────────┐ │
│  │ RwLock<HashMap> ││     │ │ RwLock<empty>│ │
│  │ (hot, mutable)  ││     │ │ (replication)│ │
│  └─────────────────┘│     │ └──────────────┘ │
└─────────────────────┘     └──────────────────┘
         ▲                            │
         │                            │
         └────────────────────────────┘
         PostgreSQL log streaming
         (Logical Replication)
```

- **Tracker, Incidents, Workflows** use Postgres + in-memory cache
- Writes go to primary PostgreSQL; replicated via WAL streaming
- Read-through cache on each instance; invalidated via Redis Pub/Sub
- Leader elected via `cave-ha` Raft (separate 3-node cluster)

**Weakly Stateful Modules (FlagsState):**
```
┌─────────────┐   ┌─────────────┐   ┌─────────────┐
│  Instance 1 │   │  Instance 2 │   │  Instance 3 │
│ FeatureCache│   │ FeatureCache│   │ FeatureCache│
│             │   │             │   │             │
└──────┬──────┘   └──────┬──────┘   └──────┬──────┘
       │                 │                 │
       └─────────────────┼─────────────────┘
                         │
                  PostgreSQL (shared)
                  ┌────────────────┐
                  │ flags.features │
                  └────────────────┘
                         ▲
                         │ Cache invalidation
                         │ (Redis Pub/Sub)
```

- All instances have in-memory FeatureCache
- Write invalidates via Redis; instances reload on next read
- Millisecond consistency; acceptable lag (100ms worst case)

**Event-Driven Modules (Streams, Metrics):**
- Logs, metrics, traces go to PostgreSQL time-series tables (Timescale extension)
- Tenant-local consumers; no cross-tenant queries
- Sharding key: `(tenant_id, date)` for retention policies

**Init Flow:**
```rust
let ha_enabled = cfg_scale == "mid" || cfg_scale == "hyperscale";
let (leader_election, consensus) = if ha_enabled {
    let ha = cave_ha::init_from_env().await?;
    (Some(ha.clone()), Some(ha.consensus()))
} else {
    (None, None)
};

let tracker_state = Arc::new(TrackerState::new(
    pg_pool.clone(),
    consensus.clone(),  // For coordination (if None, no-op)
    cache_layer.clone(),
));
```

**Scaling Limits:**
- PostgreSQL: ~10K concurrent connections (pool_size per instance × instances)
- Redis: ~100K concurrent clients; 50GB per instance
- Per instance: ~100 tenants with 10K issues each = 1M objects

**HA/DR:**

**HA (Tier 2):**
- **Mode:** Active-passive with automatic failover
- **Leader Election:** Raft in `cave-ha` module (3-node etcd-style quorum)
- **Failover Trigger:** Primary Postgres heartbeat timeout (10s) or pod crash
- **Failback:** Manual or automatic (depends on config)
- **Connection Draining:** 30s graceful period before failover
- **Data Consistency:** PostgreSQL WAL ensures durability; no data loss
- **Split-brain Prevention:** Leader verifies quorum before accepting writes

**DR (Tier 2):**
- **RPO:** 5 minutes (backup frequency)
- **RTO:** 15 minutes (restore + service start)
- **Backup Strategy:** 
  - Daily PostgreSQL full backup to S3
  - Continuous WAL archiving (via `pg_basebackup` + `archive_command`)
  - Redis snapshots every 5 minutes
- **Failback:** Replay WAL from S3 to standby; promote standby to primary

**Pros:**
- No external complexity; standard PostgreSQL + Redis
- Automatic failover; simple to operate
- Suitable for 80% of mid-market use cases
- Cost-effective (RDS + ElastiCache)

**Cons:**
- Single write master = bottleneck as tenant count grows
- Cache coherency requires messaging layer (Redis Pub/Sub)
- No geographic distribution; cross-region fail-over requires manual setup

---

### B.5 Tier 3: Hyperscale (10K-1M tenants)

**Deployment:** Multi-region Kubernetes clusters; fully sharded PostgreSQL; Kafka + NATS; distributed consensus.

**Configuration:**
```yaml
scale:
  tier: "hyperscale"
  tenants_estimate: 250000
  shard_count: 256
  regions:
    - name: "us-east-1"
      replicas: 10
    - name: "eu-west-1"
      replicas: 8
    - name: "ap-southeast-1"
      replicas: 5
  
database:
  engine: "postgres_sharded"
  shards:
    - id: 0
      url: "postgres://shard-0.db:5432"
      replicas: 3
    - id: 1
      url: "postgres://shard-1.db:5432"
      replicas: 3
    # ... 256 total
  connection_pool_size: 10
  
cache:
  engine: "redis_cluster"
  nodes:
    - "redis-0.cache:6379"
    - "redis-1.cache:6379"
    # ... 9 total
  
streams:
  engine: "kafka"
  brokers:
    - "kafka-0:9092"
    - "kafka-1:9092"
    - "kafka-2:9092"
  
ha:
  enabled: true
  mode: "consensus"
  raft_cluster: ["ha-0", "ha-1", "ha-2", "ha-3", "ha-4"]
  
modules:
  tracker:
    persistence: "postgres_sharded"
    sharding_key: "tenant_id"
    shard_count: 256
    replication: "3"  # Each shard replicated 3x
```

**State Management: Tenant Sharding**

Each module adopts **tenant-based sharding**:

```
Tenant UUID → Hash → Shard ID (0-255)

tenant-abc-def-ghi → hash(tenant-abc-def-ghi) % 256 = 42
  ↓
  Shard 42: PostgreSQL replica set (3 nodes)
  ├─ Primary: shard-42.db-us.internal:5432
  ├─ Replica 1: shard-42.db-eu.internal:5432
  └─ Replica 2: shard-42.db-ap.internal:5432
```

**Strongly Stateful Modules (TrackerState @ Hyperscale):**

```
┌────────────────────────────────────────────────────────────────┐
│ Load Balancer (Global)                                          │
│ Route by tenant_id % 256 to shard affinity                      │
└────────────────────────────────────────────────────────────────┘
                       │
          ┌────────────┼────────────┐
          │            │            │
          ▼            ▼            ▼
    ┌──────────┐  ┌──────────┐  ┌──────────┐
    │ US East  │  │EU West   │  │AP SE     │
    │ Cluster  │  │ Cluster  │  │ Cluster  │
    │(10 pods) │  │(8 pods)  │  │(5 pods)  │
    └────┬─────┘  └────┬─────┘  └────┬─────┘
         │             │             │
         └─────────────┼─────────────┘
                       │
        ┌──────────────┼──────────────┐
        │              │              │ (by shard)
      Shard 0-85    Shard 86-170   Shard 171-256
        │              │              │
     ┌──────┐       ┌──────┐       ┌──────┐
     │Pg R1 │◄──────│Pg P  │──────►│Pg R2 │
     └──────┘       └──────┘       └──────┘
     (EU)          (US)            (AP)
    (replica)    (primary)        (replica)
```

- **Write path:** Request hits load balancer → shards to (tenant_id % 256) → US East primary → replicates to EU/AP
- **Read path:** Requests route to nearest replica (latency-optimized)
- **Consistency:** Per-shard strong consistency; cross-shard eventual consistency (for rare cross-tenant joins)

**Event-Driven Modules @ Hyperscale (Kafka-based):**

Logs, metrics, traces, and audit events flow through **Kafka** with **tenant-aware partitioning**:

```
┌─────────────────────────────────────────────────────────────┐
│ Application Code (All modules)                              │
│ emit_metric(tenant_id, "request_latency", 125ms)            │
│ emit_log(tenant_id, "INFO", "User created")                │
└──────────┬──────────────────────────────────────────────────┘
           │
           ▼
┌─────────────────────────────────────────────────────────────┐
│ NATS JetStream (Event Router)                               │
│ Topic: events (64 partitions, round-robin by tenant_id)    │
│ Partition 0: tenants [0-3]                                 │
│ Partition 1: tenants [4-7]                                 │
│ ... (tenant_id % 64 = partition)                           │
└──────────┬──────────────────────────────────────────────────┘
           │
           ├─────────────┬──────────────┬────────────────┐
           ▼             ▼              ▼                ▼
      ┌─────────┐  ┌────────┐  ┌───────────┐  ┌────────────┐
      │Metrics  │  │Logs    │  │Trace      │  │Audit       │
      │Consumer │  │Consumer│  │Consumer   │  │Consumer    │
      │(Timescale)│(Timescale)│(Jaeger)   │ │(PostgreSQL) │
      └─────────┘  └────────┘  └───────────┘  └────────────┘
```

- **Kafka guarantees:** Per-partition ordering; same tenant always hits same partition
- **Retention:** Configurable per event type (metrics: 30 days, logs: 7 days, trace: 72 hours)
- **Scaling:** Add partitions/brokers as load grows; rebalance is transparent to consumers

**Cell-Based Architecture (Failure Domain Isolation):**

At hyperscale, introduce **cells** — isolated subsets of infrastructure. One cell failure does not cascade:

```
Global Load Balancer
    │
    ├────────────────────┬────────────────────┬──────────────────┐
    │                    │                    │                  │
    ▼                    ▼                    ▼                  ▼
┌─ CELL A ───┐   ┌─ CELL B ───┐   ┌─ CELL C ───┐   ┌─ CELL D ───┐
│ us-east-1  │   │ eu-west-1  │   │ ap-se-1    │   │ backup     │
│            │   │            │   │            │   │ (cold)     │
│ • Shard 0  │   │ • Shard 64 │   │ • Shard128 │   │            │
│ • Shard 63 │   │ • Shard127 │   │ • Shard191 │   │ (read-only)│
│            │   │            │   │            │   │            │
│ (32 shards)│   │ (64 shards)│   │ (64 shards)│   │ (passive)  │
│ 10 PODs    │   │ 8 PODs     │   │ 5 PODs     │   │ 1 POD      │
└────────────┘   └────────────┘   └────────────┘   └────────────┘
     10K tenants      12K tenants      8K tenants      0K tenants
                                                       (standby for
                                                        any cell)
```

- Each cell handles **distinct shard ranges** (no overlap)
- If CELL A fails, traffic shifts to CELL B/C/D (via DNS or controller)
- Backup cell (CELL D) continuously replicates from all active cells; can become active in 60s
- Tenants in failed cell experience 60-120s downtime; others unaffected

**Module Adaptation @ Hyperscale:**

Modules that were in-memory or single-store now use Kafka-backed state:

```rust
// Before (Tier 1-2):
pub struct TrackerState {
    pub store: Arc<RwLock<HashMap<Uuid, Issue>>>,
}

// After (Tier 3):
pub struct TrackerState {
    pub store: Arc<dyn ShardedStore>,  // Trait: local shard replica + Kafka upstream
    pub kafka_producer: KafkaProducer, // Emit mutations as events
    pub shard_id: u16,                 // Assigned at init based on tenant_id
}

impl TrackerState {
    pub async fn create_issue(&self, tenant_id: &str, issue: Issue) -> Result<Uuid> {
        // 1. Verify request's tenant_id maps to our shard_id
        let shard = hash_tenant(tenant_id) % 256;
        if shard != self.shard_id {
            return Err("Not responsible for this shard"); // Redirect via HTTP 307
        }
        
        // 2. Write to local Postgres shard
        self.store.put("issues", &issue.id.to_string(), &issue).await?;
        
        // 3. Emit to Kafka for audit + analytics
        self.kafka_producer.send(
            Topic::IssueEvents,
            partition: hash_tenant(tenant_id) % 64,
            Key: tenant_id,
            Value: json!({"action": "created", "issue": issue, "timestamp": now()})
        ).await?;
        
        Ok(issue.id)
    }
}
```

**HA/DR @ Hyperscale:**

**HA:**
- **Leader Election:** Raft consensus across 5+ nodes (not dependent on any single region)
- **Failover Mode:** 
  - Primary shard down → failover to replica (automatic, <5s)
  - Entire cell down → shift to backup cell (DNS update, ~60s, transparent to clients)
- **Health Checks:** 
  - Shard level: Every 5s to primary (TCP probe + SQL ping)
  - Cell level: Every 10s consensus health (Raft heartbeats)
  - Global: Every 30s all-cells check (from multiple LBs)
- **Recovery:** Automatic; no human intervention
- **Split-brain Prevention:** Quorum-based writes; leader must verify 3/5 majority before accepting writes

**DR:**

| Scenario | RPO | RTO | Mechanism |
|----------|-----|-----|-----------|
| **Shard replica failure** | 0s | 10s | Automatic rebalance to other replicas |
| **Primary shard down** | 0s | 5s | Failover to replica; promote to primary |
| **Entire region down** | 5min | 60s | Backup cell activated; replay Kafka |
| **Multi-region failure** | 30min | 15min | Restore from S3 snapshots to new cluster |
| **Complete global failure** | 1hr | 30min | RTO point-in-time restore from WAL archives |

- **Backup Strategy:** 
  - Per-shard snapshots every 5 min → S3
  - WAL streaming to S3 (continuous, near-zero lag)
  - Kafka snapshots every 1 hour → S3 Glacier (long-term)
- **Cross-Region Replication:** 
  - Primary cell in US East; replicas in EU West + AP SE (async)
  - Jitter: ~500ms for US→EU, ~1s for US→AP
- **Failback:** 
  - After failure recovery, replay Kafka events to get backup cell up-to-date
  - Manual or automatic based on policy

**Pros:**
- Truly global scale; multi-region resilience
- Per-tenant SLA isolation (one noisy tenant doesn't affect others)
- Automatic failover; no manual intervention
- Cost-optimized: cold cells reduce standby cost

**Cons:**
- Complexity: Kafka, multiple Postgres clusters, Raft consensus, cell management
- Operational overhead: Shard rebalancing, cell failover, cross-region sync
- Eventual consistency: Cross-shard operations have lag

---

### B.6 Automatic Tier Transitions

The runtime **detects scale and auto-upgrades**:

```rust
pub enum TierTransition {
    Init,                      // First startup
    ScaleUp,                   // More tenants/load
    ScaleDown,                 // Fewer tenants
    FailureRecovery,          // Restarting after outage
}

pub async fn detect_tier_and_configure() -> Result<DeploymentTier> {
    let env_tier = std::env::var("CAVE_SCALE").ok();
    
    if let Some(explicit) = env_tier {
        return Ok(DeploymentTier::from_env(&explicit)?);
    }
    
    // Auto-detect from metrics
    let metrics = load_deployment_metrics().await?;
    
    match (metrics.tenant_count, metrics.data_size_gb, metrics.qps) {
        (0..=10, 0..=2, 0..=100) => Ok(DeploymentTier::Edge),
        (11..=10000, 3..=500, 101..=10000) => Ok(DeploymentTier::Mid),
        (10001.., 501.., 10001..) => Ok(DeploymentTier::Hyperscale),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let tier = detect_tier_and_configure().await?;
    
    match tier {
        DeploymentTier::Edge => {
            // Init SQLite + in-memory state
            init_edge_tier().await?;
        }
        DeploymentTier::Mid => {
            // Init PostgreSQL + Redis + HA failover
            init_mid_tier().await?;
        }
        DeploymentTier::Hyperscale => {
            // Init sharded PostgreSQL + Kafka + cells
            init_hyperscale_tier().await?;
        }
    }
    
    serve_runtime().await
}
```

**Upgrade Flow (Edge → Mid → Hyperscale):**

```
┌──────────────────┐
│ Edge Tier        │ (SQLite, in-memory)
│ 10 tenants       │
│ 1 GB data        │
│ 50 QPS           │
└────────┬─────────┘
         │ (Tenant count grows to 50)
         │ (Metrics show sustained >90% CPU)
         │
         ▼
┌──────────────────────────────────────────────────────────────┐
│ Pre-Flight Checks                                             │
│ • PostgreSQL connectivity ✓                                  │
│ • Redis available ✓                                          │
│ • Disk space for dump ✓                                      │
│ • Network latency acceptable ✓                               │
└────────────────┬─────────────────────────────────────────────┘
                 │
         (if checks pass)
         │
         ▼
┌──────────────────────────────────────────────────────────────┐
│ Migration Sequence                                            │
│ 1. Drain requests (30s); no new traffic                      │
│ 2. Dump SQLite → PostgreSQL (15s, index on tenant_id)       │
│ 3. Init Redis cache; warm from PostgreSQL (30s)             │
│ 4. Start HA Raft cluster (leader election, ~10s)           │
│ 5. Switch to Mid tier config; reload module state          │
│ 6. Resume requests on new tier                             │
│                                                             │
│ Total downtime: ~60 seconds (acceptable for Mid-scale)    │
└────────────────┬─────────────────────────────────────────────┘
                 │
                 ▼
         ┌──────────────┐
         │ Mid Tier     │ (PostgreSQL, Redis, HA)
         │ 50 tenants   │
         │ 5 GB data    │
         │ 500 QPS      │
         └──────────────┘
                 │
              (grows to 15K tenants over 12 months)
                 │
                 ▼
         [similar flow to Hyperscale]
```

**Downtime Estimates:**

| Transition | Downtime | Data Validation |
|-----------|----------|-----------------|
| Edge → Mid | ~60s | Dump/restore verification |
| Mid → Hyperscale | ~2-3 min | Shard by tenant_id; warm replicas |
| Hyperscale cell failover | ~60s | Health check → DNS change |

---

## C. HA Architecture

### C.1 Active-Active vs Active-Passive

| Mode | Tier | Write Latency | Failover Time | Complexity |
|------|------|---------------|---------------|-----------|
| **Active-Passive** (leader/standby) | Mid | 5ms (to primary) | 10-30s | Low |
| **Active-Active** (quorum writes) | Hyperscale | 50ms (quorum consensus) | 5s | High |
| **Cell-based** | Hyperscale | 200ms (cross-region) | 60s (cell swap) | Extreme |

**Recommendation:**
- **Tier 1 (Edge):** Passive (no replication); single point of failure acceptable
- **Tier 2 (Mid):** Active-passive with automatic failover
- **Tier 3 (Hyperscale):** Active-active (per-shard) + cell-level failover

### C.2 Leader Election & Consensus (cave-ha)

The `cave-ha` module handles leader election using **Raft consensus**:

```rust
pub struct RaftConfig {
    pub node_id: NodeId,
    pub cluster: Vec<NodeInfo>,      // All nodes in cluster
    pub heartbeat_interval: Duration, // 150ms
    pub election_timeout_min: Duration, // 300ms
    pub election_timeout_max: Duration, // 900ms
}

pub struct HaManager {
    raft: RaftHandle,
    state_machine: Arc<dyn StateMachine>,  // Pluggable
}

impl HaManager {
    pub async fn apply_command(&self, cmd: Vec<u8>) -> Result<Vec<u8>> {
        // Send to Raft; waits for majority quorum
        self.raft.client_write(cmd).await
    }
    
    pub fn is_leader(&self) -> bool {
        self.raft.current_role() == Role::Leader
    }
}
```

**Failover Sequence:**

```
Time 0s:
┌──────────┐  ┌──────────┐  ┌──────────┐
│ Node A   │  │ Node B   │  │ Node C   │
│ LEADER   │◄─│ FOLLOWER │──│ FOLLOWER │
│ (healthy)│  │ (healthy)│  │ (healthy)│
└──────────┘  └──────────┘  └──────────┘

Time 15s:
┌──────────┐  ┌──────────┐  ┌──────────┐
│ Node A   │  │ Node B   │  │ Node C   │
│ DEAD     │  │ FOLLOWER │  │ FOLLOWER │
│ (crashed)│  │ (healthy)│  │ (healthy)│
└──────────┘  └──────────┘  └──────────┘
              (election timeout fires)

Time 20s:
              ┌──────────┐  ┌──────────┐
              │ Node B   │  │ Node C   │
              │ LEADER   │  │ FOLLOWER │
              │ (elected)│  │ (healthy)│
              └──────────┘  └──────────┘
              (quorum = 2/3)
              
Time 25s:
(Node A returns)
┌──────────┐  ┌──────────┐  ┌──────────┐
│ Node A   │  │ Node B   │  │ Node C   │
│ FOLLOWER │  │ LEADER   │  │ FOLLOWER │
│ (resyncs)│◄─│ (leader) │──│ (healthy)│
└──────────┘  └──────────┘  └──────────┘
(catches up on missed entries)
```

**Client Impact:**
- Write failure at T15s (leader died); client retries
- Retry succeeds at T20s (new leader elected)
- **Recovery time:** 300-900ms (configurable election timeout)

### C.3 Health Checking & Circuit Breakers

**Multi-level health checks:**

```rust
pub struct HealthChecker {
    // Level 1: Pod-level (every 5s)
    pod_checks: Vec<PodHealthCheck>,
    
    // Level 2: Shard-level (every 10s)
    shard_checks: Vec<ShardHealthCheck>,
    
    // Level 3: Cell-level (every 30s)
    cell_checks: Vec<CellHealthCheck>,
}

pub struct PodHealthCheck {
    target: String,  // "pod-name:8080"
    timeout: Duration,  // 2s
    unhealthy_threshold: u32,  // 3 consecutive failures
    healthy_threshold: u32,  // 2 consecutive successes
}

pub async fn check_pod_health(check: &PodHealthCheck) -> Result<HealthStatus> {
    // Probe: GET /health
    // Expected: 200 OK + JSON {"status": "ok"}
    let resp = reqwest::get(&format!("http://{}/health", check.target))
        .timeout(check.timeout)
        .await?;
    
    Ok(if resp.status() == 200 {
        HealthStatus::Healthy
    } else {
        HealthStatus::Unhealthy(format!("Status {}", resp.status()))
    })
}
```

**Circuit Breaker Pattern:**

```
Open (failing)     Closed (healthy)
    │                     │
    │ 3 failures          │ 2 successes
    │                     │
    └──────────────┬──────┘
                   │
                Half-Open
            (testing recovery)
                   │
        ┌──────────┴──────────┐
        │                     │
    Success              Failure
        │                     │
        ▼                     ▼
     Closed              Back to Open
  (traffic flows)     (reject fast)
```

```rust
pub struct CircuitBreaker {
    state: Mutex<CircuitState>,
    failure_threshold: u32,
    success_threshold: u32,
    timeout: Duration,  // Time in Open before trying Half-Open
}

impl CircuitBreaker {
    pub async fn call<F>(&self, f: F) -> Result<()>
    where F: FnOnce() -> BoxFuture<'static, Result<()>>
    {
        match *self.state.lock() {
            CircuitState::Closed => {
                // Try the call; track failures
                f().await
            }
            CircuitState::Open => {
                // Reject immediately (fail-fast)
                Err("Circuit open; not retrying")
            }
            CircuitState::HalfOpen => {
                // Try one call; if success, go Closed; if fail, go Open
                f().await
            }
        }
    }
}
```

### C.4 Zero-Downtime Deployments

**Blue-Green Deployment (Tier 2+):**

```
Time 0:     Time 10m:          Time 15m:
┌─────┐     ┌──────┐ ┌──────┐  ┌──────┐
│Blue │     │Blue  │ │Green │  │Green │
│(v1) │────▶│(v1)  │ │(v2)  │─▶│(v2)  │
│ 100 │     │50 reqs│ │50 req│  │100 re│
│reqs │     │       │ │      │  │      │
└─────┘     └──────┘ └──────┘  └──────┘
  (live)    (canary) (parallel) (live)
             traffic
             shifting
```

**Process:**
1. Deploy Green version (v2) alongside Blue (v1)
2. Route 5% traffic to Green; monitor metrics
3. If Green healthy, shift 25% → 50% → 100%
4. Once 100%, remove Blue

**Rollback:** If Green fails, instant switch back to Blue (50ms)

**Implementation:**

```rust
pub async fn deploy_with_blue_green(
    new_version: &str,
    traffic_curve: Vec<(Duration, f32)>,  // [(T+0s, 5%), (T+5m, 50%), (T+10m, 100%)]
) -> Result<()> {
    // 1. Start Green
    let green_pod = spawn_pod(new_version).await?;
    wait_for_ready(&green_pod, Duration::from_secs(30)).await?;
    
    // 2. Shift traffic
    for (wait_time, percentage) in traffic_curve {
        sleep(wait_time).await;
        update_lb_weights(&[("blue", 100 - percentage), ("green", percentage)]).await?;
        
        // Monitor error rate; abort if >1%
        let error_rate = get_metric("error_rate_pct").await?;
        if error_rate > 1.0 {
            rollback_to_blue().await?;
            return Err("Error rate spike detected; rolled back");
        }
    }
    
    // 3. Delete Blue
    terminate_pod("blue").await?;
    
    Ok(())
}
```

### C.5 Split-Brain Prevention

Raft consensus prevents split-brain:

```
Network partitioned at T=10s
┌──────┐              ┌──────┐  ┌──────┐
│Node A│              │Node B│  │Node C│
│      │  [cut]       │      │  │      │
│ LEAD │◄────────────▶│ FOLL │  │ FOLL │
└──────┘              └──────┘  └──────┘
  (1)                   (2)
  
Node A continues as LEADER (1 node).
Nodes B+C: Elect new LEADER (2 nodes, quorum).

Write on A: ❌ REJECTED (only 1/3 quorum)
Write on B: ✓ ACCEPTED (2/3 quorum)

When partition heals, A realizes it's behind and catches up from B's log.
```

---

## D. DR Architecture

### D.1 RPO/RTO Targets

| Tier | Failure Scenario | RPO | RTO | Strategy |
|------|------------------|-----|-----|----------|
| **Edge** | Pod crash | 1h | 5m | SQLite backup to S3 |
| **Edge** | Disk corruption | 1h | 15m | Restore from S3 snapshot |
| **Mid** | Primary fails | 0s | 10s | Postgres failover to replica |
| **Mid** | Region down | 5m | 15m | Restore from WAL archives |
| **Hyperscale** | Shard replica down | 0s | 5s | Rebalance to other replicas |
| **Hyperscale** | Shard primary down | 0s | 5s | Promote replica to primary |
| **Hyperscale** | Entire cell down | 5m | 60s | Promote backup cell |
| **Hyperscale** | Multi-region failure | 30m | 30m | Restore from cross-region snapshots |

### D.2 Backup Strategy

**Tier 1 (Edge):**
```
SQLite file → AWS S3 (daily)
Retention: 30 days
Backup size: ~2 GB (full)
```

**Tier 2 (Mid):**
```
PostgreSQL:
  - Full snapshot: daily → S3 (incremental after day 1)
  - WAL archiving: continuous → S3 (5-min rollup)
  - Retention: 30 days
  
Redis:
  - BGSAVE snapshot: every 5 min → S3
  - Retention: 7 days
  
Recovery: Restore base + replay WAL = point-in-time (1-hour granularity)
```

**Tier 3 (Hyperscale):**
```
PostgreSQL (per shard):
  - Streaming replication (3x) — synchronous writes to quorum
  - Per-shard snapshots: every 5 min → S3
  - WAL continuous archive → S3 Glacier (long-term, PITR)
  - Retention: base=30 days, WAL=90 days
  
Kafka:
  - Event snapshots: every 1 hour → S3 (for reprocessing)
  - Retention: 7 days
  - Consumer groups: auto-committed offset, backed by Zookeeper
  
NATS JetStream:
  - Built-in persistence (3 replicas per message)
  - Retention: 24 hours
  
Recovery options:
  1. Replica failover (fast, <10s)
  2. Rebuild from Kafka (slow, requires reprocessing, 1-6h)
  3. Restore from S3 snapshot + replay WAL (medium, 15-30m)
```

### D.3 Cross-Region Replication

**Tier 2 → Tier 3 upgrade:**
```
Region: US East         Region: EU West         Region: AP SE
┌──────────────┐       ┌──────────────┐       ┌──────────────┐
│ Primary PG   │───▶   │ Replica PG   │───▶   │ Replica PG   │
│ Primary Redis│───▶   │ Replica Redis│───▶   │ Replica Redis│
└──────────────┘       └──────────────┘       └──────────────┘
  (write all)            (read only)            (read only)
    ↓                      ↓                      ↓
  Kafka broker 0        Kafka broker 1        Kafka broker 2
    (leader for             (follower)          (follower)
     events topic)
```

- **Postgres:** WAL streaming (asynchronous); ~500ms lag US→EU, ~1s US→AP
- **Redis:** Replication module (master-slave); same lag
- **Kafka:** Topic replication factor=3; one broker is leader, others are followers
- **Jitter:** Monitor lag; adjust heartbeat interval if >2s

### D.4 Failover Automation

**Health-driven Failover:**

```rust
pub async fn monitor_and_failover(
    primary_shard: &ShardAddress,
    replicas: Vec<ShardAddress>,
) -> Result<()> {
    let mut failure_count = 0;
    
    loop {
        match health_check(primary_shard).await {
            Ok(_) => {
                failure_count = 0;  // Reset
            }
            Err(_) => {
                failure_count += 1;
                if failure_count >= 3 {  // 3 consecutive failures = unhealthy
                    // Promote best replica to primary
                    let new_primary = select_healthiest_replica(&replicas).await?;
                    promote_replica_to_primary(&new_primary).await?;
                    
                    // Update DNS/config
                    update_shard_routing(primary_shard, &new_primary).await?;
                    
                    // Wait for sync before resuming writes
                    wait_for_quorum_sync(&new_primary, &replicas).await?;
                    
                    tracing::info!("Promoted {:?} to primary", new_primary);
                    failure_count = 0;
                    
                    // Attempt to recover old primary
                    spawn_recovery_task(primary_shard);
                }
            }
        }
        
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
```

### D.5 Data Consistency During Failover

**Problem:** When primary fails, in-flight writes may be lost or duplicated.

**Solution — Exactly-Once Semantics (EOS):**

```rust
pub async fn create_issue_with_eos(
    shard: &Shard,
    tenant_id: &str,
    issue: &Issue,
) -> Result<Uuid> {
    let idempotency_key = format!("{}_{}_{}", tenant_id, issue.id, chrono::now());
    
    // 1. Check idempotency store (Redis)
    if let Some(cached) = shard.idempotency_store.get(&idempotency_key).await? {
        return Ok(cached);  // Duplicate request; return cached result
    }
    
    // 2. Write to Postgres
    let issue_id = shard.postgres.insert_issue(issue).await?;
    
    // 3. Store in idempotency store (TTL=1 hour)
    shard.idempotency_store.set(idempotency_key, issue_id, Duration::from_secs(3600)).await?;
    
    // 4. Emit event to Kafka
    shard.kafka.produce(
        Topic::IssueEvents,
        Event::IssueCreated { issue_id, ..., idempotency_key }
    ).await?;
    
    Ok(issue_id)
}

// On retry (network blip, client timeout):
// 1. Same idempotency_key hit → return cached result immediately
// 2. No duplicate issued in Postgres
// 3. Kafka consumer: sees same event twice but deduplicates by idempotency_key
```

---

## E. Module-Level Analysis

### E.1 Stateless Modules (Scale ∞)

These modules are **compute-only** and scale horizontally with zero coordination:

| Module | Function | State | Scaling |
|--------|----------|-------|---------|
| `cave-docs` | Git-backed doc rendering | None (cached HTML) | Unlimited replicas |
| `cave-status` | System status dashboard | Config (static) | Unlimited replicas |
| `cave-changelog` | Release notes from Git | None (Git read-only) | Unlimited replicas |
| `cave-certs` | Certificate issuance (ACME) | Ledger (append-only) | Unlimited replicas |
| `cave-lint` | Code quality rules | Config (static) | Unlimited replicas |
| `cave-security` | Trivy + Falco scanning | Config (static) | Unlimited replicas |
| `cave-dast` | Web security scanning | Results (ephemeral) | Unlimited replicas |
| `cave-admission` | Admission webhooks | Rules (config) | Unlimited replicas |
| `cave-pii` | Regex-based PII detection | Rules (config) | Unlimited replicas |

**Implementation Pattern:**

```rust
pub struct LintModule;

impl LintModule {
    pub async fn check_code(&self, code: &str) -> Result<Vec<LintIssue>> {
        // Stateless: no dependencies on state
        // Can be run in parallel across many replicas
        // No coordination needed
        let rules = LINT_RULES.get(); // Loaded from config at startup
        rules.iter()
            .filter_map(|rule| rule.matches(code))
            .collect()
    }
}
```

**Tier Deployment:**
- **Edge:** Single replica (no cost)
- **Mid:** 2-3 replicas (load-balanced)
- **Hyperscale:** 10+ replicas (per-region)

### E.2 Weakly Stateful Modules (Scale 10K-100K)

These modules have **soft state** (cache, ephemeral) and tolerate eventual consistency:

| Module | State | Consistency | Scaling |
|--------|-------|-------------|---------|
| `cave-flags` | Feature flags + cache | Millisecond lag acceptable | 1-10 replicas per shard |
| `cave-chat` | LLM conversation (ephemeral) | No persistence after session end | 1-5 replicas |
| `cave-ai-obs` | LLM observability (metrics only) | Aggregated; exact count not critical | 1-10 replicas |
| `cave-alerts` | Alert routing + notification queue | At-least-once delivery (retries) | 1-5 replicas |

**Implementation Pattern:**

```rust
pub struct FlagsState {
    pub store: Arc<dyn Storage>,  // Shared (PostgreSQL)
    pub cache: Arc<RwLock<FeatureCache>>,  // Local (in-memory)
    pub cache_invalidation: Arc<CacheInvalidator>,  // Pub/sub (Redis)
}

impl FlagsState {
    pub async fn evaluate_flag(&self, tenant_id: &str, flag_key: &str) -> Result<bool> {
        // Fast path: check local cache (no DB call)
        {
            let cache = self.cache.read().await;
            if let Some(feature) = cache.features.iter().find(|f| f.key == flag_key) {
                return Ok(feature.enabled);
            }
        }
        
        // Cache miss: load from DB
        let feature = self.store.get::<Feature>("features", flag_key).await?;
        
        // Update local cache
        {
            let mut cache = self.cache.write().await;
            cache.features.push(feature.clone());
        }
        
        Ok(feature.enabled)
    }
    
    pub async fn set_flag(&self, flag: Feature) -> Result<()> {
        // Write to shared store
        self.store.put("features", &flag.key, &flag).await?;
        
        // Invalidate cache on all replicas
        self.cache_invalidation.publish(&format!("flag:{}", flag.key)).await?;
        
        // Clear local cache
        {
            let mut cache = self.cache.write().await;
            cache.features.retain(|f| f.key != flag.key);
        }
        
        Ok(())
    }
}
```

**Tier Deployment:**
- **Edge:** Single replica; no cache invalidation (not multi-tenant)
- **Mid:** 2-5 replicas per shard; invalidate via Redis Pub/Sub
- **Hyperscale:** 5-10 replicas per shard; invalidate via NATS JetStream

**Consistency SLA:** "Your flag change will be live within 100ms on 95% of replicas."

### E.3 Strongly Stateful Modules (Scale 1K-10K)

These modules **require strong consistency** and cannot tolerate data loss:

| Module | State | Consistency | Scaling |
|--------|-------|-------------|---------|
| `cave-tracker` | Issues, boards, sprints, workflows | Strict consistency | Raft + replicas per shard |
| `cave-incidents` | Alert state, escalation, timeline | Strong consistency | Raft + replicas per shard |
| `cave-slo` | SLO definitions, burn rate, events | Strong consistency | Raft + replicas per shard |
| `cave-workflows` | Execution state, variables, retries | Strong consistency | Raft + replicas per shard |
| `cave-backup` | Backup metadata, restore points | Strong consistency | Raft + replicas per shard |

**Implementation Pattern:**

```rust
pub struct TrackerState {
    pub db: Arc<dyn Storage>,  // PostgreSQL (sharded by tenant_id)
    pub raft: Option<RaftHandle>,  // Leader election (Tier 2+)
    pub cache: Arc<RwLock<IssueCache>>,  // L1 cache
}

impl TrackerState {
    pub async fn create_issue(&self, tenant_id: &str, issue: Issue) -> Result<Uuid> {
        // 1. Verify I'm the leader (Tier 2+)
        if let Some(ref raft) = self.raft {
            if !raft.is_leader() {
                return Err("Not leader; redirect to leader");
            }
        }
        
        // 2. Write to database (Tier 1: SQLite, Tier 2+: PostgreSQL)
        self.db.put("issues", &issue.id.to_string(), &issue).await?;
        
        // 3. Invalidate cache
        self.cache.write().await.issues.remove(&issue.id);
        
        // 4. Emit event to Kafka (Tier 3 only)
        if cfg!(feature = "hyperscale") {
            self.kafka_producer.send(...).await?;
        }
        
        Ok(issue.id)
    }
    
    pub async fn get_issue(&self, issue_id: Uuid) -> Result<Option<Issue>> {
        // 1. Check cache
        {
            let cache = self.cache.read().await;
            if let Some(issue) = cache.issues.get(&issue_id) {
                return Ok(Some(issue.clone()));
            }
        }
        
        // 2. Load from DB
        let issue = self.db.get("issues", &issue_id.to_string()).await?;
        
        // 3. Populate cache
        if let Some(ref issue) = issue {
            self.cache.write().await.issues.insert(issue_id, issue.clone());
        }
        
        Ok(issue)
    }
}
```

**Raft Consensus (Tier 2+):**

```rust
pub async fn init_tracker_with_raft(
    tier: DeploymentTier,
    db_pool: Arc<PostgresPool>,
) -> Result<Arc<TrackerState>> {
    let raft = if matches!(tier, DeploymentTier::Mid | DeploymentTier::Hyperscale) {
        let ha = cave_ha::init_from_env().await?;
        Some(ha.raft_handle())
    } else {
        None
    };
    
    Ok(Arc::new(TrackerState {
        db: Arc::new(PostgresStorage::new(db_pool)),
        raft,
        cache: Arc::new(RwLock::new(IssueCache::default())),
    }))
}
```

**Tier Deployment:**
- **Edge:** Single instance; no Raft (loses data on restart, acceptable for dev)
- **Mid:** 3-node Raft quorum; write goes to leader; read from any
- **Hyperscale:** Per-shard Raft quorum (3-5 nodes per shard); ~256 shards → ~1K Raft nodes

### E.4 Event-Driven Modules (Scale 1M)

These modules **append-only** and shard by partition/tenant:

| Module | Events | Sharding | Scaling |
|--------|--------|----------|---------|
| `cave-streams` | Kafka topics | Partition ID (0-63) | 64+ partitions |
| `cave-metrics` | Prometheus samples | Tenant + timestamp | Unlimited |
| `cave-logs` | Structured logs | Tenant + timestamp | Unlimited |
| `cave-trace` | Trace spans | Tenant + trace ID | Unlimited |
| `cave-audit` | Audit events | Tenant + timestamp | Unlimited |

**Implementation Pattern:**

```rust
pub async fn emit_issue_created(
    tenant_id: &str,
    issue: &Issue,
    producer: &KafkaProducer,
) -> Result<()> {
    // Partition = tenant_id.hash() % 64
    // Ensures all events for a tenant go to same partition (ordering)
    let partition = hash(tenant_id) % 64;
    
    producer.send(
        Topic::Events,
        partition,
        Key: format!("{}:{}", tenant_id, issue.id),
        Value: json!({
            "tenant_id": tenant_id,
            "event_type": "IssueCreated",
            "issue": issue,
            "timestamp": now(),
            "version": 1,
        })
    ).await?;
    
    Ok(())
}

pub async fn consume_issue_events(
    tenant_id: &str,
    consumer_group: &str,
) -> Result<()> {
    // Consumer group subscribes to partition(s) responsible for this tenant
    let partition = hash(tenant_id) % 64;
    
    let consumer = KafkaConsumer::new(
        Topic::Events,
        consumer_group,
        ConsumerConfig {
            partition_assignment: PartitionAssignment::Static(vec![partition]),
            isolation_level: IsolationLevel::ReadCommitted,  // Exactly-once
        }
    ).await?;
    
    for msg in consumer.stream() {
        let event: Event = serde_json::from_slice(&msg.value)?;
        
        // Process event (e.g., update metrics)
        process_event(event).await?;
    }
    
    Ok(())
}
```

**Scaling Properties:**
- **Throughput:** Each partition handles ~10K msgs/sec; 64 partitions = 640K msgs/sec
- **Retention:** Configurable (7 days default)
- **Consumers:** Can scale independently per partition
- **No resharding:** Partition count fixed at cluster creation (no rebalancing)

**Tier Deployment:**
- **Edge:** Single partition (no scale needed)
- **Mid:** 4-8 partitions; 1 consumer per partition
- **Hyperscale:** 64+ partitions; multiple consumers per partition (for higher QPS)

---

## F. Data Architecture

### F.1 Multi-Tenant Data Isolation

**Strategy: Schema-per-Tenant (Tier 1-2) → Shard-per-Tenant (Tier 3)**

**Tier 1 (Edge):**
```sql
-- Single SQLite database
CREATE TABLE issues (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR,  -- NOT indexed; assumed single tenant
    project_key VARCHAR,
    title VARCHAR,
    status VARCHAR,
    ...
);
```

**Tier 2 (Mid):**
```sql
-- PostgreSQL, single database, multiple schemas
CREATE SCHEMA tenant_abc;
CREATE SCHEMA tenant_def;

-- Isolation via schema:
CREATE TABLE tenant_abc.issues (
    id UUID PRIMARY KEY,
    project_key VARCHAR,
    title VARCHAR,
    ...
);

CREATE TABLE tenant_def.issues (
    id UUID PRIMARY KEY,
    project_key VARCHAR,
    title VARCHAR,
    ...
);

-- RLS not needed; app filters by schema
```

**Tier 3 (Hyperscale):**
```sql
-- PostgreSQL, sharded (256 shards)
-- Shard 0-63 (US East primary, EU West replica, AP SE replica)
CREATE TABLE issues (
    id UUID PRIMARY KEY,
    tenant_id VARCHAR NOT NULL,  -- INDEXED; used for sharding
    project_key VARCHAR,
    title VARCHAR,
    status VARCHAR,
    -- Check constraint ensures shard locality
    CHECK (hash(tenant_id) % 256 IN (0, 1, 2, ..., 63)),
);

CREATE INDEX idx_issues_tenant ON issues(tenant_id);
CREATE INDEX idx_issues_tenant_status ON issues(tenant_id, status);

-- Routing layer determines shard:
// GET /api/issues?tenant=tenant-abc
// 1. shard_id = hash("tenant-abc") % 256 = 42
// 2. Route to postgres://shard-42.db-us.internal:5432
// 3. Query: SELECT * FROM issues WHERE tenant_id = 'tenant-abc'
```

### F.2 Row-Level Security (RLS) for Extra Safety

**Tier 2+: PostgreSQL RLS (defense-in-depth)**

```sql
-- Enable RLS on all tables
ALTER TABLE issues ENABLE ROW LEVEL SECURITY;

-- Create tenant policy
CREATE POLICY tenant_isolation ON issues
    USING (tenant_id = current_setting('app.tenant_id'));

-- Application sets tenant context before each query:
SET app.tenant_id = 'tenant-abc';  -- In transaction
SELECT * FROM issues WHERE project_key = 'FOO-123';
-- Database automatically filters: WHERE tenant_id = 'tenant-abc'
```

Even if application routing fails, database RLS prevents data leaks.

### F.3 Time-Series Data (Separate Storage)

Logs, metrics, traces need different storage characteristics than transactional data:

**Tier 1 (Edge):**
```sql
-- SQLite with time-series tables
CREATE TABLE metrics (
    timestamp INTEGER,
    tenant_id VARCHAR,
    metric_name VARCHAR,
    labels JSON,
    value REAL,
    PRIMARY KEY (timestamp, metric_name, tenant_id)
);

-- TTL: manually DELETE WHERE timestamp < strftime('%s', 'now') - 86400*7  -- 7 days
```

**Tier 2 (Mid):**
```sql
-- TimescaleDB (PostgreSQL extension)
SELECT create_hypertable('metrics', 'timestamp', if_not_exists => TRUE);

CREATE TABLE metrics (
    timestamp TIMESTAMPTZ NOT NULL,
    tenant_id VARCHAR NOT NULL,
    metric_name VARCHAR NOT NULL,
    labels JSONB,
    value DOUBLE PRECISION
);

-- TimescaleDB auto-chunks by time (1 day default)
-- Compression: compress chunks older than 7 days
SELECT add_compression_policy('metrics', INTERVAL '7 days');

-- Retention: drop chunks older than 30 days
SELECT add_retention_policy('metrics', INTERVAL '30 days');

-- Efficient queries:
SELECT time_bucket('1 hour', timestamp) as hour,
       avg(value) as avg_value
FROM metrics
WHERE tenant_id = 'tenant-abc'
  AND metric_name = 'request_latency'
  AND timestamp > now() - INTERVAL '7 days'
GROUP BY hour;
```

**Tier 3 (Hyperscale):**
```rust
// Use cave-metrics (TSDB) which implements:
// - Gorilla XOR compression (10x better than raw)
// - Inverted index on labels (fast label matching)
// - Downsampling (1min → 5min → 1hour as age increases)
// - Sharding by tenant + label combo

pub async fn emit_metric(
    tenant_id: &str,
    metric: Metric,
    tsdb: &Tsdb,
) -> Result<()> {
    // Metric routing:
    // 1. Partition by (tenant_id + timestamp)
    // 2. Store in separate Kafka topic (metrics)
    // 3. Consumer group aggregates and writes to TSDB
    // 4. TSDB returns cardinality estimate (prevent explosion)
    
    if tsdb.estimated_cardinality() > 1_000_000 {
        return Err("Metric cardinality limit reached; reject high-cardinality labels");
    }
    
    tsdb.write(metric).await
}
```

### F.4 Secrets Management at Scale

**Tier 1 (Edge):**
```rust
// Secrets stored in encrypted YAML file
let secrets_file = Path::new("./secrets.encrypted.yaml");
let secrets = read_encrypted_file(secrets_file, "master_key")?;
```

**Tier 2 (Mid):**
```rust
// Use cave-vault (HashiCorp Vault parity)
// Secrets stored in PostgreSQL (encrypted at rest)
// Accessed via HTTPS + TLS client certs

let vault = CaveVault::new(db_pool);
let api_key = vault.get_secret("tenant-abc/api-keys/github")?;
```

**Tier 3 (Hyperscale):**
```rust
// Use cave-vault with:
// - Dynamic secrets (short-lived, auto-rotated)
// - Per-tenant isolation (separate encryption keys per customer)
// - Audit logging (every secret access logged to Kafka)

let vault = CaveVault::new(db_pool, audit_producer);

// Dynamic secret: valid for 1 hour, auto-rotated
let db_creds = vault.generate_dynamic_secret(
    tenant_id,
    SecretType::PostgresCredential {
        ttl: Duration::from_secs(3600),
    }
)?;
// Uses Postgres role: vault_tenant_abc_1234567 (auto-created, auto-dropped after TTL)

// Per-tenant encryption: key stored in HSM or cloud KMS
let encrypted = vault.encrypt_for_tenant(tenant_id, plaintext)?;  // Uses tenant-specific key
```

### F.5 Audit Trail and Compliance

**Immutable Audit Log (Append-Only):**

```sql
-- Audit events in Kafka (immutable, append-only)
CREATE TABLE audit_log (
    id BIGSERIAL PRIMARY KEY,  -- Global sequence
    tenant_id VARCHAR NOT NULL,
    actor_id UUID NOT NULL,
    action VARCHAR NOT NULL,  -- "issue.created", "issue.updated", etc.
    resource_type VARCHAR NOT NULL,
    resource_id VARCHAR NOT NULL,
    before_state JSONB,  -- Snapshot of old value
    after_state JSONB,   -- Snapshot of new value
    change_summary VARCHAR,
    timestamp TIMESTAMPTZ DEFAULT now(),
    ip_address INET,
    user_agent VARCHAR
);

CREATE INDEX idx_audit_tenant_timestamp ON audit_log(tenant_id, timestamp DESC);
```

**Immutability:** Kafka is the source of truth; audit_log table is read-only replica.

**Compliance:** GDPR/HIPAA audits query this table; never deleted (with GDPR right-to-be-forgotten exception).

---

## G. Concrete Implementation Roadmap

### Phase 1: MVP for Single Tenant (Weeks 1-4)

**Goal:** Edge tier running on developer laptop; zero external dependencies.

**Deliverables:**
1. Scale detection layer (CAVE_SCALE=edge)
2. SQLite persistence for all strongly-stateful modules
3. In-memory caching for weakly-stateful modules
4. Single-pod deployment

**Effort Estimate:** 80 hours (10 person-days)

**Checklist:**
- [ ] Modify main.rs to detect tier and init state accordingly
- [ ] Update cave-db to support SQLite DiskStorage as default for Tier 1
- [ ] Migrate TrackerState to use DiskStorage instead of HashMap
- [ ] Add schema migration runner for SQLite
- [ ] Test with 100 tenants, 1000 issues, single pod
- [ ] Document single-node deployment

**Code Changes:**
```rust
// cave-runtime/src/main.rs
#[tokio::main]
async fn main() -> Result<()> {
    let tier = detect_tier_and_reconfigure().await?;
    
    match tier {
        DeploymentTier::Edge => init_edge_tier().await?,
        _ => panic!("Unsupported tier for this phase"),
    }
    
    serve_runtime().await
}

async fn init_edge_tier() -> Result<()> {
    // SQLite storage
    let storage = cave_db::persistence::DiskStorage::new("./data").await?;
    let pool = Arc::new(cave_db::pool::CavePool::from_storage(storage.clone()));
    
    // Run migrations
    cave_db::migrate::run_all_migrations(&pool).await?;
    
    // Init module states with DiskStorage
    let tracker_state = Arc::new(cave_tracker::TrackerState::new(pool.clone()));
    let flags_state = Arc::new(cave_flags::FlagsState::new(pool.clone()));
    
    // ... etc
    
    Ok(())
}
```

**Testing:**
```rust
#[tokio::test]
async fn test_tracker_persistence_edge() {
    let storage = DiskStorage::new("./test-data").await.unwrap();
    let state = TrackerState::new(Arc::new(CavePool::from_storage(storage)));
    
    // Create issue
    let issue_id = state.create_issue("tenant-a", Issue::default()).await.unwrap();
    
    // Restart (simulate)
    drop(state);
    
    // Reopen storage
    let storage = DiskStorage::new("./test-data").await.unwrap();
    let state = TrackerState::new(Arc::new(CavePool::from_storage(storage)));
    
    // Issue should still exist
    assert!(state.get_issue(issue_id).await.unwrap().is_some());
}
```

---

### Phase 2: Mid-Tier Scaling (Weeks 5-10)

**Goal:** Support 1K tenants on 3 nodes with PostgreSQL, Redis, HA failover.

**Deliverables:**
1. PostgreSQL backend integration
2. Redis cache invalidation (Pub/Sub)
3. Raft consensus for leader election
4. Automatic failover testing
5. Blue-green deployment

**Effort Estimate:** 120 hours (15 person-days)

**Checklist:**
- [ ] Add CAVE_SCALE=mid configuration
- [ ] Integrate PostgreSQL via deadpool-postgres
- [ ] Implement Redis Pub/Sub cache invalidation in FlagsState
- [ ] Integrate cave-ha Raft module into main.rs
- [ ] Update module state initialization to use PostgresStorage
- [ ] Test failover: kill primary, observe promotion of replica
- [ ] Implement blue-green deployment (LoadBalancer shifting)
- [ ] Document Kubernetes manifests (Deployment, Service, StatefulSet for PG)

**Code Changes:**
```rust
// New module: cave-runtime/src/scale/mid.rs
pub async fn init_mid_tier() -> Result<RuntimeState> {
    // 1. Connect to PostgreSQL
    let pg_pool = create_pg_pool(std::env::var("DATABASE_URL")?).await?;
    
    // 2. Run migrations
    cave_db::migrate::run_all_migrations(&pg_pool).await?;
    
    // 3. Init HA (Raft consensus)
    let ha = cave_ha::init_from_env().await?;
    
    // 4. Init Redis for cache invalidation
    let redis = redis::Client::open(std::env::var("REDIS_URL")?)?;
    
    // 5. Init module states with PostgreSQL + caching
    let tracker_state = Arc::new(cave_tracker::TrackerState::new(
        Arc::new(cave_db::persistence::PostgresStorage::new(pg_pool.clone()).await?),
        Some(ha.clone()),
        Arc::new(CacheInvalidator::new(redis.clone())),
    ));
    
    let flags_state = Arc::new(cave_flags::FlagsState::new(
        Arc::new(cave_db::persistence::PostgresStorage::new(pg_pool.clone()).await?),
        Arc::new(CacheInvalidator::new(redis)),
    ));
    
    Ok(RuntimeState { tracker_state, flags_state, ha })
}
```

**Testing:**
```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_mid_tier_failover() {
    // Setup: 3-node Raft cluster + PostgreSQL
    let mut raft_cluster = RaftTestCluster::new(3).await.unwrap();
    let pg = setup_test_postgres().await.unwrap();
    
    // Create issue on leader
    let leader = raft_cluster.current_leader();
    let issue_id = leader.create_issue("tenant-a", Issue::default()).await.unwrap();
    
    // Kill leader
    raft_cluster.kill_node(leader.id()).await;
    sleep(Duration::from_secs(2)).await;
    
    // New leader elected; issue still exists
    let new_leader = raft_cluster.current_leader();
    assert!(new_leader.get_issue(issue_id).await.unwrap().is_some());
}
```

---

### Phase 3: Hyperscale Sharding (Weeks 11-20)

**Goal:** Support 100K tenants on 50 nodes across 3 regions; multi-shard PostgreSQL; Kafka.

**Deliverables:**
1. Tenant-based sharding logic (hash(tenant_id) % 256)
2. Sharded PostgreSQL deployment (256 shards, 3 replicas each)
3. Kafka event streaming (64 partitions)
4. Cell-based failure domain isolation
5. Cross-region replication

**Effort Estimate:** 200 hours (25 person-days)

**Checklist:**
- [ ] Add CAVE_SCALE=hyperscale configuration
- [ ] Implement ShardRouter (routes requests by tenant_id % 256)
- [ ] Update all module states to use ShardedStorage
- [ ] Integrate cave-streams (Kafka) for event-driven data
- [ ] Implement cave-metrics TSDB for logs/metrics/traces
- [ ] Setup cell-based architecture (Cell A/B/C/Backup)
- [ ] Implement cross-region replication
- [ ] Add shard health monitoring and rebalancing
- [ ] Stress test: 100K tenants, 10K QPS across 50 nodes

**Code Changes:**
```rust
// New: cave-runtime/src/scale/hyperscale.rs
pub struct ShardRouter {
    shards: HashMap<u16, ShardEndpoint>,
    local_shard_id: u16,
}

impl ShardRouter {
    pub fn route(&self, tenant_id: &str) -> ShardEndpoint {
        let shard_id = hash(tenant_id) % 256;
        self.shards[&shard_id].clone()
    }
}

pub async fn init_hyperscale_tier() -> Result<RuntimeState> {
    // 1. Determine local shard ID from env (e.g., POD_INDEX -> shard 0-255)
    let local_shard_id = std::env::var("POD_INDEX")?
        .parse::<u16>()? % 256;
    
    // 2. Init shard router (all 256 shards)
    let shard_router = ShardRouter::from_config(&CONFIG).await?;
    
    // 3. Connect to local shard's PostgreSQL (primary + 2 replicas)
    let pg_pool = create_pg_pool(
        format!("postgres://shard-{}.db:5432/cave", local_shard_id)
    ).await?;
    
    // 4. Init Kafka for events
    let kafka = KafkaProducer::new(std::env::var("KAFKA_BROKERS")?).await?;
    
    // 5. Init cell assignment (Cell A/B/C based on region)
    let cell = determine_cell_from_env()?;
    
    // 6. Init module states with sharding awareness
    let tracker_state = Arc::new(TrackerState::with_sharding(
        Arc::new(PostgresStorage::new(pg_pool.clone()).await?),
        Some(ha),
        local_shard_id,
        shard_router.clone(),
        kafka.clone(),
    ));
    
    Ok(RuntimeState { tracker_state, shard_router, kafka, cell })
}

// In module handlers:
pub async fn create_issue(
    State(state): State<Arc<TrackerState>>,
    Path(tenant_id): Path<String>,
    Json(issue): Json<Issue>,
) -> Result<Json<Uuid>> {
    // Check routing
    let shard_id = state.shard_router.route(&tenant_id).shard_id;
    if shard_id != state.local_shard_id {
        return Err(AxumError::Redirect(
            format!("/api/tracker/issues?shard_id={}", shard_id)
        ));
    }
    
    // Write locally
    let issue_id = state.create_issue(&tenant_id, issue).await?;
    
    // Emit to Kafka
    state.kafka.produce(...).await?;
    
    Ok(Json(issue_id))
}
```

**Testing:**
```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_hyperscale_sharding() {
    let mut cluster = HyperscaleCluster::new(50 /* nodes */, 256 /* shards */).await.unwrap();
    
    // Create 10K issues across 1K tenants (each tenant on different shard)
    for tenant_idx in 0..1000 {
        let tenant_id = format!("tenant-{}", tenant_idx);
        for issue_idx in 0..10 {
            let shard_id = hash(&tenant_id) % 256;
            let node = cluster.get_node_for_shard(shard_id);
            node.create_issue(&tenant_id, Issue::default()).await.unwrap();
        }
    }
    
    // Verify sharding: all issues from tenant-N live in same shard
    for tenant_idx in 0..1000 {
        let tenant_id = format!("tenant-{}", tenant_idx);
        let shard_id = hash(&tenant_id) % 256;
        let node = cluster.get_node_for_shard(shard_id);
        let issues = node.list_issues(&tenant_id).await.unwrap();
        assert_eq!(issues.len(), 10);  // All 10 issues found
    }
    
    // Simulate shard failure; verify rebalance
    cluster.kill_shard(0).await;
    sleep(Duration::from_secs(10)).await;
    
    // Queries should still succeed (traffic rerouted to replica)
    let node = cluster.get_node_for_shard(0);
    let issues = node.list_issues("tenant-0").await.unwrap();
    assert!(!issues.is_empty());  // Still accessible
}
```

---

### Phase 4: Global Scale & Optimization (Weeks 21-26)

**Goal:** 1M tenants across 200+ nodes, 4 regions, <10ms p99 latency.

**Deliverables:**
1. Global load balancing (DNS + anycast)
2. Multi-region replication
3. Automatic cell failover
4. Performance optimization (caching, indexing, query planning)
5. Cost optimization (cold cell, resource autoscaling)

**Effort Estimate:** 160 hours (20 person-days)

**Checklist:**
- [ ] Implement global load balancer (GeoDNS or Anycast)
- [ ] Setup multi-region replication (async)
- [ ] Implement cell promotion logic (backup → active in <60s)
- [ ] Add performance profiling (flame graphs, slow query logs)
- [ ] Optimize hot paths (batch writes, SIMD operations)
- [ ] Add resource autoscaling (by QPS, CPU, memory)
- [ ] Implement cost-aware scheduling (cheap regions first)
- [ ] Stress test: 1M tenants at 100K QPS globally

---

### Summary Table

| Phase | Duration | Tenants | Nodes | Key Deliverable |
|-------|----------|---------|-------|-----------------|
| Phase 1 | 4 weeks | 100 | 1 | SQLite edge deployment |
| Phase 2 | 6 weeks | 1K | 3 | PostgreSQL + HA failover |
| Phase 3 | 10 weeks | 100K | 50 | Sharded Kafka-based hyperscale |
| Phase 4 | 6 weeks | 1M | 200+ | Multi-region global |
| **Total** | **26 weeks** | **1M** | **200+** | **Elastic zero-to-billion** |

---

## H. Technology Choices (Justified)

### H.1 Database: PostgreSQL (Not MySQL, Not MongoDB)

**Why PostgreSQL:**
- **Strong consistency:** ACID guarantees; perfect for financial/compliance data
- **Sharding-friendly:** Partition constraints + RLS enable multi-tenant isolation
- **TimescaleDB:** Time-series support without separate database
- **Replication:** WAL streaming + logical replication for DR
- **Rust ecosystem:** Excellent tokio-postgres + sqlx support
- **Cloud-native:** Available in all clouds (RDS, CloudSQL, etc.)

**Cost:** $0.01/GB/month on AWS RDS; scales linearly.

### H.2 Cache: Redis (Not Memcached, Not Varnish)

**Why Redis:**
- **Pub/Sub:** Essential for cache invalidation
- **Expiry:** Native TTL support (Memcached limited)
- **Cluster mode:** Horizontal sharding (3.0+)
- **Persistence:** AOF + RDB for durability
- **Lua scripting:** Atomic operations (e.g., check-and-set)
- **Streams:** Event log alternative to Kafka for low-volume cases

**Limitations:**
- Single-threaded per core (but modern Redis is multi-threaded)
- Memory-bound; expensive at scale (but worth it for latency)

### H.3 Streams: Apache Kafka (Not RabbitMQ, Not NATS)

**Why Kafka:**
- **Partition-based ordering:** Critical for per-tenant event replay
- **Replication:** 3x replication by default
- **Retention policies:** Automatic cleanup
- **Schema Registry:** Avro + Protobuf support
- **Ecosystem:** Kafka Connect, Confluent Cloud, etc.
- **Proven:** Used by Netflix, LinkedIn, Uber at massive scale

**Alternative:** NATS JetStream (lighter-weight, simpler)

### H.4 Consensus: Raft (via cave-ha)

**Why Raft:**
- **Understandable:** Simpler than Paxos
- **Leader-based:** Natural fit for read/write routing
- **Proven:** etcd, Consul use Raft
- **No external quorum store needed:** Unlike Zookeeper

**Alternative:** etcd (more mature, but heavier)

### H.5 Container: OCI (Docker/Podman, not systemd-nspawn)

**Why OCI:**
- **Universal:** Works everywhere (local, Kubernetes, AWS, etc.)
- **Rust support:** Excellent (cargo build → Dockerfile)
- **Alpine base:** ~50MB final image (small, fast)
- **Layers:** Efficient caching during builds

### H.6 Orchestration: Kubernetes (Not Docker Swarm, Not Nomad)

**Why Kubernetes:**
- **De facto standard:** Most users already have it
- **Declarative:** Easy to reason about (YAML)
- **Multi-region:** Federation, ArgoCD for GitOps
- **Ecosystem:** Operators, Helm, kube-proxy, etc.
- **Scaling:** HPA (Horizontal Pod Autoscaler) built-in

**Alternative:** Nomad (more flexible; but Kubernetes is ecosystem win)

---

## I. Operational Runbooks

### I.1 Tier 1 → Tier 2 Upgrade Runbook

```
# Prerequisites
- PostgreSQL cluster running (AWS RDS or managed service)
- Redis cluster running (AWS ElastiCache or managed service)
- Kubernetes cluster with 3 nodes (for Raft quorum)
- Configure env vars:
  - CAVE_SCALE=mid (instead of "edge")
  - DATABASE_URL="postgres://cave:pwd@postgres.db:5432/cave"
  - REDIS_URL="redis://redis.cache:6379"
  - RAFT_PEERS="cave-0.raft,cave-1.raft,cave-2.raft"

# 1. Backup current edge deployment
kubectl exec cave-edge-0 -- sqlite3 /data/cave.db .backup /tmp/backup.db
kubectl cp cave-edge-0:/tmp/backup.db ./cave-backup-$(date +%s).db

# 2. Pre-flight checks
- [ ] PostgreSQL is reachable and empty
- [ ] Redis is reachable
- [ ] Raft quorum nodes are up
- [ ] Network latency: <50ms between nodes

# 3. Dump SQLite to PostgreSQL
cave-runtime migrate --from sqlite:/data/cave.db --to postgres://... --tenant-filter='*'

# 4. Start Raft cluster (3 nodes)
kubectl apply -f deploy/mid-tier/raft-statefulset.yaml

# 5. Upgrade runtime to mid tier
kubectl set env deployment/cave-runtime CAVE_SCALE=mid
kubectl set env deployment/cave-runtime DATABASE_URL=postgres://...
kubectl set env deployment/cave-runtime REDIS_URL=redis://...

# 6. Rollout upgrade (blue-green)
kubectl set image deployment/cave-runtime \
  cave-runtime=cave-runtime:v0.2.0-mid-tier
kubectl rollout status deployment/cave-runtime

# 7. Verify
curl http://cave-runtime/health
curl http://cave-runtime/api/modules | jq .

# 8. Drain edge deployment
kubectl delete deployment cave-edge-0

# 9. Monitor
- [ ] No increase in error rate (< 0.1%)
- [ ] Latency p99 < 100ms
- [ ] All tenants accessible
- [ ] Failover test: kill primary; observe promotion

# Success! 🎉
```

### I.2 Shard Failure Recovery (Hyperscale)

```
# Scenario: Shard 42 primary PostgreSQL instance died

# 1. Detect failure (automated)
Health check fails 3 times (15s)
Alert fires: "Shard 42 unavailable"

# 2. Promote replica to primary (automated)
Failover script triggers:
  ALTER SYSTEM SET recovery_target = 'immediate';
  SELECT pg_ctl_promote();

# 3. Monitor replica promotion
watch -n 1 'psql -c "SELECT pg_is_in_recovery();"'
# Should return false (means it's now primary)

# 4. Update routing config
kubectl patch configmap shard-routing --patch '{"data": {"shard-42": "pg-shard-42-replica-1:5432"}}'

# 5. Verify traffic flows to new primary
watch -n 5 'curl http://shard-42.api/health'

# 6. Repair old primary
- Stop PostgreSQL on dead node
- Check disk/network (likely corrupt index or connection loss)
- Run FSCK if disk corruption suspected
- Restart PostgreSQL in recovery mode
- Run REINDEX on all tables (if needed)
- Resync from new primary:
  rm -rf /var/lib/postgresql/data/*
  pg_basebackup -h pg-shard-42-replica-1 -D /var/lib/postgresql/data
  systemctl restart postgresql

# 7. Verify replica is synced
SELECT now() - pg_last_xact_replay_timestamp() < '1 sec';

# 8. Return to normal topology (optional)
# If you want original primary as primary again:
SELECT pg_promote();  -- on replica
# Then demote current primary (switchover)

# 9. Alert resolution
Auto-triggered when health check passes 3 times (15s)
Slack notification: "Shard 42 recovery complete. RTO: 2 min."

# Success! Shard is now healthy and replicated 3x again.
```

---

## J. Appendix: Glossary

| Term | Definition |
|------|-----------|
| **Cell** | Geographic failure domain; e.g., us-east-1 has 10 nodes + shards 0-85 |
| **Shard** | Horizontal partition; e.g., tenant-abc → shard 42 via hash(tenant_id) % 256 |
| **Replica** | Full copy of shard data; 3x replication = primary + 2 standbys |
| **Leader** | Raft term; the node accepting writes in a quorum |
| **Quorum** | Majority of nodes in consensus; 3/5, 5/9, etc. |
| **RTO** | Recovery Time Objective; max time to restore service |
| **RPO** | Recovery Point Objective; max acceptable data loss |
| **WAL** | Write-Ahead Log; PostgreSQL durability mechanism |
| **MVCC** | Multi-Version Concurrency Control; PostgreSQL isolation level |
| **RLS** | Row-Level Security; PostgreSQL per-row access control |
| **EOS** | Exactly-Once Semantics; no duplicates or losses |
| **Idempotency** | Operation produces same result if executed once or many times |

---

## K. References & Further Reading

- Designing Data-Intensive Applications (DDIA) — Kleppmann
- PostgreSQL 15 official docs — sharding, replication, RLS
- Raft Consensus Algorithm — raft.github.io
- Kafka Architecture — Confluent documentation
- etcd design — coreos/etcd repository
- Kubernetes multi-tenancy — official docs
- TimescaleDB time-series — timescale.com

---

## Conclusion

This architecture enables the CAVE Unified Runtime to scale elastically from 1 tenant on a laptop to 1 million tenants across global regions, **using the same binary and codebase**. The three-tier design—Edge, Mid, Hyperscale—matches deployment complexity to operational capabilities and cost constraints.

The key insight is **automatic tier detection and reconfiguration**: the runtime observes its deployment environment and wires up state management, persistence, and HA mechanisms accordingly. A development team with 10 tenants doesn't pay the operational tax of a 1M-tenant hyperscale system.

Rust's zero-cost abstractions, async/await, and memory safety make this feasible. The `cave-ha`, `cave-store`, `cave-streams`, and `cave-metrics` modules provide the distributed systems primitives; the challenge is integrating them into a coherent, user-facing platform.

Implementation should proceed in phases:
1. **Phase 1 (Weeks 1-4):** Edge tier (SQLite, single pod)
2. **Phase 2 (Weeks 5-10):** Mid tier (PostgreSQL, HA failover)
3. **Phase 3 (Weeks 11-20):** Hyperscale (sharding, Kafka, cells)
4. **Phase 4 (Weeks 21-26):** Global (multi-region, optimization)

This 6-month roadmap puts a 1M-tenant system within reach.

