# ADR-057: Application Security Testing Strategy — Defense-in-Depth

**Status:** Accepted

**Scope:** Azure, Universal

**Category:** Security

**Related ADRs:** 010, 017, 018, 019, 023, 035, 058

## Context

CAVE's security testing must span the entire software lifecycle — from pre-commit through runtime. No single tool catches all vulnerability types. A layered strategy ensures coverage across secrets, code quality, dependencies, containers, infrastructure, compliance, and runtime behavior.

## Candidates

| Approach | Multi-layer (chosen) | Single SAST tool | SaaS scanner | Manual review only |
|---|---|---|---|---|
| Coverage | ✅ 7 layers | ⚠️ Code only | ⚠️ Vendor-dependent | ❌ Human bandwidth limited |
| Automation | ✅ CI-integrated | ✅ | ✅ | ❌ |
| False negative rate | ✅ Low (complementary tools) | ❌ High (single perspective) | ⚠️ | ❌ High |

## Decision

## Decision (revised — split)

CAVE'in software security testing **iki ayrı pipeline** ile koşar:

### Platform Security Pipeline
Cave team'in kendi kodu (Cave Runtime, Cave Platform repo'ları, ADR'lar, tooling). Trusted internal contributors. Strict policy:
- 7-layer aynı: gitleaks → SonarQube+Semgrep → CycloneDX+DTrack → Trivy → Conftest+Checkov → Kubescape → ZAP
- **Tüm BLOCK threshold'ları HIGH+** (Critical+High mandatory fix; Medium 30d SLA)
- Sovereign supply chain: cosign signing zorunlu, Trufflehog CI-time + gitleaks pre-commit
- Cave-specific tools: cave-self-improver dependency drift, internal package mirror

### Tenant Security Pipeline
Tenant'ın Cave'e push ettiği kendi uygulaması (ADR-031 WebApplication composition). Less-trusted contributor base. **Tenant-customizable policy with platform floor:**
- Aynı 7-layer toolchain (toolchain consistency, finding fan-in DefectDojo'ya)
- **Platform floor:** Critical secrets/SAST/SCA → tenant override edemez (zorunlu BLOCK)
- **Tenant-tunable:** High/Medium thresholds tenant'ın phase + classification level'ına göre (ADR-102) tenant tarafından konfigüre edilebilir
- Tenant **kendi scanner extension**'larını ekleyebilir (örn. tenant'ın license'lı Snyk veya kendi SonarQube projesi) — additive olarak çalışır, replace etmez
- **Waiver flow:** ADR-140 (Waiver Framework) tenant exception'ları için resmi süreç; security-team approval + expiration
- Tenant DefectDojo product-per-tenant scope, finding access RBAC tenant'a sınırlı

Both pipelines feed unified DefectDojo (ADR-035) but with strict tenant scoping. SLA tracking same toolchain, different per-tier targets.

### Tool layer matrix (ortak iki pipeline'da)

| Layer | Tool | CI Stage | Platform Gate | Tenant Gate (default) | Finding Destination |
|---|---|---|---|---|---|
| Secrets | gitleaks (ADR-017) | Pre-commit + Stage 2 | BLOCK any | BLOCK any (floor) | DefectDojo |
| SAST | SonarQube + Semgrep (ADR-019) | Stages 3-4 | BLOCK High+ | BLOCK Critical (floor) + tenant-tunable High | DefectDojo |
| SCA/SBOM | CycloneDX + DTrack | Stages 9-10 | BLOCK High+ | BLOCK Critical (floor) | DTrack + DefectDojo |
| Container | Trivy (ADR-018) | Stage 16 | BLOCK High+ | BLOCK Critical (floor) | DefectDojo |
| IaC | Conftest + Checkov | Stages 17-18 | BLOCK policy violations | Tenant-tunable | DefectDojo |
| Compliance | Kubescape (ADR-058) | Stage 19 | BLOCK Phase 3+ | WARN → BLOCK Phase 3+ | DefectDojo |
| DAST | OWASP ZAP (ADR-023) | Stages 23-24 | BLOCK High+ | Tenant-tunable | DefectDojo |

## Rejected

- **Single SAST tool only:** Catches code-level issues but misses dependencies, containers, infrastructure, runtime. Single tool = single perspective = blind spots.
- **SaaS security scanner (Snyk, Checkmarx):** Proprietary, per-scan pricing, code sent to external service. Contradicts sovereign profile.
- **Manual security review only:** Doesn't scale for 27-stage pipeline across multiple tenants. Human review complements but cannot replace automated scanning.

## Consequences

**Positive:**
- Seven layers of automated security testing — comprehensive coverage.
- CI-integrated — no manual steps between development and deployment.
- All findings in one dashboard (DefectDojo) — unified triage and SLA tracking.
- Complementary tools reduce false negatives (what SonarQube misses, Semgrep catches).

**Negative:**
- Seven tools to maintain (version updates, rule tuning, false positive management).
- CI pipeline duration increases (~5-15 min from security stages).
- Finding volume can be high — triage discipline required.
- DefectDojo deduplication is good but not perfect — some cross-tool duplicates need manual triage.

## Notes

**Universal scope.** Bu ADR meta-orchestrator — runtime mirror her alt ADR'de (cave-secrets ADR-017, cave-sast ADR-019, cave-sbom, cave-container-scan ADR-018, cave-iac-scan, cave-compliance-scan ADR-058, cave-dast ADR-023, cave-defectdojo ADR-035) ayrı ayrı zaten REQUIRED. Bu ADR-057 onları **runtime-side'da cave-security-orchestrator** crate (Mirror-001 blanket; meta-coordinator) altında tek workflow engine'e bağlar.

**Dual sub-orchestrator:** cave-security-orchestrator iki mod ile çalışır — **platform mode** (strict policy bundle, floor + ceiling aynı, no waiver) + **tenant mode** (platform floor + tenant-tunable ceiling, waiver flow ADR-140). Aynı core orchestrator binary, farklı policy bundle'ı yüklenir; pipeline scope'u (Cave team kodu vs tenant uygulaması) commit-time identity'den (ADR-RUNTIME-CERT-LIFECYCLE-001 PQC signer) belirlenir.

Sovereign deployment'da DefectDojo helm bağımlılığı yok, finding aggregation runtime native (per-tenant product scope tenant mode'da, platform mode'da single-bucket), SLA enforcement Reflex Engine ile zincir, Sovereign Ledger (ADR-093) WORM signed proof. Kubescape Phase 3+ BLOCK transition cave-self-improver'ın reasoning-loop'unda phase-gate olarak yer alır. Tenant scanner extension'ları (Snyk, kendi SonarQube vb.) plugin slot'una bağlanır — additive, platform floor'u replace edemez.

## Compliance Mapping

SOC2 CC7.1 (multi-layer vulnerability management). ISO A.8.25-28 (complete secure development lifecycle). OWASP Top 10 (covered by SAST + DAST). NIS2 Art.21 (comprehensive vulnerability management). SLSA Level 3 (supply chain scanning).
