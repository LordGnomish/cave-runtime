# CAVE Runtime Architecture

## Overview

CAVE Runtime is a single Rust binary that boots an Internal Developer Platform (IDP) on port 8080. It reimplements 70+ upstream open-source tools as lightweight modules, each exposed via REST API and unified under JWT authentication.

## Design Principles

1. **Single binary, zero dependencies** — no Docker, no Kubernetes, no external databases required to boot
2. **Sovereign by default** — every component self-hosted, zero vendor lock-in
3. **Wire-compatible** — MongoDB (port 27017) and PostgreSQL (port 5432) wire protocols for native client compatibility
4. **Lazy initialization** — modules boot with mock pools when no database is configured
5. **Module isolation** — each cave-* crate owns its routes, state, and storage

## Binary Architecture

```
                    ┌─────────────────────────────────┐
                    │        cave-runtime (main)       │
                    │    axum 0.8 + tower middleware    │
                    │         JWT auth layer           │
                    ├─────────────────────────────────┤
                    │   /api/auth/*    (cave-auth)     │
                    │   /api/tracker/* (cave-tracker)  │
                    │   /api/registry/*(cave-registry)  │
                    │   /api/scan/*   (cave-scan)      │
                    │   /api/vault/*  (cave-vault)     │
                    │   /api/deploy/* (cave-deploy)    │
                    │   /api/flags/*  (cave-flags)     │
                    │   /portal/*     (cave-portal)    │
                    │   ... 70+ module routers ...     │
                    ├─────────────────────────────────┤
                    │  Wire Protocol Servers (tokio)   │
                    │  :27017 MongoDB (cave-docdb)     │
                    │  :5432  PostgreSQL (cave-rdbms)  │
                    └─────────────────────────────────┘
```

## Workspace Structure

```
cave-runtime/
├── crates/
│   ├── cave-runtime/     # Binary entry point, router assembly
│   ├── cave-auth/        # JWT middleware, OIDC, dev token
│   ├── cave-db/          # Database pool (CavePool, mock support)
│   ├── cave-core/        # Shared config, platform profile
│   ├── cave-tracker/     # Issue tracking (Jira/Linear reimpl)
│   ├── cave-registry/    # Container registry + scan pipeline
│   ├── cave-scan/        # SAST scanner (50 rules, 6 languages)
│   ├── cave-vault/       # Secrets management (OpenBao-compatible)
│   ├── cave-deploy/      # Deployment orchestration
│   ├── cave-flags/       # Feature flags (Unleash-compatible)
│   ├── cave-portal/      # Developer portal UI
│   ├── cave-pipelines/   # CI/CD pipeline engine
│   ├── cave-docdb/       # MongoDB wire protocol
│   ├── cave-rdbms/       # PostgreSQL wire protocol
│   ├── cave-crossplane/  # Infrastructure provisioning
│   └── ... (78 crates total)
├── docs/
│   └── adr/              # 123 Architecture Decision Records
├── .github/workflows/    # CI (cargo check + test + clippy)
└── Cargo.toml            # Workspace manifest
```

## Authentication Flow

All API routes pass through JWT middleware except bypass paths:
- `GET /` — portal redirect
- `GET /health` — health check
- `GET /portal/*` — static portal UI
- `POST /api/auth/token` — dev token endpoint (CAVE_DEV_MODE only)

JWT tokens use HS256 with `CAVE_JWT_SECRET` env var. Claims include `sub`, `email`, `roles`, `exp`.

## Module Pattern

Each module follows the same pattern:

```rust
// State initialization
let state = cave_module::ModuleState::default(); // uses CavePool::mock() if no DB

// Router creation
let router = cave_module::router(state);

// Merged into main app
let app = Router::new()
    .merge(router)
    .layer(auth_middleware);
```

## Wire Protocols

- **cave-docdb**: MongoDB OP_MSG (opcode 2013) on port 27017. Supports `find`, `insert`, `aggregate`, `createIndexes`.
- **cave-rdbms**: PostgreSQL v3 wire protocol on port 5432. Supports simple query, extended query, transactions.

## Related Documents

- [ADR Index](./docs/adr/README.md) — 123 Architecture Decision Records
- [CONTRIBUTING.md](./CONTRIBUTING.md) — How to contribute
