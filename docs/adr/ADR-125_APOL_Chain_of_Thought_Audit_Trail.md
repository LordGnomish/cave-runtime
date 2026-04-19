# ADR-125: APOL Chain-of-Thought Audit Trail

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** AI Governance

**Related ADRs:** 112, 128

## Context

AI decisions must be explainable and reproducible. Every APOL action must have a complete reasoning chain that a human can review post-hoc.

## Candidates

| Approach | Structured CoT → Sovereign Ledger | Log-only | No audit | Post-hoc explanation |
|---|---|---|---|---|
| Written before execution | ✅ | ❌ Log written during/after | N/A | ❌ After the fact |
| Structured format | ✅ trigger→metrics→playbook→action→outcome→confidence | ❌ Free-text | N/A | ⚠️ |
| Replayable | ✅ cave-ctl ai audit --since <t> | ❌ | N/A | ❌ |
| Tamper-proof | ✅ Sovereign Ledger (WORM + Sigstore) | ❌ Mutable logs | N/A | ❌ |

## Decision

Every AI decision produces structured reasoning trace written to Sovereign Ledger **before execution**. Trace: trigger condition → evaluated metrics → matched playbook → selected action → expected outcome → confidence score. Redacted per ADR-128 (no raw values, no PII). `cave-ctl ai audit --since <t>` replays any decision chain. Traces link to forensic data in WORM by incident ID.

## Rejected

- **Log-only audit:** Mutable logs can be altered. Written during/after execution — cannot prevent bad decisions. Not structured — hard to query.
- **No audit:** Opaque AI. Impossible to explain decisions to auditors or post-mortem reviewers.
- **Post-hoc explanation (ask AI to explain after):** AI can rationalize any past action. Not reliable. Pre-execution trace captures actual reasoning.

## Consequences

**Positive:**
- Complete audit trail for every AI decision. Auditors can replay any action chain.
- Written BEFORE execution — trace exists even if action fails mid-execution.
- Structured format enables automated analysis (e.g., "show all Class C actions with confidence < 0.8").
- WORM + Sigstore ensures traces cannot be tampered with.

**Negative:**
- Trace generation adds ~1-2 seconds per AI decision (LLM generates structured trace).
- Sovereign Ledger write per AI action increases Ledger volume (mitigated: redaction keeps traces compact).
- Traces must be redacted (ADR-128) — full context only in forensic WORM bucket.

## Compliance Mapping

SOC2 CC7.2 (monitoring evidence). ISO A.5.26 (incident response — explainable automation). NIS2 Art.21 (audit trail for automated security measures). GDPR Art.22 (right to explanation for automated decisions).
