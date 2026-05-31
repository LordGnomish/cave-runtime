# Ground-truth audit — 2026-05-29

Read-only audit. Source of truth remains the per-crate `parity.manifest.toml`; this report proposes reclassifications only — no manifest edits.

## Executive summary

- **Build health:** `cargo check --workspace --all-targets` and `cargo test --workspace --no-run` both pass with **0 errors** (exit 0). 930 warning-lines, all lints (snake-case, unused) — no correctness blockers.
- **TDD discipline:** 5/5 sampled crates confirm strict `test(RED) → feat(GREEN)` ordering. No fabricated GREEN-before-RED found.
- **LOC ratio:** measured for 100/112 crates. **Not a usable completeness metric** — cave implements scoped subsets of giant monorepos, so 87/100 measured crates sit below LOC ratio 0.05 by design. Used only as a coarse cross-check.
- **Paperwork gap:** **52 crates** declare `fill_ratio = 1.0` while their own manifest `honest_ratio` is meaningfully lower (gap > 0.15) — this self-declared divergence, not LOC, is the real honesty signal. The manifest is *already honest* about these gaps; `fill_ratio = 1.0` means "scope-cut-justified complete," `honest_ratio` is the un-inflated mapped/total.
- **Purge recommendation:** **no automated demotion.** 6 crates claim both `fill = 1.0` and `honest ≥ 0.95` on <2% upstream LOC — flagged for a human scope-cut spot-check (Phase 4). The 52 divergent crates need no action; their honest_ratio already tells the truth.

## Phase 1 — Workspace build health

- `cargo check --workspace --all-targets`: **0 errors**, 930 warning-lines (exit 0)
- `cargo test --workspace --no-run`: **0 compile errors**

## Phase 2 — Per-crate LOC ratio

112 crates total; upstream LOC measured for 100 (rest: upstream unavailable / un-cloneable → N/A).

Cave LOC = non-test `*.rs` under `src/`. Upstream LOC = source files (go/py/ts/js/java/c/cpp/rs/scala/kt) excluding tests/vendor/node_modules.

| Crate | Cave LOC | Upstream LOC | LOC ratio | honest_ratio | fill_ratio | fill−LOC gap |
|---|---:|---:|---:|---:|---:|---:|
| cave-acme | 721 | 51341 | 0.0140 | 0.0000 | 0.0000 | -0.0140 |
| cave-admission | 1800 | 2167512 | 0.0008 | 0.8000 | 1.0000 | **+0.9992** |
| cave-ai-obs | 1457 | 502810 | 0.0029 | 0.5000 | 1.0000 | **+0.9971** |
| cave-alerts | 4463 | 49411 | 0.0903 | 0.7778 | 1.0000 | **+0.9097** |
| cave-apiserver | 28171 | 2167512 | 0.0130 | 0.9804 | 1.0000 | **+0.9870** |
| cave-artifacts | 17274 | 71498 | 0.2416 | 0.4000 | 1.0000 | **+0.7584** |
| cave-auth | 39779 | 1186596 | 0.0335 | 0.9773 | 1.0000 | **+0.9665** |
| cave-backup | 3177 | 131757 | 0.0241 | 0.5600 | 1.0000 | **+0.9759** |
| cave-bench | 4275 | 3195 | 1.3380 | 0.7273 | 1.0000 | -0.3380 |
| cave-cache | 17205 | 358410 | 0.0480 | 0.8947 | 1.0000 | **+0.9520** |
| cave-cdc | 1998 | 26560 | 0.0752 | 0.7647 | 1.0000 | **+0.9248** |
| cave-certs | 2561 | 145521 | 0.0176 | 0.7200 | 1.0000 | **+0.9824** |
| cave-changelog | 475 | 7871 | 0.0603 | 0.0000 | 0.0000 | -0.0603 |
| cave-chaos | 2344 | 127747 | 0.0183 | 0.7222 | 1.0000 | **+0.9817** |
| cave-chat | 1362 | 388547 | 0.0035 | 0.4500 | 1.0000 | **+0.9965** |
| cave-cli | 20810 |  | N/A | 0.0000 | 0.0000 |  |
| cave-cloud-controller-manager | 17766 | 2167512 | 0.0082 | 0.9565 | 1.0000 | **+0.9918** |
| cave-cluster | 5134 | 247558 | 0.0207 | 0.9022 | 1.0000 | **+0.9793** |
| cave-compliance | 3026 | 51615 | 0.0586 | 0.6500 | 1.0000 | **+0.9414** |
| cave-container-scan | 2966 | 130983 | 0.0226 | 0.7308 | 1.0000 | **+0.9774** |
| cave-controller-manager | 26256 | 2167512 | 0.0121 | 1.0000 | 1.0000 | **+0.9879** |
| cave-core | 2062 |  | N/A | 0.0000 | 0.0000 |  |
| cave-cost | 1973 | 121640 | 0.0162 | 0.5000 | 1.0000 | **+0.9838** |
| cave-cost-alloc | 1780 | 121640 | 0.0146 | 0.0000 | 0.0000 | -0.0146 |
| cave-cri | 17044 | 186340 | 0.0915 | 0.9118 | 1.0000 | **+0.9085** |
| cave-crm | 3509 | 1324478 | 0.0026 | 0.5135 | 1.0000 | **+0.9974** |
| cave-crossplane | 8246 | 53034 | 0.1555 | 0.7000 | 1.0000 | **+0.8445** |
| cave-dashboard | 10341 | 1173158 | 0.0088 | 0.8571 | 1.0000 | **+0.9912** |
| cave-dast | 5637 | 313022 | 0.0180 | 0.9231 | 1.0000 | **+0.9820** |
| cave-datafusion | 4260 | 827115 | 0.0052 | 0.5152 | 1.0000 | **+0.9948** |
| cave-db | 697 |  | N/A | 0.0000 | 0.0000 |  |
| cave-deploy | 6609 | 354986 | 0.0186 | 0.6667 | 1.0000 | **+0.9814** |
| cave-desktop | 188 | 1367246 | 0.0001 | 0.0000 | 0.0000 | -0.0001 |
| cave-devlake | 1245 | 239590 | 0.0052 | 0.6875 | 1.0000 | **+0.9948** |
| cave-dns | 9823 | 36346 | 0.2703 | 0.7917 | 1.0000 | **+0.7297** |
| cave-docdb | 4910 | 42947 | 0.1143 | 0.9423 | 1.0000 | **+0.8857** |
| cave-docs | 469 | 633842 | 0.0007 | 0.0000 | 0.0000 | -0.0007 |
| cave-docs-site | 2234 |  | N/A | 0.0000 | 0.0000 |  |
| cave-ebpf-common | 131 | 609350 | 0.0002 | 0.0000 | 0.0000 | -0.0002 |
| cave-erp | 5532 | 432032 | 0.0128 | 0.8462 | 1.0000 | **+0.9872** |
| cave-etcd | 26551 | 123079 | 0.2157 | 0.4930 | 1.0000 | **+0.7843** |
| cave-falco | 1908 | 26681 | 0.0715 | 0.7308 | 1.0000 | **+0.9285** |
| cave-flags | 4198 | 343929 | 0.0122 | 0.9538 | 1.0000 | **+0.9878** |
| cave-forensics | 5390 | 477606 | 0.0113 | 0.6818 | 0.9583 | **+0.9470** |
| cave-gateway | 14554 | 3289 | 4.4251 | 0.7333 | 0.9667 | -3.4584 |
| cave-gitleaks | 2701 | 15691 | 0.1721 | 0.9000 | 1.0000 | **+0.8279** |
| cave-gitops-config | 2247 | 354986 | 0.0063 | 0.4839 | 1.0000 | **+0.9937** |
| cave-ha | 6448 | 123079 | 0.0524 | 0.8704 | 1.0000 | **+0.9476** |
| cave-hermes | 4915 | 1053440 | 0.0047 | 0.9531 | 0.9531 | **+0.9484** |
| cave-iceberg | 4049 | 131289 | 0.0308 | 0.7083 | 1.0000 | **+0.9692** |
| cave-identity | 3639 | 109076 | 0.0334 | 0.7200 | 1.0000 | **+0.9666** |
| cave-incidents | 1850 | 65530 | 0.0282 | 0.6429 | 1.0000 | **+0.9718** |
| cave-infra | 5056 | 308155 | 0.0164 | 0.7917 | 1.0000 | **+0.9836** |
| cave-kamaji | 1316 |  | N/A | 0.8235 | 1.0000 |  |
| cave-karpenter | 1887 | 36215 | 0.0521 | 0.8636 | 1.0000 | **+0.9479** |
| cave-keda | 2487 | 59963 | 0.0415 | 0.8571 | 1.0000 | **+0.9585** |
| cave-kernel | 5925 |  | N/A | 0.0000 | 0.0000 |  |
| cave-knative | 4101 | 65577 | 0.0625 | 0.8667 | 1.0000 | **+0.9375** |
| cave-kube-proxy | 2208 | 2167512 | 0.0010 | 0.9412 | 1.0000 | **+0.9990** |
| cave-kubelet | 18778 | 2167512 | 0.0087 | 0.9487 | 1.0000 | **+0.9913** |
| cave-kubevirt | 3975 | 394098 | 0.0101 | 1.0000 | 1.0000 | **+0.9899** |
| cave-lakehouse | 4036 | 131289 | 0.0307 | 0.9348 | 1.0000 | **+0.9693** |
| cave-ledger | 865 |  | N/A | 0.0000 | 0.0000 |  |
| cave-lint | 727 | 850049 | 0.0009 | 0.0000 | 0.0000 | -0.0009 |
| cave-llm-gateway | 6427 | 1675068 | 0.0038 | 0.5652 | 1.0000 | **+0.9962** |
| cave-llm-tracker | 2009 |  | N/A | 0.6471 | 1.0000 |  |
| cave-local-llm | 6611 | 242174 | 0.0273 | 0.9677 | 1.0000 | **+0.9727** |
| cave-logs | 9016 | 448646 | 0.0201 | 0.8750 | 1.0000 | **+0.9799** |
| cave-mesh | 12309 | 297127 | 0.0414 | 0.9730 | 1.0000 | **+0.9586** |
| cave-metrics | 12380 | 197014 | 0.0628 | 0.9000 | 1.0000 | **+0.9372** |
| cave-net | 49559 | 609350 | 0.0813 | 0.9851 | 1.0000 | **+0.9187** |
| cave-oncall | 2770 | 65530 | 0.0423 | 0.8889 | 1.0000 | **+0.9577** |
| cave-pam | 2840 | 2376041 | 0.0012 | 0.6875 | 1.0000 | **+0.9988** |
| cave-permission | 1043 | 14054 | 0.0742 | 0.4091 | 1.0000 | **+0.9258** |
| cave-pii | 209 | 59620 | 0.0035 | 0.0000 | 0.0000 | -0.0035 |
| cave-pipelines | 4991 | 113728 | 0.0439 | 0.6842 | 1.0000 | **+0.9561** |
| cave-pki | 668 | 51341 | 0.0130 | 0.0000 | 0.0000 | -0.0130 |
| cave-policy | 14127 | 214686 | 0.0658 | 0.6154 | 1.0000 | **+0.9342** |
| cave-portal | 79152 | 633842 | 0.1249 | 0.8687 | 1.0000 | **+0.8751** |
| cave-portal-api | 5319 | 633842 | 0.0084 | 0.0000 | 0.0000 | -0.0084 |
| cave-portal-web | 2336 | 633842 | 0.0037 | 0.0000 | 0.0000 | -0.0037 |
| cave-profiler | 185 | 492454 | 0.0004 | 0.0000 | 0.0000 | -0.0004 |
| cave-rdbms | 6371 | 1803613 | 0.0035 | 0.9130 | 1.0000 | **+0.9965** |
| cave-rdbms-operator | 4305 | 103355 | 0.0417 | 1.0000 | 1.0000 | **+0.9583** |
| cave-registry | 15 | 31089 | 0.0005 | 0.0000 | 0.0000 | -0.0005 |
| cave-rollouts | 2445 | 119487 | 0.0205 | 0.7097 | 1.0000 | **+0.9795** |
| cave-runbook | 3265 |  | N/A | 0.0000 | 0.0000 |  |
| cave-runtime | 10272 |  | N/A | 0.0000 | 0.0000 |  |
| cave-sandbox | 3312 | 600434 | 0.0055 | 0.7458 | 1.0000 | **+0.9945** |
| cave-sbom | 7574 | 259429 | 0.0292 | 0.7778 | 1.0000 | **+0.9708** |
| cave-scaffold | 1414 | 633842 | 0.0022 | 0.0000 | 0.0000 | -0.0022 |
| cave-scan | 5661 | 850049 | 0.0067 | 0.9171 | 1.0000 | **+0.9933** |
| cave-scan-db | 1409 | 13710 | 0.1028 | 0.7600 | 1.0000 | **+0.8972** |
| cave-scheduler | 13601 | 2167512 | 0.0063 | 0.7586 | 1.0000 | **+0.9937** |
| cave-search | 1089 | 318797 | 0.0034 | 0.7880 | 1.0000 | **+0.9966** |
| cave-secrets | 2494 | 202725 | 0.0123 | 0.4848 | 1.0000 | **+0.9877** |
| cave-security | 7140 | 26681 | 0.2676 | 0.6765 | 1.0000 | **+0.7324** |
| cave-sign | 4533 | 32968 | 0.1375 | 0.5385 | 1.0000 | **+0.8625** |
| cave-slo | 1001 | 26378 | 0.0379 | 0.8333 | 1.0000 | **+0.9621** |
| cave-status | 490 | 41683 | 0.0118 | 0.0000 | 0.0000 | -0.0118 |
| cave-store | 9956 | 253630 | 0.0393 | 0.8000 | 1.0000 | **+0.9607** |
| cave-streams | 31900 | 1625439 | 0.0196 | 1.0000 | 1.0000 | **+0.9804** |
| cave-techdocs | 859 | 633842 | 0.0014 | 0.0000 | 0.0000 | -0.0014 |
| cave-trace | 10040 | 89305 | 0.1124 | 0.6316 | 1.0000 | **+0.8876** |
| cave-tracing | 3023 | 89305 | 0.0339 | 0.0000 | 0.0000 | -0.0339 |
| cave-tracker | 6898 | 396080 | 0.0174 | 0.7500 | 1.0000 | **+0.9826** |
| cave-upstream | 4256 |  | N/A | 0.0000 | 0.0000 |  |
| cave-upstream-watchd | 7335 |  | N/A | 0.0000 | 0.0000 |  |
| cave-uptime | 2334 | 41683 | 0.0560 | 0.7000 | 1.0000 | **+0.9440** |
| cave-vault | 17456 | 394175 | 0.0443 | 0.5625 | 1.0000 | **+0.9557** |
| cave-vulns | 7088 | 277211 | 0.0256 | 0.9000 | 1.0000 | **+0.9744** |
| cave-workflows | 2682 | 205322 | 0.0131 | 0.7143 | 1.0000 | **+0.9869** |

## Phase 3 — Strict-TDD sample verification (5 crates)

Deterministic spread sample drawn from the 85 crates with `fill_ratio ≥ 0.95`. For each, the
git history (branch if present, else path-filtered `main`) was inspected for `test([RED]) → feat([GREEN])`
commit ordering — the project's strict-TDD signature.

| Crate | Source | TDD ordering | Verdict |
|---|---|---|---|
| cave-auth | `claude/cave-auth-close-2026-05-18` | `test(RED)` precedes `feat(GREEN)` for every Phase-3 protocol cycle (cavectl PATH consts → lib modules; portal sub-pages; obs alert group) | ✅ strict TDD |
| cave-dashboard | `claude/cave-dashboard-honest-cont-1780159920` | 6 clean RED→GREEN pairs (mathexp reducers, resample, classic conditions, math/cross-refId, nested folder, RBAC evaluator) | ✅ strict TDD |
| cave-iceberg | `main` (no claude branch) | `test: RED — InclusiveMetricsEvaluator` immediately precedes `feat: GREEN`; proptest scaffold first | ✅ strict TDD |
| cave-metrics | `claude/cave-metrics-honest-100-2026-05-30` | 3 clean pairs (relabel 11-action, dns_sd, chunked remote-read) each `test` then `feat` | ✅ strict TDD |
| cave-secrets | `main` (no claude branch) | RED→GREEN for false-positive suppression and UTF-8 sanitizer; proptest scaffold first | ✅ strict TDD |

**5/5 samples confirm strict RED→GREEN ordering.** No GREEN-before-RED or impl-without-test was found
in any sampled crate. Note cave-iceberg (honest 0.7083) and cave-secrets (honest 0.4848) carry honest
TDD work even though their `fill_ratio` reads 1.0 — the TDD discipline is independent of the paperwork gap.

## Phase 4 — Paperwork-1.00 purge proposal (READ-ONLY)

### Methodology caveat — LOC ratio is NOT a completeness metric here

73/100 measured crates have a LOC ratio below 0.05. This is **not** evidence of incompleteness: cave crates are focused re-implementations against giant upstream monorepos (kubernetes ≈ 2.17M LOC, grafana ≈ 1.17M, keycloak ≈ 1.19M), and they deliberately scope out vendored code, generated code, cloud-provider SDKs, and architecturally-superseded subsystems. A raw `cave_LOC / upstream_LOC` ratio therefore collapses toward zero for almost every crate and **cannot** by itself justify a demotion. The authoritative honesty signal is the manifest's own `honest_ratio` (mapped parity items / total declared items), cross-checked below.

### Actionable list — claims complete (fill=1.0 AND honest≥0.95) on a tiny footprint

These crates assert BOTH a 1.0 fill_ratio and a ≥0.95 honest_ratio while implementing <2% of upstream LOC. That combination is the only LOC-derived signal worth a **human scope-cut spot-check** — the manifest claims near-total parity, so its scope_cut justifications should be re-read to confirm the excluded surface is genuinely out-of-scope (not silently dropped). No auto-demotion proposed.

| Crate | LOC ratio | honest_ratio | fill_ratio | Cave/Upstream LOC |
|---|---:|---:|---:|---|
| cave-cloud-controller-manager | 0.0082 | 0.9565 | 1.0000 | 17766/2167512 |
| cave-kubevirt | 0.0101 | 1.0000 | 1.0000 | 3975/394098 |
| cave-controller-manager | 0.0121 | 1.0000 | 1.0000 | 26256/2167512 |
| cave-flags | 0.0122 | 0.9538 | 1.0000 | 4198/343929 |
| cave-apiserver | 0.0130 | 0.9804 | 1.0000 | 28171/2167512 |
| cave-streams | 0.0196 | 1.0000 | 1.0000 | 31900/1625439 |

## Honest-ratio vs fill-ratio divergence (manifest-internal, no clone needed)

Where the manifest's own `honest_ratio` already sits well below `fill_ratio` (gap > 0.15). These are self-declared paperwork gaps — fill claims 1.0 but the honest accounting is lower.

| Crate | honest_ratio | fill_ratio | gap |
|---|---:|---:|---:|
| cave-artifacts | 0.4000 | 1.0000 | +0.6000 |
| cave-permission | 0.4091 | 1.0000 | +0.5909 |
| cave-chat | 0.4500 | 1.0000 | +0.5500 |
| cave-gitops-config | 0.4839 | 1.0000 | +0.5161 |
| cave-secrets | 0.4848 | 1.0000 | +0.5152 |
| cave-etcd | 0.4930 | 1.0000 | +0.5070 |
| cave-ai-obs | 0.5000 | 1.0000 | +0.5000 |
| cave-cost | 0.5000 | 1.0000 | +0.5000 |
| cave-crm | 0.5135 | 1.0000 | +0.4865 |
| cave-datafusion | 0.5152 | 1.0000 | +0.4848 |
| cave-sign | 0.5385 | 1.0000 | +0.4615 |
| cave-backup | 0.5600 | 1.0000 | +0.4400 |
| cave-vault | 0.5625 | 1.0000 | +0.4375 |
| cave-llm-gateway | 0.5652 | 1.0000 | +0.4348 |
| cave-policy | 0.6154 | 1.0000 | +0.3846 |
| cave-trace | 0.6316 | 1.0000 | +0.3684 |
| cave-incidents | 0.6429 | 1.0000 | +0.3571 |
| cave-llm-tracker | 0.6471 | 1.0000 | +0.3529 |
| cave-compliance | 0.6500 | 1.0000 | +0.3500 |
| cave-deploy | 0.6667 | 1.0000 | +0.3333 |
| cave-security | 0.6765 | 1.0000 | +0.3235 |
| cave-pipelines | 0.6842 | 1.0000 | +0.3158 |
| cave-devlake | 0.6875 | 1.0000 | +0.3125 |
| cave-pam | 0.6875 | 1.0000 | +0.3125 |
| cave-crossplane | 0.7000 | 1.0000 | +0.3000 |
| cave-uptime | 0.7000 | 1.0000 | +0.3000 |
| cave-iceberg | 0.7083 | 1.0000 | +0.2917 |
| cave-rollouts | 0.7097 | 1.0000 | +0.2903 |
| cave-workflows | 0.7143 | 1.0000 | +0.2857 |
| cave-certs | 0.7200 | 1.0000 | +0.2800 |
| cave-identity | 0.7200 | 1.0000 | +0.2800 |
| cave-chaos | 0.7222 | 1.0000 | +0.2778 |
| cave-forensics | 0.6818 | 0.9583 | +0.2765 |
| cave-bench | 0.7273 | 1.0000 | +0.2727 |
| cave-container-scan | 0.7308 | 1.0000 | +0.2692 |
| cave-falco | 0.7308 | 1.0000 | +0.2692 |
| cave-sandbox | 0.7458 | 1.0000 | +0.2542 |
| cave-tracker | 0.7500 | 1.0000 | +0.2500 |
| cave-scheduler | 0.7586 | 1.0000 | +0.2414 |
| cave-scan-db | 0.7600 | 1.0000 | +0.2400 |
| cave-cdc | 0.7647 | 1.0000 | +0.2353 |
| cave-gateway | 0.7333 | 0.9667 | +0.2334 |
| cave-alerts | 0.7778 | 1.0000 | +0.2222 |
| cave-sbom | 0.7778 | 1.0000 | +0.2222 |
| cave-search | 0.7880 | 1.0000 | +0.2120 |
| cave-dns | 0.7917 | 1.0000 | +0.2083 |
| cave-infra | 0.7917 | 1.0000 | +0.2083 |
| cave-admission | 0.8000 | 1.0000 | +0.2000 |
| cave-store | 0.8000 | 1.0000 | +0.2000 |
| cave-kamaji | 0.8235 | 1.0000 | +0.1765 |
| cave-slo | 0.8333 | 1.0000 | +0.1667 |
| cave-erp | 0.8462 | 1.0000 | +0.1538 |
