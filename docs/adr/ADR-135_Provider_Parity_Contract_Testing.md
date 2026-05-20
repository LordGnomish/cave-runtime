# ADR-135: Provider Parity Contract Testing

**Status:** Accepted

**Scope:** Hyperscaler, Sovereign, Runtime, Universal

**Category:** Platform Governance

**Related ADRs:** 066 (Provider Choice), 067 (Crossplane v2), 071 (kuttl Testing), 113 (Data Residency)

## Context

CAVE's core promise is "same developer experience regardless of infrastructure target." Every Crossplane XR abstracts provider differences — a `Database` XR provisions CloudNativePG on sovereign cloud and hyperscaler PG Flexible on Azure. But without automated verification, this parity is an assertion, not a guarantee.

Scenarios where parity can silently break:
- Azure PG adds a feature that changes default backup behavior — Hetzner CNPG doesn't match
- MinIO erasure coding semantics differ from ADLS Gen2 versioning
- Valkey sentinel failover behaves differently from Azure Redis Enterprise active geo-replication
- Qdrant vector similarity scores differ from Azure AI Search vector implementation

---

## Candidates

| Approach | kuttl parity tests, 6 dimensions (chosen) | Trust Crossplane abstraction | Full parity guaranteed | Provider-specific testing only |
|---|---|---|---|---|
| Parity validation | ✅ Automated, measurable | ❌ Assumption-only | ❌ Impossible (different services) | ❌ Not cross-provider |
| Exception documentation | ✅ parity-exceptions.yaml | ❌ | ⚠️ Blocked by reality | ⚠️ Per-provider |
| Test cadence | ✅ Monthly/quarterly by risk | ❌ None | N/A | ⚠️ |
| Exit strategy proof | ✅ Directly validates portability | ❌ Unverified | N/A | ❌ Unverified |
| Evidence for audit | ✅ Parity Verified attestation | ❌ | N/A | ❌ |

## Decision

Every Crossplane XR must pass **parity contract tests** that verify behavioral equivalence across sovereign cloud and hyperscaler compositions.

### Parity Dimensions

| Dimension | What is Tested | How |
|---|---|---|
| **API shape** | Same XR spec produces equivalent resource on both providers | kuttl test: apply identical XR spec, verify status fields match contract |
| **Policy enforcement** | Same OPA policies apply regardless of provider | Conftest: identical policy evaluation on both compositions |
| **Backup semantics** | Same backup frequency, retention, restore procedure | Automated backup + restore test (monthly in staging) |
| **Observability labels** | Same Prometheus metrics, same Grafana dashboard compatibility | Metric name + label parity assertion |
| **Failure classification** | Same failure modes produce same alert categories | Chaos test: inject equivalent failures on both providers |
| **Restore contract** | Same RPO/RTO targets achievable on both providers | Recovery drill: measure actual RPO/RTO on both |

### Test Execution

- kuttl parity tests run in CI for every XRD/Composition change
- Full parity drill (backup+restore+chaos) runs monthly in staging on both profiles
- Parity test results recorded as Sovereign Ledger `Parity Verified` attestation
- `cave-ctl portability drill --tenant <n>` includes parity validation

### Parity Exceptions

Some capabilities are inherently provider-specific (e.g., Azure PG HA uses zone-redundant architecture that CNPG cannot replicate identically on single-DC Hetzner). Exceptions require:
1. ADR documenting the gap
2. Tenant-facing documentation explaining behavioral difference
3. Degradation classification (cosmetic / functional / SLA-affecting)
4. Compensating control if SLA-affecting

---

## Rejected

- **No parity testing (trust Crossplane abstractions):** Crossplane Compositions may have bugs or provider-specific behavioral differences. Without testing, parity is an assertion, not a fact.
- **Full parity (identical behavior guaranteed):** Impossible between self-hosted (sovereign) and managed (Azure) services. Some differences are inherent (backup semantics, failover behavior). Parity exceptions must be documented.
- **Provider-specific testing only:** Tests that only validate one provider don't prove portability. Cross-provider parity tests are specifically designed to validate that the same XR produces equivalent behavior on both providers.

## Consequences

## ### Positive
- "Developer experience identical" becomes a tested guarantee, not just a principle
- Provider migration risk quantified via regular parity measurement
- Exit strategy (ADR-066) strengthened with empirical portability evidence

### Negative
- Monthly parity drills consume staging resources (2-4 hours per drill)
- Some provider differences are fundamental and cannot be eliminated (acknowledged via exceptions)
- Parity test maintenance overhead as XRDs evolve

## Compliance Mapping

SOC2 CC9.1 (risk mitigation — provider exit strategy validated by parity tests). ISO A.5.23 (cloud service agreements — provider portability). ISO A.5.30 (ICT readiness — exit strategy). NIS2 Art.21 (supply chain — provider independence). GDPR Art.44-49 (data transfers — parity validates residency across providers).
