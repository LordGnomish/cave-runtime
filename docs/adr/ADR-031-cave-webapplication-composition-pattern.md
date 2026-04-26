# ADR-031 — Cave WebApplication Composition Pattern

> **Renumber note (2026-04-26):** originally drafted as ADR-145 in this
> repository; renumbered to ADR-031 to resolve a collision with the
> Platform catalog's ADR-145 ("Cave Runtime — Async Runtime
> tokio + io_uring").  Final semantic renumbering is deferred to a
> separate pass; for now, slot 031 (previously empty in the catalog) is
> used.

Status: Proposed (2026-04-26 — Burak finalize edecek)
Scope: Universal (Platform; Pipeline inherits; Runtime via mirror principle ADR-RUNTIME-UPSTREAM-MIRROR-001)
Category: Platform / Application Composition / Tenant Developer Experience
Related: ADR-011 (Backstage), ADR-026 (ArgoCD), ADR-067 (Crossplane v2), ADR-076 (cavectl), ADR-124 (Crossplane MRAP), ADR-013 (LiteLLM Gateway), ADR-027 (Kong API Gateway), ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001, ADR-RUNTIME-CERT-LIFECYCLE-001, ADR-021 (Streaming), ADR-RUNTIME-STREAMING-CONSOLIDATION-001, ADR-008 (Cache), ADR-047 (PostgreSQL), ADR-020 (OpenBao), ADR-053 (ESO), ADR-014 (Zero-Trust), ADR-029 (LGTM Stack), ADR-043 (Schema Migration)

## Context

Tenant Java/Python/Go/Node ile bir REST web service yazdığında, deploy etmek için 9+ component'i manuel orchestre etmesi gerekir:

1. Application workload (Deployment + Service)
2. Gateway expose (HTTPRoute + plugin chain: rate-limit + auth + cors)
3. TLS cert (cert-manager / cave-certs Certificate CRD)
4. DNS record (cave-dns A/CNAME)
5. NetworkPolicy (Cilium default-deny + tenant allowlist)
6. AuthorizationPolicy (cave-mesh)
7. **Database** (Postgres schema + tables + dynamic credentials)
8. **Streaming** (Kafka topic + consumer group + ACL + schema registry)
9. **Cache** (Redis namespace + TTL config)
10. Secrets injection (cave-vault dynamic creds + ESO sync to K8s Secret)
11. Observability (ServiceMonitor + tracing)
12. Backstage entity registration (catalog + dependency graph)

Manuel wire = saatlerce friction + error prone. Tenant DX kötü, platform mantıksız.

**Endüstri pattern:** Score-style workload spec + platform-engineered composition. Tenant intent declare eder, platform reconciles.

## Decision

`WebApplication` Crossplane XR (Composite Resource) — tenant intent + Cave reconciliation:

```yaml
apiVersion: cave.run/v1
kind: WebApplication
metadata:
  name: order-service
  namespace: tenant-acme
spec:
  language: java
  framework: spring-boot
  expose:
    path: /api
    domain: api.acme.cave.run
    auth: oidc
    rate_limit: { rpm: 1000 }
  resources:
    - type: postgres
      schema: order_schema
      tables: [users, orders, line_items]
      backup_retention: 30d
    - type: kafka-topic
      produce: [processed-events]
      subscribe:
        - topic: order-events
          consumer_group: order-processor
          ack: at-least-once
    - type: redis-cache
      ttl: 1h
      max_memory: 256mb
  observability:
    metrics: prometheus
    traces: opentelemetry
    logs: loki
  replicas: { min: 2, max: 10, target_cpu: 70 }
```

Cave reconciles 12-component bundle.

## 3 surface (unified)

Tenant 3 alternatif giriş noktası:

### 1. Backstage Software Template (cave-portal Software Templates page)

Wizard adımları:
- Language picker (Java + Spring/Quarkus, Python + FastAPI/Flask, Go + Gin/Echo, Node + Express/NestJS)
- Resource picker (Postgres? Kafka? Redis? S3? Vector DB?)
- Schema/topic naming (defaults from convention, ADR-021 governance)
- Auth flow (OIDC scopes, ADR-PORTAL-AUTH-001)
- Endpoint config (path, domain, rate limit)
- Replication strategy
- Generates: project skeleton + WebApplication YAML + git push initial commit

### 2. cavectl CLI

```bash
cavectl create webapp \
  --name order-service \
  --lang java \
  --framework spring-boot \
  --persist postgres:order_schema:users,orders,line_items \
  --stream kafka:produce=processed-events,subscribe=order-events:order-processor \
  --cache redis:ttl=1h \
  --expose api.acme.cave.run/api --auth oidc \
  --replicas 2-10 \
  --output ./order-service/
```

Aynı çıktı (project + manifests + git push).

### 3. Crossplane XR (declarative GitOps)

Tenant manuel `WebApplication` YAML yazar, ArgoCD sync eder. Power-user path.

## Reconciliation flow

WebApplication XR reconciler (Crossplane Composition):

1. **Crossplane resource provisioning:**
   - cave-pg `Database` CRD: schema + tables (Flyway/Alembic migrations queued in CI step)
   - cave-streams `KafkaTopic` CRD: topic + ACL + schema registry entry
   - cave-cache `RedisNamespace` CRD: namespace + TTL config
   - cave-vault `DynamicCredential` CRD: rotating DB cred (default 24h TTL)
   - ESO syncs creds → K8s Secret (`{name}-secrets`)

2. **Application workload:**
   - Deployment with secret env injection (DATABASE_URL, KAFKA_BOOTSTRAP_SERVERS, REDIS_URL, OIDC_CLIENT_SECRET)
   - Service (ClusterIP)
   - HPA (replicas min/max/target_cpu)

3. **Network exposure:**
   - HTTPRoute (Cilium GatewayAPI) → Service backend
   - cave-gateway plugin chain: rate-limit + key-auth/oauth2/jwt + cors + transformations
   - cave-certs Certificate CRD → TLS cert (sovereign CA per-tenant)
   - cave-dns DNSRecord CRD → A/CNAME for domain

4. **Security:**
   - NetworkPolicy: default-deny + ingress from gateway + egress to listed deps
   - cave-mesh AuthorizationPolicy: source identity check + JWT claim verify

5. **Observability:**
   - ServiceMonitor (cave-metrics scrape)
   - OTel SDK auto-instrument (lang-specific)
   - Loki log forwarder annotation

6. **Catalog:**
   - Backstage entity registered (Component kind + dependsOn relationships)
   - Dependency graph viz: app → DB + Kafka + Cache (cave-portal `/admin/code-intel` ile, ADR-144 entegrasyon)

## CI/CD integration

Pipeline (ADR-010 27-stage CI):
- Pre-deploy: schema migration runs (Flyway/Alembic, ADR-043) — DB tablolarını yaratır/günceller
- Pre-deploy: Kafka topic ACL update + schema registry sync
- Pre-deploy: Trivy image scan (ADR-018)
- Deploy: ArgoCD sync (ADR-026)
- Post-deploy: smoke test + DAST OWASP ZAP (ADR-023)
- Post-deploy: Backstage entity update

## Industry pattern reference

(Specific vendor adı verilmiyor — common practice)

Sektörde olgunlaşmış yaklaşım:
- Workload spec abstraction (CNCF Score)
- Composite Resource patterns (Crossplane Compositions)
- Software Templates + Service Catalog
- Dependency-driven CI/CD

Cave bu desenleri sovereign Rust reimpl + multi-tenant first-class + PQC-ready zorluğunda birleştirir.

## Pipeline + Runtime Inheritance

- **Pipeline-platform-starter:** ADR-031'i inherits eder. Pipeline'ın kendisi Cave WebApplication pattern'iyle yaratılan + maintained app (CI/CD pipeline'ın self-hosting'ı bu compose pattern'inin instance'ı).
- **Runtime:** `cave-app` (XR reconciler) + `cave-app-templates/*` (Java/Python/Go/Node skeletons) + `cavectl create webapp` wizard + `cave-portal` Software Templates integration. ADR-RUNTIME-UPSTREAM-MIRROR-001 charter principle altında otomatik. Multi-tenant + PQC charter-default. **Yeni Runtime override ADR YOK.**

`cave-app` crates plan:
- `cave-app` — WebApplication XR + Composition reconciler
- `cave-app-templates-java` (Spring Boot, Quarkus)
- `cave-app-templates-python` (FastAPI, Flask)
- `cave-app-templates-go` (Gin, Echo)
- `cave-app-templates-node` (Express, NestJS)
- `cavectl create webapp` (cave-cli extension)

## Reddedilen Alternatifler

- **Tenant manual wire 12-component** — DX kötü, error-prone, friction high
- **Sadece Helm chart per app** — declarative ama composition + dependency provisioning yok (Postgres schema, Kafka ACL, dynamic creds bağımsız manuel)
- **Vendor SaaS application orchestrator** (Humanitec/Octopus/Spinnaker) — sovereign violation
- **Sadece Backstage template (no Crossplane reconciliation)** — scaffolding'de generate ama runtime drift yok, dependency'ler tenant manuel yönetir
- **Sadece Crossplane XR (no template)** — declarative ama tenant'ın 200-satır YAML yazması gerekiyor, DX kötü

## Consequences

### Positive
- Tenant DX dramatik iyileşir — saatler → dakikalar
- Platform-engineered (no copy-paste boilerplate)
- Dependency consistency garantili (schema migration + ACL + dynamic creds otomatik)
- Multi-tenant isolation strict (tenant_id propagated her component'e)
- Backstage catalog auto-populate (dependency graph viz)
- CI/CD pre-deploy steps standardize (migration + ACL update)

### Negative
- Crossplane XR composition complexity (yeni pattern öğrenme)
- Template maintenance (yeni framework versiyonları için)
- Dependency lifecycle edge case'ler (DB schema rollback, topic ACL revoke)

### Risks
- Application drift (tenant manuel değişiklik) → continuous reconciliation
- Schema migration failure → blue-green deployment + rollback
- Dynamic credential rotation race → graceful drain + retry

## Compliance

- SOC2 CC8.1 — change management automated
- ISO 27001 A.14.2 — secure development (template-driven)
- GDPR Art.25 — data protection by design (schema + access control automated)
- NIS2 Art.21 — security measures (default-deny + auto-encryption)
- PCI-DSS Req 6.4 — change control (CI/CD pipeline integration)

## Implementation Phases

**v0.1 (this OSS launch):**
- Crossplane WebApplication XR Composition (basic: Postgres + Kafka + Redis + Deployment + Service)
- Backstage Software Template (Java + Spring Boot only)
- cavectl create webapp basic (Java + Spring Boot)

**v0.2 (post-launch):**
- Multi-language templates (Python + Go + Node)
- Multi-framework (FastAPI/Flask, Gin/Echo, Express/NestJS)
- Vector DB resource (cave-iceberg + Qdrant)
- S3 / Object Storage resource (cave-blobs)
- Search resource (cave-search)

**v0.3:**
- ML inference endpoint resource (cave-ml integration)
- Multi-region deployment
- Workflow resource (Argo Workflows binding)
- Cron resource

## Status update plan

ADR-031 v0.1 implement: Crossplane Composition + cave-app crate scaffold + Backstage template + cavectl basic. Sonraki Sonnet sprint'lerinde wave by wave.
