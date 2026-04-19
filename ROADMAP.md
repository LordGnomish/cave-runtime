# CAVE Runtime Roadmap

## Current: Pre-Release (Target: May 14, 2026)

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
- [x] 123 Architecture Decision Records (scope-classified)
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
