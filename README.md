# CAVE Platform Runtime

A unified, modular Rust runtime for cloud-native security, observability, and compliance. Single binary, 79 composable modules, extensible API surface for platform engineering at scale.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/cave-runtime/cave-runtime/actions/workflows/ci.yml/badge.svg)](https://github.com/cave-runtime/cave-runtime/actions)

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    CAVE Runtime Binary                       │
│                  (Single Deployable Unit)                    │
└────────────────────────┬────────────────────────────────────┘
                         │
        ┌────────────────┼────────────────┐
        │                │                │
        ▼                ▼                ▼
   ┌─────────┐      ┌─────────┐      ┌──────────┐
   │ Infra & │      │Security │      │Platform  │
   │Network  │      │& Auth   │      │Intel     │
   │ Modules │      │Modules  │      │Modules   │
   └────┬────┘      └────┬────┘      └──────┬───┘
        │                │                   │
        └────────────────┼───────────────────┘
                         │
        ┌────────────────┼────────────────┐
        │                │                │
        ▼                ▼                ▼
   REST API         WebSocket         gRPC
   /health          /metrics          /stream
   /modules         /logs             /events
   /config          /traces           /status
```

## Quick Start

### Build

```bash
cd /path/to/cave-runtime
CAVE_JWT_SECRET=dev-secret cargo build --release -p cave-runtime
```

### Run

```bash
CAVE_JWT_SECRET=dev-secret CAVE_DEV_MODE=true \
  ./target/release/cave-runtime --port 8080
```

### Health Check

```bash
curl -s http://127.0.0.1:8080/health | jq .
```

Example response:
```json
{
  "status": "healthy",
  "uptime_secs": 42,
  "modules_loaded": 79,
  "version": "0.1.0"
}
```

## Module Organization

**79 modules across 8 categories:**

- **Infra & Networking** (8): Gateway, DNS, Mesh, Cluster, HA, Infra, CrossPlane, Container Scanning
- **Security & Auth** (7): Core, Auth, Admission, Security, Vault, Certs, Sign
- **Data & Storage** (4): PostgreSQL, Cache, Object Store, Streams
- **Observability** (4): Metrics, Logs, Traces, Profiler
- **Compliance & Governance** (11): Compliance, Policy, Runbook, GitOps Config, Cost Allocation, Backup, Tracker, Ledger, OnCall, ERP, DocDB
- **Security Scanning** (8): Vulnerabilities, SBOM, Forensics, DAST, PAM, eBPF Common, Scan, Container Scan
- **Platform Intelligence** (15): DevLake, AI Observability, PII Detection, Incidents, Chat, SLO, Alerts, Workflows, Portal, Scaffold, Chaos, LLM Gateway, Uptime, Cost, RDBMS
- **Developer Experience** (6): CLI, Dashboard, Docs Site, Deploy, Pipelines, Rollouts

## Key Features

- **Single Binary**: All 79 modules compiled into one statically-linked executable
- **Modular Architecture**: Each module is independently versioned and testable
- **Composable APIs**: REST, WebSocket, and gRPC endpoints per module
- **Zero-Copy Observability**: Built-in metrics, logging, and distributed tracing
- **Cloud-Native**: Kubernetes-ready, supports HA clusters, multi-region deployment
- **Security-First**: Mandatory JWT auth, RBAC, secrets vault, vulnerability scanning
- **Extensible**: Plugin system for custom compliance policies and security rules
- **Type-Safe**: Full Rust type system with compile-time guarantees

## Compatible With

This runtime provides unified interfaces compatible with:

- **Kubernetes** (native resource types via custom operators)
- **Prometheus** (metrics export)
- **OpenTelemetry** (distributed tracing and logs)
- **PostgreSQL** (primary data store)
- **Docker** (containerization and scanning)
- **GitHub Actions** (CI/CD workflows)
- **Vault** (secrets management)
- **OWASP SBOM** (software bill of materials)
- **CIS Benchmarks** (compliance scanning)
- **eBPF** (kernel-level observability)
- **gRPC** (service-to-service communication)

## Development

### Prerequisites

- Rust 1.85+ ([rustup](https://rustup.rs/))
- PostgreSQL 14+ (for local dev)
- Docker (for running in containers)

### Build All Crates

```bash
CAVE_JWT_SECRET=dev-secret cargo build --workspace
```

### Run Tests

```bash
CAVE_JWT_SECRET=dev-secret cargo test --workspace --lib
```

### Lint & Format

```bash
cargo fmt --check --all
cargo clippy --workspace -- -D warnings
```

### Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on submitting changes.

## Configuration

Set environment variables to customize behavior:

| Variable | Default | Purpose |
|----------|---------|---------|
| `CAVE_JWT_SECRET` | *(required)* | JWT signing key (any string for dev) |
| `CAVE_DEV_MODE` | `false` | Disable auth checks (dev only) |
| `CAVE_PORT` | `8080` | HTTP server listen port |
| `CAVE_DB_URL` | `postgres://localhost` | PostgreSQL connection string |
| `RUST_LOG` | `info` | Tracing filter (trace, debug, info, warn, error) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | *(optional)* | OpenTelemetry collector endpoint |

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

---

**Built with security, observability, and compliance in mind.**
