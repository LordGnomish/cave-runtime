<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Upstream TDD coverage audit — summary

Line-by-line audit of each cave crate's upstream test corpus vs cave's test
inventory, filling missing behavioral coverage via strict test-first TDD.

- Branch: `claude/tdd-audit-2026-05-30` (off main `40ba2a98`)
- Merged to `main` through `c3bd5c99` (4 batches, fast-forward, no force-push)
- Method: parallel gap-analysis + test authoring (Workflow); serial
  build → commit-test-first → `cargo test --release` → ff-merge in one warm worktree.

## Totals

| Metric | Value |
|--------|-------|
| Crates audited | 29 |
| Crates filled | 28 |
| Honest 0-fill (no portable surface) | 1 (cave-cluster) |
| **Behavioral tests added** | **251** |
| Source-impl changes to satisfy tests | **0** |
| Dev-dep additions (async tests) | 1 (`tokio` → cave-registry) |
| TDD compliance (test commit precedes any impl) | 100% |

Every gap closed was **portable-coverage**: a public cave function already
implemented but lacking a behavioral test. Because cave's implementations were
already correct, the tests pass on first run — the honest TDD outcome for a
mature codebase. Two integration REDs were caught and both proved to be **wrong
test expectations** (self-inconsistent assertions from the authoring pass), not
cave bugs; corrected to the verified contract:
- `cave-pii::engine::redact` keeps a 2-char prefix/suffix (not 1).
- `cave-registry` GC `blobs_retained` excludes the manifest (separate store).

## Coverage matrix (crate × upstream × tests added)

| Crate | Upstream | Tests | Impl |
|-------|----------|------:|-----:|
| cave-cost-alloc | OpenCost v1.108.0 | 11 | 0 |
| cave-cost | OpenCost v1.108.0 | 10 | 0 |
| cave-compliance | OPA Gatekeeper v3.17.1 | 7 | 0 |
| cave-acme | smallstep/certificates v0.30.2 | 9 | 0 |
| cave-pki | smallstep/certificates v0.30.2 | 7 | 0 |
| cave-pii | Microsoft Presidio v2.2.0 | 11 | 0 |
| cave-gitops-config | Argo CD v3.4.2 | 9 | 0 |
| cave-profiler | Grafana Pyroscope v1.3.0 | 9 | 0 |
| cave-status | Uptime Kuma v1.23.0 | 9 | 0 |
| cave-registry | distribution v3.1.1 | 8 | 0 |
| cave-permission | Casbin v3.10.0 | 8 | 0 |
| cave-rollouts | Argo Rollouts v1.9.0 | 18 | 0 |
| cave-workflows | Argo Workflows v4.0.5 | 14 | 0 |
| cave-pipelines | Tekton Pipelines v0.55.0 | 10 | 0 |
| cave-ha | etcd v3.5.13 | 13 | 0 |
| cave-cdc | Debezium Server v3.5.0 | 8 | 0 |
| cave-cluster | Cluster API v1.6.0 | 0 (scope-cut) | 0 |
| cave-changelog | towncrier 25.8.0 | 9 | 0 |
| cave-slo | nobl9-go v0.126.1 | 9 | 0 |
| cave-container-scan | Trivy v0.70.0 | 9 | 0 |
| cave-scan-db | trivy-db | 7 | 0 |
| cave-ai-obs | Langfuse v3.75.1 | 6 | 0 |
| cave-incidents | Grafana OnCall v1.10.0 | 12 | 0 |
| cave-kamaji | Kamaji v1.0.0 | 3 | 0 |
| cave-backup | Velero | 6 | 0 |
| cave-tracing | Jaeger v2.17.0 | 6 | 0 |
| cave-dns | CoreDNS v1.14.3 | 12 | 0 |
| cave-certs | cert-manager v1.17.2 | 6 | 0 |
| cave-identity | SPIRE v1.15.0 | 5 | 0 |

Per-crate behavioral-gap detail (upstream test → cave coverage classification)
lives in `docs/audit/tdd/<crate>-gaps.md`.

## Remaining work (~54 external-upstream crates)

Dominated by large web-app / k8s monorepos (Backstage ×3, SonarQube, Kubernetes,
Terraform, LibreChat, ERPNext, Twenty, DevLake, MinIO, DataFusion, …) whose
upstream test corpora are overwhelmingly UI / controller / CRD-conversion
plumbing — i.e. **scope-cut** for cave's focused backend subset, with low
portable-coverage yield. The audit machinery (`/tmp/tdd-audit/upstream-inventory.sh`
+ the batched gap-analysis/author Workflow) is reusable to continue.
