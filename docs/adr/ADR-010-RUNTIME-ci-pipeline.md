# ADR-010-RUNTIME — Multi-Dimensional Future-Proof CI Pipeline (Runtime variant)

**Status:** Accepted
**Scope:** Cave Runtime (sovereign, self-hosted — no public-cloud dependency)
**Category:** CI/CD / Supply-chain / Charter
**Decided:** 2026-06-07 (Burak Tartan)
**Variant-of:** Platform ADR-010 (Multi-Dimensional Future-Proof CI Pipeline — 17-dim, 7-phase, ~55-stage Azure reference)
**License:** AGPL-3.0-or-later

## Executive Summary

Platform ADR-010 tanımladığı **17-boyutlu, 7-fazlı, ~55-stage** "future-proof" CI
pipeline'ı **Azure-resident** primitive'lere (GitHub Actions + ARC, Managed
Identity, Defender for Containers, Azure Policy, Key Vault Managed HSM, Azure
Monitor) dayanıyordu. Cave Runtime **sovereign** bir Cloud OS'tür (bkz.
[ADR-RUNTIME-STACK-001](ADR-RUNTIME-STACK-001-cave-runtime-stack-architecture.md))
ve hiçbir public-cloud control-plane'e bağımlı olamaz.

Bu ADR aynı **17-boyutlu future-proofing** garantisini koruyan, fakat her
Azure'a özgü primitive'i **cave-native** karşılığıyla değiştiren **Runtime
variant**'ı bağlayıcı olarak tanımlar. Universal backbone'un **45 stage**'i
aynen korunur; Azure'un 10 cloud-managed add-on stage'i Cave'in sovereign
karşılıklarıyla **~2-3 stage**'e konsolide edilir.

**Toplam Runtime variant: ~45 universal + 2-3 sovereign add-on = ~47-48 stage.**

Pipeline self-hosted **Argo Workflows** (cave-knative üzerinde) ile yürür; runner
kimliği **SPIFFE/SPIRE** (cave-identity) ile bağlanır; FRA → HEL multi-region DR
drill bir first-class deploy stage'idir. Cave hiçbir zaman dışarı bir yere bağımlı
olmadan kendi pipeline'ını tam olarak çalıştırabilir — **air-gap-capable by
construction**.

## Context

### Neden bir Runtime variant gerekli

Platform ADR-010 mükemmel bir **referans tasarım**dır, ama Azure'a kök salmıştır:

- GitHub Actions + ARC → GitHub'a (control-plane) bağımlı; sovereignty ihlali.
- Managed Identity dual-binding → Entra ID'ye bağımlı; cave-identity ile çakışır.
- Defender for Containers, Azure Policy, Key Vault Managed HSM, Azure Monitor →
  hepsi Azure regional control-plane'e bağlı, billing'i Azure'a akıtır, ve
  air-gap senaryosunda çalışmaz.

Cave Runtime [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) gereği Hetzner
(FRA/HEL) bare-metal üzerinde, kendi reimpl'leriyle çalışır. CI pipeline'ı da
**aynı sovereignty postürüne** uymak zorundadır: pipeline'ın hiçbir stage'i
Cave kümesinin dışında bir managed servise reach-out edemez.

### Korunan değer: 17-dim future-proofing

Variant **boyut sayısını düşürmez**. 17 boyutun her biri Azure variant'taki ile
aynı garantiyi sunar; sadece *implementation primitive* sovereign hale gelir.
Bu, pipeline'ın 2026-ötesi tehdit modeline (PQC transition, multi-arch
genişleme, supply-chain SLSA L4, AI-assisted attack/defense) hazır kalmasını
sağlar — Azure'a kilitlenmeden.

## Decision

**Argo Workflows** (cave-knative serving üzerinde, cave-mesh ile bağlı) Cave
Runtime'ın yürütücüsü olur. 17-boyut korunur, 7-faz korunur, ~47-stage
materialize edilir. Her Azure-specific primitive aşağıdaki tabloyla **cave-native**
karşılığına map'lenir.

### Azure → Runtime cave-native substitution

| # | Azure (Platform variant) | Runtime cave-native karşılık | Substitution gerekçesi |
|---|---|---|---|
| 1 | GitHub Actions + ARC | **Argo Workflows** (cave-knative üzerinde) | Self-hosted, sovereign, GitHub control-plane bağımlılığı yok |
| 2 | Azure Managed Identity dual-binding | **cave-identity** (SPIFFE/SPIRE pure, ek binding yok) | Tek kimlik düzlemi; Entra ID çakışması ortadan kalkar |
| 3 | Defender for Containers | **cave-defender** (Trivy + Grype + Kubescape consolidated) | Üç OSS scanner tek sovereign surface; Azure billing yok |
| 4 | Azure Policy | **Kyverno + cave-policy** (OPA Gatekeeper alternative) | Admission + supply-chain policy, in-cluster, air-gap-capable |
| 5 | Azure Key Vault Managed HSM | **cave-vault** (OpenBao parity, PQC ML-DSA hybrid) | Sovereign secret + imza anahtarı; PQC seal ([[cave-vault-pqc-seal]]) |
| 6 | Azure Monitor + Application Insights | **cave-metrics + cave-logs + cave-trace** (Prometheus + Loki + Tempo parity) | Üç-pillar gözlemlenebilirlik, in-cluster, sovereign |
| 7 | Azure Container Apps WASI | **cave-knative serving** (WASI runtime) | Serverless WASI workload, sovereign serving |
| 8 | Azure regional residency | **Hetzner FRA + HEL** (multi-region sovereign) | EU-sovereign veri ikametgâhı; FRA→HEL DR failover |

> Cosign keyless OIDC imzası Azure variant'ta GitHub OIDC'ye bağlıydı. Runtime'da
> imza **cave-identity SPIFFE SVID** + **cave-vault**'ta tutulan **ML-DSA hybrid
> PQC** anahtarıyla yapılır (keyful-but-sovereign + transparency log = cave-ledger).

## 17-Dimension Future-Proof Coverage Matrix (Runtime-adapted)

Her boyut korunur; sütun "Runtime mechanism" Azure primitive yerine cave-native
mekanizmayı gösterir.

| # | Future-proof dimension | Runtime mechanism (cave-native) | Phase(s) | Gate |
|---|---|---|---|---|
| 1 | **Supply-chain integrity** | SLSA L4 provenance + in-toto + cosign ML-DSA hybrid → cave-ledger | 3 | BLOCK |
| 2 | **Post-quantum readiness** | ML-DSA (Dilithium) hybrid imza + ML-KEM seal (cave-vault) | 3 | BLOCK |
| 3 | **Multi-architecture** | x86_64 + ARM64 + **RISC-V** (Hetzner native builds) | 2, 3 | BLOCK |
| 4 | **Reproducibility** | Hermetic, network-isolated, build-twice byte-diff | 3 | BLOCK |
| 5 | **Identity & zero-trust** | cave-identity SPIFFE/SPIRE runner SVID, no static creds | 1 | BLOCK |
| 6 | **Policy-as-code** | Kyverno + cave-policy (OPA) admission + supply-chain | 1, 4 | BLOCK |
| 7 | **Vulnerability posture** | cave-defender (Trivy + Grype + Kubescape) + VEX suppression | 2 | BLOCK |
| 8 | **SBOM & transparency** | CycloneDX SBOM gen + diff + VEX → cave-ledger | 2, 3 | WARN→BLOCK |
| 9 | **AI-assisted quality** | cave-agent PR review + AI workload eval + flaky detect | 1, 4, 7 | WARN |
| 10 | **Progressive delivery** | ArgoCD sync (cave-knative) + Argo Rollouts canary + blue-green | 5 | BLOCK |
| 11 | **Resilience / chaos** | cave-chaos fault injection + multi-region DR drill (FRA→HEL) | 5 | BLOCK |
| 12 | **Performance regression** | Baseline + memory gate + startup-time gate + SLO statistical | 4, 5 | BLOCK |
| 13 | **Runtime hardening** | rootless distroless, CIS/NSA/MITRE verify, seccomp/AppArmor | 3, 4 | BLOCK |
| 14 | **Observability SLI/SLO** | cave-metrics + cave-logs + cave-trace real-time SLI/SLO | 6 | WARN |
| 15 | **Cost & sustainability** | cave-metrics cost tracking (Hetzner flat-rate, no cloud egress) | 6 | WARN |
| 16 | **Compliance & licensing** | REUSE/SPDX license gate + Charter v2 native gate + AGPL audit | 1, 2 | BLOCK |
| 17 | **Self-improving feedback** | cave-agent auto-fix PR + stage optimization + DORA → cave-dora | 7 | — |

## 7-Phase Architecture (~47 stage, Runtime-detailed)

Universal backbone'un **45 stage**'i korunur (Phase 1–3 + 7 tam, Phase 4 ve 6
add-on'ları çıkarılır), üzerine **2-3 sovereign add-on** eklenir.

### Phase 1 — Source-time guards (8 stage)

Kaynak henüz build edilmeden, en ucuz kapılar:

| # | Stage | Runtime tooling | Gate |
|---|---|---|---|
| 1 | Linters / formatters | rustfmt + clippy + per-lang linters | BLOCK |
| 2 | Pre-commit hook replay | pre-commit-rs (server-side replay) | BLOCK |
| 3 | **Charter v2 native gate** | cave Charter v2 8-gate self-audit (in-repo) | BLOCK |
| 4 | Semantic diff | cave-agent semantic AST diff (intent-vs-change) | WARN |
| 5 | License gate | REUSE/SPDX lint (AGPL-3.0-or-later enforce) | BLOCK |
| 6 | SBOM diff | CycloneDX delta vs. previous commit | WARN |
| 7 | AI PR review | cave-agent (LLM review, finding triage) | WARN |
| 8 | **SPIFFE runner identity** | cave-identity SPIRE SVID attest (runner zero-trust) | BLOCK |

### Phase 2 — Quality + security (12 stage)

| # | Stage | Runtime tooling | Gate |
|---|---|---|---|
| 9 | Unit test (multi-arch) | x86_64 + ARM64 + **RISC-V** Hetzner builds | BLOCK |
| 10 | Integration test | Argo Workflows ephemeral env | BLOCK |
| 11 | Property-based test | proptest / quickcheck | BLOCK |
| 12 | Mutation test | cargo-mutants threshold | WARN |
| 13 | Fuzz test | cargo-fuzz / libFuzzer corpus | WARN |
| 14 | SAST | Semgrep + clippy security lints | BLOCK |
| 15 | License compliance | REUSE deep scan | BLOCK |
| 16 | Vuln scan | **cave-defender** (Trivy + Grype + Kubescape) | BLOCK |
| 17 | Secret scan | gitleaks + cave-defender secret rules | BLOCK |
| 18 | SBOM generation | CycloneDX full SBOM | — |
| 19 | VEX | VEX statement generation (suppress non-exploitable) | — |
| 20 | Coverage | cargo-llvm-cov threshold gate | BLOCK |

### Phase 3 — Build + artifact (10 stage)

| # | Stage | Runtime tooling | Gate |
|---|---|---|---|
| 21 | Multi-arch build | x86_64 + ARM64 + RISC-V cross-compile | BLOCK |
| 22 | Hermetic reproducible build | network-isolated, pinned toolchain | BLOCK |
| 23 | Container build | rootless + distroless (buildah --no-network) | BLOCK |
| 24 | WASM build | WASI artifact (cave-knative serving target) | BLOCK |
| 25 | OCI manifest | multi-arch manifest list assembly | — |
| 26 | Sign | cosign + **ML-DSA hybrid PQC** (cave-vault key) | BLOCK |
| 27 | SLSA L4 provenance | slsa generator → cave-ledger transparency | BLOCK |
| 28 | in-toto attestation | in-toto link metadata | BLOCK |
| 29 | Reproducible build twice | second build byte-diff verification | BLOCK |
| 30 | **cave-registry push** | Harbor parity (cave-registry OCI, sovereign) | — |

### Phase 4 — Pre-deploy validation (6 universal stage)

> Azure variant'ın 8 stage'inden Azure-managed 2 add-on (Defender posture sync,
> Azure Policy attest) çıkarılır — bunlar Phase 2/1'de cave-defender + cave-policy
> tarafından zaten karşılanır.

| # | Stage | Runtime tooling | Gate |
|---|---|---|---|
| 31 | AI workload eval | cave-agent workload behavior eval | WARN |
| 32 | Performance baseline | criterion benchmark vs. baseline | BLOCK |
| 33 | Memory gate | RSS/heap ceiling enforcement | BLOCK |
| 34 | Startup-time gate | cold-start latency ceiling | BLOCK |
| 35 | Runtime hardening verify | seccomp/AppArmor/rootless verify | BLOCK |
| 36 | CIS / NSA / MITRE | Kubescape benchmark (cave-defender) | BLOCK |

### Phase 5 — Deploy progressive (9 stage)

| # | Stage | Runtime tooling | Gate |
|---|---|---|---|
| 37 | ZAP DAST | OWASP ZAP dynamic scan (ephemeral env) | BLOCK |
| 38 | Integration tests (deployed) | post-deploy smoke + contract | BLOCK |
| 39 | ArgoCD sync | cave-knative via cave-mesh (GitOps) | BLOCK |
| 40 | Argo Rollouts canary | progressive traffic shift + analysis | BLOCK |
| 41 | **Multi-region DR drill** | **Hetzner FRA → HEL failover** verification | BLOCK |
| 42 | Chaos | cave-chaos fault injection (resilience gate) | BLOCK |
| 43 | SLO regression (statistical) | statistical SLO comparison vs. baseline | BLOCK |
| 44 | Blue-green | blue-green cutover + auto-rollback | BLOCK |
| 45 | Promotion gate | aggregate phase-5 verdict | BLOCK |

### Phase 6 — Post-deploy monitor (5 universal stage)

> Azure variant'ın 6 stage'inden Azure-managed 1 add-on (Application Insights
> auto-instrument sync) çıkarılır — cave-trace zaten in-cluster auto-instrument eder.

| # | Stage | Runtime tooling | Gate |
|---|---|---|---|
| 46 | SLI/SLO real-time | cave-metrics live SLI/SLO evaluation | WARN |
| 47 | Cost tracking | cave-metrics cost (Hetzner flat-rate model) | WARN |
| 48 | Metrics push | cave-metrics (Prometheus parity) ingest | — |
| 49 | Logs ingest | cave-logs (Loki parity) ingest | — |
| 50 | Trace | cave-trace (Tempo parity) span ingest | — |

### Phase 7 — Self-improving feedback (4 stage)

| # | Stage | Runtime tooling | Gate |
|---|---|---|---|
| 51 | Flaky test detect | cave-agent flaky-test detection | — |
| 52 | Auto-fix PR | cave-agent auto-fix pull request | — |
| 53 | Pipeline stage optimization | cave-agent stage timing/cost optimization | — |
| 54 | DORA event push | DORA metrics → **cave-dora** | — |

### Stage tally

```
Phase 1:  8  (source-time guards)
Phase 2: 12  (quality + security)
Phase 3: 10  (build + artifact)
Phase 4:  6  (pre-deploy, Azure −2 add-on)
Phase 5:  9  (deploy progressive)
Phase 6:  5  (post-deploy, Azure −1 add-on)
Phase 7:  4  (self-improving feedback)
─────────────────────────────────────────
Universal backbone:           45 stage
Sovereign add-on (folded in): cave-defender (16,36),
                              Hetzner FRA→HEL DR (41),
                              cave-vault PQC sign (26)
─────────────────────────────────────────
Runtime variant total:        ~47-48 stage
```

Azure variant'ın **~55 stage**'i, Cave'in cloud-managed add-on'ları sovereign
in-cluster mekanizmalara konsolide ettiği için **~47-48 stage**'e iner — boyut
*kaybı yok*, sadece Azure'un dağıttığı işi Cave tek surface'te toplar.

## Charter v2 8-gate self-audit linkage

Cave altın kuralı: **strict TDD ADR'nin kendisine de uygulanır.** Bu ADR
[Charter v2](ADR-RUNTIME-STACK-001-cave-runtime-stack-architecture.md) 8-gate
compliance'a bağlanır — Phase 1, Stage 3 ("Charter v2 native gate") pipeline'ın
*kendi* self-audit'idir ve aşağıdaki 8 kapıyı her PR'da yeniden çalıştırır:

| Gate | Charter v2 kontrolü | Bu ADR için anlamı |
|---|---|---|
| 1 | No fabrication | Stage tally gerçek (45 universal + add-on), şişirilmemiş |
| 2 | Version pins honest | Tooling pin'leri (Argo, Trivy, Grype, Kubescape) doğrulanır |
| 3 | No stub macros | Hiçbir stage `todo!()`/placeholder ile "mapped" sayılmaz |
| 4 | Honest fill ratio | 17/17 dim *mekanizma ile* karşılanır, sayı şişirme yok |
| 5 | Audit date fresh | `last_audit == TODAY` (2026-06-07) lockstep |
| 6 | Upstream traceable | Her cave-native primitive upstream parity'ye map'li |
| 7 | Self-scanner clean | Gate'in kendisi false-positive üretmez |
| 8 | License consistent | AGPL-3.0-or-later her artifact'ta enforce |

Pipeline kodlandığında bu kapılar `crates/<crate>/src/parity_self_audit.rs`
desenini izleyen bir CI self-audit testine bağlanır (bkz. README "Related").

## Consequences

### Olumlu

- **Air-gap-capable by construction** — pipeline hiçbir public-cloud
  control-plane'e bağlı değil; Cave kümesi izole bir DC'de tam çalışır.
- **Tek sovereignty düzlemi** — kimlik (cave-identity), policy (cave-policy),
  secret (cave-vault), gözlemlenebilirlik (cave-metrics/logs/trace) hepsi
  in-cluster.
- **PQC-ready bugünden** — ML-DSA hybrid imza + ML-KEM seal Phase 3'te aktif.
- **17-dim garantisi korunur** — future-proofing boyutu Azure variant ile eşit.
- **EU-sovereign residency** — Hetzner FRA + HEL, multi-region DR drill
  first-class deploy stage.

### Olumsuz / maliyet

- **Operasyon yükü** — Azure-managed servislerin yerini Cave'in *kendi*
  çalıştırdığı bileşenler alır; cave-defender/cave-vault/Argo bakımı bizde.
- **Marketplace yok** — GitHub Actions marketplace ekosistemini kaybederiz;
  reusable Argo WorkflowTemplate'leri kendimiz yazarız.
- **RISC-V build maliyeti** — multi-arch matrise RISC-V eklemek build süresini
  uzatır (cross-compile + native Hetzner runner).

### Riskler & azaltım

- **Argo Workflows ölçek** → cave-knative serving HPA + ephemeral runner pool.
- **DR drill flakiness** → Stage 41 statistical retry + cave-chaos baseline.
- **Self-host scanner drift** → cave-defender'ın Trivy/Grype/Kubescape DB'leri
  cave-runtime-tracker ile günlük güncellenir ([[cave-runtime-tracker-bootstrap-2026-06-07]]).

## Related ADRs

- [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) — Sovereign bare-metal (Hetzner FRA/HEL)
- [ADR-RUNTIME-STACK-001](ADR-RUNTIME-STACK-001-cave-runtime-stack-architecture.md) — Cave Runtime stack (Charter v2)
- [ADR-RUNTIME-CERT-LIFECYCLE-001](ADR-RUNTIME-CERT-LIFECYCLE-001-sovereign-cert-hierarchy-pqc-acme.md) — PQC cert hierarchy
- [ADR-154](ADR-154_ArgoCD_GitOps_Adoption.md) — cave-deploy / ArgoCD GitOps
- [ADR-157](ADR-157_Sigstore_Cosign_Adoption.md) — cosign adoption
- [ADR-076](ADR-076_cave_ctl_CLI_MCP_Server_Architecture.md) — cavectl CLI
- **Platform ADR-010** — Multi-Dimensional Future-Proof CI Pipeline (Azure reference variant)

---

*Bu ADR Platform ADR-010'un **Runtime sovereign variant**'ıdır. Azure-specific
primitive'ler cave-native karşılıklarıyla değiştirilmiş, 17-dim future-proofing
ve 7-faz mimari korunmuştur. Cave Runtime AGPL-3.0-or-later.*
