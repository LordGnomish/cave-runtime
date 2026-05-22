# ADR-102: Mandatory Data Classification Labels

**Status:** Accepted

**Scope:** Universal

**Category:** Security

**Related ADRs:** 103, 111, 113

## Context

Data governance requires knowing what kind of data each resource holds. Without classification, security controls cannot be applied appropriately.

## Candidates

| Classification | LLM Routing | Log Handling | Backup | Egress | Residency |
|---|---|---|---|---|---|
| public | Any provider | Standard | Encrypted | Default | Tenant choice |
| internal | Any provider | PII-filtered | Encrypted | Default | EU preferred |
| confidential | AzOAI/Ollama | Redacted (no PII) | Encrypted+verified | Restricted | EU required |
| restricted | Ollama only | Minimal metadata only | Encrypted+dual-verified | Strict allowlist | EU pinned |

## Decision

Every data resource carries mandatory classification: `public | internal | confidential | restricted`. OPA rejects resources without classification label at admission. Classification drives LLM routing (ADR-111), backup encryption level, log redaction depth, egress posture (ADR-110), data residency (ADR-113), prompt/output retention policy.

## Rejected

- **Optional classification:** Creates gaps. Unclassified data defaults to lowest protection. Compliance failure for GDPR Art.25.
- **Binary (public/private):** Too coarse. Confidential and restricted have very different handling requirements (LLM routing, residency, log redaction depth).
- **Application-level classification:** Not enforceable at platform level. Developers can forget or misclassify. Platform OPA enforcement ensures no unclassified data exists.

## Consequences

**Positive:**
- Every data resource has explicit classification from creation.
- Security controls applied automatically based on classification.
- Classification-based routing prevents restricted data from reaching external LLMs.
- Compliance evidence produced automatically (classification → controls → attestation).

**Negative:**
- Classification selection adds friction to developer self-service (mitigated: Golden Path templates pre-select default).
- Misclassification risk (classifying restricted as internal). Partially mitigated by AI Compliance Officer anomaly detection.
- Classification change on existing resource triggers re-evaluation of all derived controls (backup, routing, residency).

## Compliance Mapping

GDPR Art.25 (data protection by design). GDPR Art.5(1)(f) (integrity and confidentiality). ISO A.5.12 (classification of information). ISO A.5.13 (labelling of information). SOC2 CC6.1 (classification-based access controls).
