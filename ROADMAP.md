# CAVE Runtime Roadmap

## Current: Pre-Release (Target: May 21, 2026)

### OSS launch readiness
- [x] LICENSE (Apache-2.0)
- [x] NOTICE (upstream attribution summary)
- [x] CODE_OF_CONDUCT.md (Contributor Covenant v2.1)
- [x] AUTHORS, CREDITS.md
- [x] docs/upstream-attribution.md (per-crate table generated from `parity.manifest.toml`)
- [x] Provider-specific ADRs moved to `docs/adr/internal/`
- [ ] `cave-runtime cluster init` subcommand — single-node bootstrap (WIP)

### Post-quantum crypto
PQC migration is on the roadmap, not complete. ML-KEM / ML-DSA / SLH-DSA
adoption is staged across `cave-crypto`, `cave-vault`, and `cave-auth`. See
[`ADR-RUNTIME-CERT-LIFECYCLE-001`](docs/adr/ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md)
for the sovereign cert hierarchy + PQC + multi-DNS ACMEv2 plan. Classical-
crypto paths remain for compatibility during the migration; please flag any
production-only-classical paths.

### Tier 0 — Core (Complete)
- [x] Single binary boot on port 8080
- [x] JWT authentication middleware with bypass paths
- [x] Developer portal with module dashboard
- [x] CavePool mock pattern for DB-free boot
- [x] Health check endpoint

### Tier 1 — Essential Modules (80%+ parity target)
- [x] cave-auth — JWT, OIDC, dev token endpoint
- [x] cave-tracker — Issue tracking with portal UI
- [x] cave-registry — Container registry + pull-through proxy + scan pipeline
- [x] cave-scan — SAST (50 rules, 6 languages) + coverage import
- [x] cave-vault — Secrets management (OpenBao-compatible API)
- [x] cave-deploy — Deployment orchestration
- [x] cave-flags — Feature flags
- [x] cave-pipelines — CI/CD pipeline engine
- [x] cave-crossplane — Infrastructure provisioning

### Tier 2 — Extended Modules (In Progress)
- [ ] cave-docdb — MongoDB wire protocol (OP_MSG)
- [ ] cave-rdbms — PostgreSQL wire protocol v3
- [ ] cave-streams — Kafka+Pulsar consolidated streaming
- [ ] cave-gateway — Kong+Gravitee consolidated API gateway
- [ ] cave-oncall — Incident management
- [ ] cave-erp — ERP integration (Odoo-compatible)
- [ ] cave-container-scan — Container image vulnerability scanning
- [ ] cave-ledger — Sovereign audit ledger

### Documentation
- [x] ~120 Architecture Decision Records — scope-classified; provider-specific deploy ADRs archived in `docs/adr/internal/` (see [docs/adr/README.md](docs/adr/README.md))
- [x] README, LICENSE, CONTRIBUTING, CI
- [ ] Per-module API documentation
- [ ] Operations runbook
- [ ] Local development guide

## Phase 2: Beta

- [ ] Full Tier 2 module completion
- [ ] End-to-end integration tests
- [ ] Performance benchmarks
- [ ] Multi-tenant isolation verification
- [ ] Security audit

## Phase 3: GA

- [ ] Production deployment guides (Hetzner + Azure)
- [ ] Helm chart / Kustomize manifests
- [ ] Plugin system for custom modules
- [ ] Upgrade / migration tooling
