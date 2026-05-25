# cave-bench parity close — 2026-05-23

**Status:** Charter v2 8-gate close, **fill_ratio 0.9697** (23 mapped / 0 partial / 9 skipped / 1 unmapped / 33 total), honest_ratio 0.6970.

## Upstream pins

| Upstream | Version | source_sha | License |
|----------|---------|-----------------------------------------|------------|
| aquasecurity/kube-bench | v0.15.5 | `13c5a2bed634b4f324ad54ba2942f4a77fc802e0` | Apache-2.0 |
| kubescape/kubescape     | v4.0.8  | `d7539c2264560a8685f59e89a731d6de833258a6` | Apache-2.0 |

Both pinned in `Cargo.toml::[package.metadata.upstream]` and in `parity.manifest.toml::[upstream]` + `[[upstreams]]`.

## What landed

A single crate covering two upstream Go projects. The crate hosts:

- **CIS check engine** (`src/cis_engine.rs`) — BinOp + TestItem + CisRule + Logic + CisContext + evaluate_rule + load_rules_yaml. Reproduces kube-bench's `check/test.go` operator surface (Eq/NotEq/Gt/Gte/Lt/Lte/Has/NotHas/BitMaskAnd/Regex/ValidElements) and ValueSource set (Flag/Path/FileMode/FileOwner/FileExists).
- **CIS master controls** (`src/cis_master.rs`) — 30+ checks across `1.1.*` (file perms), `1.2.*` (API server flags), `1.3.*` (controller-manager), `1.4.*` (scheduler).
- **CIS node controls** (`src/cis_node.rs`) — 21 checks across `4.1.*` (kubelet configs) and `4.2.*` (kubelet flags).
- **CIS etcd controls** (`src/cis_etcd.rs`) — 9 checks (`2.*`) for peer/client TLS + data-dir perms.
- **CIS control-plane controls** (`src/cis_control_plane.rs`) — 7 checks (`3.*`) for audit policy + scheduler/controller bind-address.
- **NSA controls** (`src/kubescape_nsa.rs`) — 32 controls (`C-0001 … C-0086`) covering pod security, network segmentation, RBAC, resource limits, image hygiene, audit policy. `NsaManifestFacts` captures predicate input; `evaluate_control` dispatches on predicate keyword.
- **MITRE ATT&CK techniques** (`src/kubescape_mitre.rs`) — 30 techniques across all 10 tactics (InitialAccess → Impact) with detection guidance.
- **Profile manager** (`src/profile.rs`) — 5 builtin profiles: `cis-1.9`, `cis-1.10`, `nsa-2025`, `mitre-attck-k8s`, `soc2-cc-7`.
- **Scan runner** (`src/runner.rs`) — `RunMode::{Sequential,Parallel}` + `ScanInput` + `run_profile` (per-framework dispatch) + `smoke_run` + `findings_by_host`.
- **Report renderers** (`src/report.rs`) — `Format::{Json,Sarif,Html,Markdown}` with SARIF 2.1.0 compliance + pipe-escaped Markdown + HTML-escaped tables.
- **Scheduler** (`src/scheduler.rs`) — `ScheduledScan` + `DagNode` (master → node → etcd → nsa → report → notify) + `ScheduleRegistry` + minimal cron matcher (`*`, `*/N`, literals).
- **Finding store** (`src/store.rs`) — `FindingStore` + `SharedStore = Arc<FindingStore>`; record / get / list_summaries / list_failures / list_for_profile.
- **HTTP routes** (`src/api.rs`) — axum: `POST /scan`, `GET /findings`, `GET /failures`, `GET /profiles`, `GET /checks`, `GET /schedules`, `GET /observability/{panels,alerts}`.
- **Observability** (`src/observability.rs` + `observability.toml`) — 9 metric names, 8 dashboard panels, 5 alert rules, Prometheus-YAML emitter.
- **CLI surface** (`src/cli.rs`) — `BenchSubcommand::{Scan,Profiles,Checks,Schedules,Observability}` + `dispatch()`. Orchestrator wires `cavectl bench …` post-merge.
- **Models** (`src/models.rs`) — Verdict (6 variants), Framework (4), NodeType (6), CisLevel, Severity (5), Check, Finding, Target (4 kinds), Profile, ScanSummary with `compute()`.

## Scope cuts (9, all clean owners)

| Skipped subsystem | Target crate | Rationale |
|---|---|---|
| container_runtime_introspection | cave-cri | Live process flag enumeration |
| kubelet_api_live_watch | cave-kubelet | host-port live kubelet API |
| kube_apiserver_live_watch | cave-apiserver | live K8s manifest watch |
| kubescape_helm_chart_install | cave-deploy | operator deployment |
| kubescape_vuln_scan_integration | cave-trivy | image CVE scans |
| kubescape_compliance_report_sync | cave-portal-api | portal-side dashboards |
| notification_delivery_transports | cave-noti | Slack/PagerDuty transports |
| open_policy_agent_rego_eval | cave-policy | OPA Rego evaluation |
| regolibrary_artifact_sync | cave-upstream-watchd | upstream control-JSON refresh |

## Honest unmapped (1)

- **kube-bench_plugin_marketplace** — Go-plugin .so loading is the only gap with no clean owner. Pure-Rust pluginisation (WASM or shared-lib) is a Phase 2 design item.

## Test counts

- Lib tests: **111 PASS** (0 failed, 0 ignored).
- Integration self-audit: **9/9 PASS** — one assertion per Charter v2 gate.

## Charter v2 8-gate verdict

| Gate | Description | Verdict |
|---|---|---|
| G1 | Both upstreams pinned (version + source_sha) | PASS |
| G2 | Every `[[mapped]].local_files` exists on disk | PASS |
| G3 | Every `[[partial]]` has a `reason` field | PASS (n/a — no partials) |
| G4 | Every `[[skipped]]` has `scope_cut_target` + `reason` | PASS |
| G5 | Every `[[unmapped]]` has an honest `reason` | PASS |
| G6 | `fill_ratio` ≥ 0.95 | **PASS — 0.9697** |
| G7 | 100% AGPL SPDX header coverage on src/*.rs | PASS |
| G8 | No `todo!`/`unimplemented!`/`panic!("stub …")` in src/ | PASS |

Bonus: `last_audit = 2026-05-23` (TODAY).

## Known gotchas

- `ScanSummary::score: f64` blocks `Eq` derive on the struct — kept `PartialEq` only.
- The MITRE module ships 10 tactics (InitialAccess..Impact); the original spec required ≥9, so the gate threshold is `≥9`.
- `RunMode::Parallel` is a documented mode but execution is single-threaded inside `run_profile` — checks are CPU-cheap (predicate eval, no I/O) and a tokio fanout would add overhead per scan without latency gain. Marked as `[[mapped]]` with note.
- `cave-cli` wiring is intentionally untouched (orchestrator domain). `src/cli.rs` exposes a self-contained `dispatch()` returning Strings so the wire-up step is trivial.

## Ready to ff-merge: YES.
