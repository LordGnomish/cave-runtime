# cave-forensics — Charter v2 8-gate close-out

**Date:** 2026-05-23
**Branch:** `claude/cave-forensics-close-2026-05-23`
**Upstream pin:** cilium/tetragon `v1.7.0` (`1de2ed8ebea18e56257dc59597aa13bf8f0e471e`) — Apache-2.0
**Parity:** `fill_ratio = 0.9545` (42/44) · `honest_ratio = 0.6818` (30/44)

| # | Gate | Status | Evidence |
| - | --- | --- | --- |
| 1 | **Upstream pinned** (always-latest) | PASS | `parity.manifest.toml::[upstream].version = "v1.7.0"` + `source_sha = "1de2ed8e…f8f0e471e"`. `assertion_1_gate_1_upstream_pinned`. |
| 2 | **mapped local_files exist on disk** | PASS | All 30 [[mapped]] `local_files` resolve under `crates/cave-forensics/src/...`. `assertion_2_gate_2_mapped_files_exist`. |
| 3 | **partial has gap reason** | PASS | 0 [[partial]] entries — vacuously true. `assertion_3_gate_3_partial_has_reason`. |
| 4 | **skipped has scope_cut target** | PASS | Every [[skipped]] carries `scope_cut_target` + `reason`. `assertion_4_gate_4_skipped_has_scope_cut`. |
| 5 | **unmapped has honest reason** | PASS | 2 [[unmapped]] (btf_relocation_resolver + tetragon_helm_chart_install) each have a `reason` line. `assertion_5_gate_5_unmapped_has_reason`. |
| 6 | **fill_ratio ≥ 0.95** | PASS | `0.9545` = (30 mapped + 0 partial + 12 skipped) / 44. honest_ratio `0.6818` ≥ 0.65 floor. `assertion_6_gate_6_fill_ratio_meets_floor`. |
| 7 | **AGPL SPDX header coverage 100%** | PASS | All 28 `.rs` files in `src/` + `tests/` carry `SPDX-License-Identifier: AGPL-3.0-or-later`. `assertion_7_gate_7_spdx_coverage`. |
| 8 | **no stub macros in src/** | PASS | No `todo!()` / `unimplemented!()` / `panic!("stub")` / `panic!("not impl")` in `src/**/*.rs`. `assertion_8_gate_8_no_stub_macros`. |

Bonus assertion 9 (Charter v2 surface integrity + audit date): full Tetragon path reachable through `cave_forensics` — tracing-policy → filter → enforcer → export-codecs → case-store + `last_audit = "2026-05-23"`. `assertion_9_surface_integrity_and_audit_date`.

## Subsystem counts

| Bucket | Count | Examples |
| --- | --- | --- |
| Mapped | 30 | tracing_policy_crd, tracing_policy_kprobe_spec, tracing_policy_uprobe_spec, tracing_policy_tracepoint_spec, tracing_policy_lsm_hook_spec, policy_mode_enforce_monitor, pod_selector, container_selector, namespace_selector, process_credentials, process_namespaces, process_tree_reaping, capability_constants, event_process_exec, event_process_exit, event_file_op, event_network, event_capability, event_bpf_load, event_kprobe_uprobe_meta, filter_match_pids, filter_match_namespaces, filter_match_capabilities, filter_match_binaries, filter_match_args, filter_match_actions, enforcer_state_machine, policy_store, export_grpc_codec, export_json_stream |
| Partial | 0 | — |
| Skipped | 12 | ebpf_kernel_loader, ebpf_perf_buffer_reader, ringbuf_consumer, tetragon_grpc_server, tetragon_cli_tetra, fileexport_rate_limiter, hubble_flow_correlation, loki_log_correlation, k8s_audit_correlation, fgs_runtime_metrics_otel, worm_storage_backend, policyfilter_cgroup_v1 |
| Unmapped (honest gaps) | 2 | btf_relocation_resolver, tetragon_helm_chart_install |

## Test totals

| Suite | Pass | Fail | Skip |
| --- | ---: | ---: | ---: |
| Lib unit tests (`cargo test --lib`) | 197 | 0 | 0 |
| `tests/integration.rs` | 6 | 0 | 0 |
| `tests/parity_self_audit.rs` | 9 | 0 | 0 |
| `tests/smoke.rs` | 6 | 0 | 0 |
| **TOTAL** | **218** | **0** | **0** |

## Scope-cuts → Phase 2 owners

| Scope-cut | Target crate | Reason |
| --- | --- | --- |
| `ebpf_kernel_loader` | cave-runtime host-preflight | Kernel-side eBPF C generation requires libbpf-rs + linux headers + root; out of pure-Rust OSS runtime scope. |
| `ebpf_perf_buffer_reader` | cave-runtime host-preflight | Requires libbpf-rs + privileged container. |
| `ringbuf_consumer` | cave-runtime host-preflight | Same — kernel-side BPF map consumer. |
| `tetragon_grpc_server` | cave-portal-api | tonic-based gRPC server transport; codec is mapped, transport is its own concern. |
| `tetragon_cli_tetra` | cave-cli | Standalone Go CLI; replaced by `cavectl forensics` (`src/cli.rs`). |
| `fileexport_rate_limiter` | cave-portal-api | Token-bucket file-export rate limiter; portal-side concern. |
| `hubble_flow_correlation` | cave-net | Hubble network-flow merge lives in cave-net (Hubble role). |
| `loki_log_correlation` | cave-logs | Loki app-log correlation lives in cave-logs. |
| `k8s_audit_correlation` | cave-k8s | K8s audit-log merge lives in cave-k8s. |
| `fgs_runtime_metrics_otel` | cave-metrics | OTel exporter for runtime metrics; cave-metrics already covers. |
| `worm_storage_backend` | cave-artifacts | WORM evidence storage (S3 object-lock); cave-artifacts owns. |
| `policyfilter_cgroup_v1` | cave-runtime | cgroup-v1 path matcher; cave-runtime targets v2-only hosts. |

## Honest unmapped (no automated owner)

| Subsystem | Why unmappable |
| --- | --- |
| `btf_relocation_resolver` | BTF relocation requires kernel BTF + libbpf — a pure-Rust BTF reader is non-trivial. Tracked as a future `cave-bpf-btf` crate, but not in MVP. |
| `tetragon_helm_chart_install` | Helm chart install/upgrade is operator-deployment glue. cave-deploy handles ArgoCD, not Helm — no clean owner today. |

## Four-track breakdown

- **Backend modules (16 .rs in src/):** `case.rs`, `cli.rs`, `enforcer.rs`, `engine.rs`, `error.rs`, `evidence.rs`, `filter.rs`, `lib.rs`, `models.rs`, `observability.rs`, `parity_self_audit.rs`, `process.rs`, `routes.rs`, `selectors.rs`, `store.rs`, `tracing_policy.rs` + 6 under `events/` (`bpf.rs`, `capability.rs`, `file.rs`, `kprobe.rs`, `mod.rs`, `network.rs`, `process_exec.rs`) + 3 under `export/` (`grpc_codec.rs`, `json_stream.rs`, `mod.rs`) — 26 src files / 5390 LOC total.
- **cavectl commands (7 subcommands):** `cavectl forensics {policy,events,filter,enforce,case,observability,help}` via `src/cli.rs::dispatch`.
- **Portal observability artefacts (3):** `src/observability.rs::dashboard_panels()` → 8 panels JSON, `alert_rules()` → 5 alerts, `alert_rules_yaml()` → Prometheus group YAML.
- **Observability metrics (8 panels + 5 alerts):** events / violations / enforcement / cases / evidence / process-tree / followed-fds / policy-install panels + HighViolationRate / EnforcementSpike / CasesOpenTooLong / EventsDried / ProcessTreeRunaway alerts.
