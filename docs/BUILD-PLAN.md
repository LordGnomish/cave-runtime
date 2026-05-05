# CAVE Runtime — Prioritized Build Plan (Sovereign Reimplementation)

**Generated:** 2026-04-13
**Approach:** Every OSS tool from the CAVE platform is reimplemented in Rust+eBPF as a single binary. Upstream tracked for API/protocol compat, our own code ships.
**Current state:** 67 crates, ~178K lines, 14 GOOD reimplementations, 20 stubs, 17 missing

---

## Phase 0: Foundation Fixes (Week 1–4)

These are architectural pieces everything else depends on.

### 0.1 Profile Configuration System ✅ DONE
- `cave-core/src/profile.rs` — 621 lines
- 7 profiles, resource sizing, module enable/disable, identity/DB backend per profile

### 0.2 Tenant Context & Model ✅ DONE
- `cave-core/src/tenant.rs` — 261 lines
- 3 tiers (soft/hard/dedicated), lifecycle, classification, rate limits, egress

### 0.3 Sovereign Ledger ✅ DONE
- `cave-ledger` — 821 lines
- Merkle hash chain, SHA-256, append-only, entry types, storage backends

### 0.4 Authentication Middleware
**Priority: CRITICAL**
- Wire cave-auth (4,997 lines) as Axum middleware layer
- OIDC token verification + cave_uid extraction (ADR-129)
- RBAC enforcement per route (5 roles from One-Prompt)
- Profile-aware provider selection (Keycloak vs Okta)
- **Estimate:** ~1,200 lines in cave-auth + cave-runtime

### 0.5 Database Abstraction Layer
**Priority: CRITICAL**
- Refactor cave-pg to provider-agnostic interface
- RLS per tenant, migration framework
- CloudNativePG path (Hetzner) vs Azure PG Flexible path
- SQLite for local profile (from Elastic Scale Tier 1)
- **Estimate:** ~2,000 lines

### 0.6 Upstream Tracker Expansion ✅ DONE
- `cave-upstream/src/projects.rs` expanded from 26 to 73 projects
- All components from One-Prompt now tracked with category and phase

---

## Phase 1: Grow the 20 Stubs (Week 5–12)

Each stub (<1K lines) needs to become a real Rust reimplementation of its upstream.
Ordered by platform phase dependency.

### Priority 1: Phase 1 Core stubs

| Crate | Upstream | Current | Target | Key Work |
|-------|----------|---------|--------|----------|
| cave-secrets | Trufflehog | 374 | 2,500 | Entropy analysis, pattern matching, multi-language |
| cave-certs | cert-manager | 256 | 3,000 | ACME client, X.509 lifecycle, CA integration |
| cave-sign | Sigstore cosign | 183 | 3,000 | Keyless signing, SLSA provenance, verification |
| cave-admission | Policy Controller | 981 | 3,000 | Image sig verify, mutation webhooks |
| cave-lint | Hadolint | 291 | 2,000 | Dockerfile AST parser, rule engine |
| cave-status | (native) | 274 | 1,500 | Status page, component health aggregation |
| cave-changelog | (native) | 300 | 1,500 | Conventional commits, release notes gen |

### Priority 2: Phase 2 Data/AI stubs

| Crate | Upstream | Current | Target | Key Work |
|-------|----------|---------|--------|----------|
| cave-pii | Presidio | 204 | 2,500 | NER recognizers, anonymization, 10+ languages |
| cave-chat | LibreChat | 206 | 3,000 | Multi-provider chat, conversation store, plugins |
| cave-ai-obs | Langfuse | 201 | 2,500 | LLM trace capture, evaluation framework |

### Priority 3: Phase 3 Advanced stubs

| Crate | Upstream | Current | Target | Key Work |
|-------|----------|---------|--------|----------|
| cave-incidents | Grafana OnCall | 452 | 3,000 | Escalation chains, schedule, routing |
| cave-slo | (native + k6) | 409 | 2,500 | SLO budget, burn rate, error budget tracking |
| cave-alerts | (native) | 508 | 2,500 | Alert routing, grouping, silencing |
| cave-chaos | Chaos Mesh | 420 | 3,000 | Network/pod/IO chaos, SLO-aware injection |
| cave-workflows | n8n/Argo WF | 501 | 4,000 | DAG execution engine, triggers, artifact passing |
| cave-vulns | DefectDojo | 384 | 3,000 | Finding management, dedup, import formats |
| cave-sbom | DependencyTrack | 434 | 3,000 | CycloneDX/SPDX parse, NVD/OSV correlation |
| cave-scan | Trivy/Sonar | 392 | 3,000 | Container scan, SAST, misconfiguration rules |
| cave-dast | ZAP | 304 | 3,000 | HTTP fuzzing, OpenAPI-driven scanning |
| cave-forensics | Tetragon/Hubble | 210 | 4,000 | eBPF hooks, syscall tracing, flow capture |
| cave-pam | Teleport | 353 | 5,000 | Session recording, JIT, break-glass, audit |
| cave-devlake | DevLake | 205 | 2,500 | DORA metrics, data connectors, calculation |
| cave-uptime | Uptime Kuma | 299 | 2,000 | HTTP/TCP/DNS monitors, notifications |
| cave-profiler | Pyroscope | 180 | 2,500 | eBPF profiling, pprof, flame graph gen |

**Estimated total for stubs:** ~57,000 new lines

---

## Phase 2: Create 17 Missing Crates (Week 13–24)

New crates for components that have no Rust reimplementation yet.

### HIGH Priority (blocks tenant workflows)

| New Crate | Upstream | Est. Lines | Key Capabilities |
|-----------|----------|-----------|-----------------|
| cave-search | OpenSearch + Qdrant + Faiss + Milvus | 11,000 | Inverted index, BM25, analyzers (OpenSearch); HNSW + IVF/PQ vector ANN (Qdrant/Faiss/Milvus); hybrid lexical + vector search. Sovereign OSS only. |
| cave-vcluster | vcluster | 4,000 | Virtual K8s control plane, resource sync |
| cave-crossplane | Crossplane v2 | 5,000 | XRD engine, Composition Functions, MRAP |
| cave-opal | OPAL | 3,000 | Real-time policy push, external data sources |

### MEDIUM Priority

| New Crate | Upstream | Est. Lines | Key Capabilities |
|-----------|----------|-----------|-----------------|
| cave-thanos | Thanos | 4,000 | Store API, global query, compaction |
| cave-hubble | Cilium Hubble | 3,000 | Flow capture, service map, L7 visibility |
| cave-gitea | Gitea | 5,000 | Git protocol, API, webhooks, LFS |
| cave-keda | KEDA | 3,000 | ScaledObject, event triggers, metrics server |
| cave-arc | ARC | 2,500 | Runner registration, ephemeral lifecycle |
| cave-renovate | Renovate | 3,000 | Manager framework, versioning, auto-merge |
| cave-k6 | k6 | 2,500 | JS runtime (QuickJS), checks, thresholds |

### LOW Priority (Phase 4)

| New Crate | Upstream | Est. Lines | Key Capabilities |
|-----------|----------|-----------|-----------------|
| cave-spark | Spark Op | 3,000 | Job submission, resource management |
| cave-jupyter | JupyterHub | 2,000 | Notebook spawner, kernel management |
| cave-mlflow | MLflow | 3,000 | Experiment tracking, model registry |
| cave-knative | Knative | 3,000 | Serverless serving, autoscale-to-zero |
| cave-databricks | Databricks | 2,000 | Managed API wrapper (Azure path only) |

**Estimated total for new crates:** ~63,500 lines

---

## Phase 3: Expand 7 MEDIUM Crates (Week 13–20, parallel with Phase 2)

These have meaningful code but need to grow 2-3x.

| Crate | Upstream | Current | Target | Key Expansion |
|-------|----------|---------|--------|--------------|
| cave-flags | Unleash | 2,945 | 6,000 | Strategy engine, client SDK protocol, A/B testing |
| cave-llm-gateway | LiteLLM/Ollama | 3,684 | 8,000 | Classification routing, fallbacks, budget mgmt |
| cave-backup | Velero | 2,662 | 6,000 | CSI snapshots, schedule, cross-cloud restore |
| cave-cost | OpenCost | 1,965 | 5,000 | Cloud pricing APIs, attribution model |
| cave-cost-alloc | (native) | 1,737 | 4,000 | Per-tenant unit economics, kill switch |
| cave-rollouts | Argo Rollouts | 1,894 | 5,000 | Canary analysis, Istio traffic split |
| cave-portal | Backstage | 1,072 | 8,000 | Catalog, scaffolder, plugin framework |

**Estimated total for medium expansion:** ~26,000 lines

---

## Phase 4: cave-ctl Full Command Surface (Week 10–24, parallel)

Expand cave-cli from 1,416 lines to ~15,000+. The One-Prompt documents ~60 subcommands.

### Command Groups (ordered by priority)
1. **profile** — create, local, list, switch, show, promote (~1,500 lines)
2. **tenant** — create, delete, promote, budget, egress (~2,000 lines)
3. **xr** — create db/bucket/cache/messagebus/search/vectordb (~1,500 lines)
4. **stack** — deploy core/data/ai/auth/cicd (~1,000 lines)
5. **doctor** — profile health check, drift detection (~1,000 lines)
6. **finops** — report, pnl, budget (~1,000 lines)
7. **ledger** — list, verify, export (~800 lines)
8. **compliance** — export soc2/iso27001/nis2/gdpr (~1,000 lines)
9. **pam** — sessions, request, approve (~800 lines)
10. **incident** — list, create, escalate (~800 lines)
11. **chaos** — status, pause, resume, inject (~800 lines)
12. **reflex** — list, history, dry-run (~800 lines)
13. **apol** — status, override, fallback (~800 lines)
14. **identity** — dormant, recertify, jit, drift (~800 lines)
15. **MCP Server mode** — expose all commands as MCP tools (~2,000 lines)

---

## Infrastructure (Parallel Track, Ongoing)

| Item | Est. | Priority |
|------|------|----------|
| Dockerfile (multi-stage, distroless) | 100 lines | HIGH |
| Helm chart (profile-aware values) | 500 lines | HIGH |
| GitHub Actions CI (start with 10 of 27 stages) | 800 lines | HIGH |
| ADR documents (130+ stubs) | 130 files | MEDIUM |
| Remaining 36 runbook sections | 36 files | MEDIUM |

---

## Total Estimated Work

| Work Item | Lines | Duration |
|-----------|-------|----------|
| Phase 0 remainder (auth + DB) | ~3,200 | 2 weeks |
| Phase 1 (grow 20 stubs) | ~57,000 | 8 weeks |
| Phase 2 (17 new crates) | ~63,500 | 12 weeks |
| Phase 3 (expand 7 medium) | ~26,000 | 8 weeks |
| Phase 4 (cave-ctl commands) | ~15,000 | 8 weeks |
| Infrastructure | ~1,400 + docs | ongoing |
| **TOTAL** | **~166,100** | **~24 weeks parallel** |

### Projected Codebase at Completion
- **~84 crates** (67 current + 17 new)
- **~344,000 lines of Rust** (178K current + 166K new)
- **73 upstream OSS projects** with sovereign Rust reimplementation
- **Single binary** compiled from all crates, profile-driven module loading
- **130+ ADRs** documenting every decision

---

## Immediate Next Actions

1. **Phase 0.4** — Auth middleware (wire cave-auth into main router)
2. **Phase 0.5** — DB abstraction (provider-agnostic cave-pg)
3. **Start stubs in parallel** — cave-sign, cave-certs, cave-secrets (Phase 1 security core)
4. **Dockerfile** — verify single binary compiles and runs in container
