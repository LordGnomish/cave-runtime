# ADR-128: APOL Attestation Redaction Policy

**Status:** Accepted

**Scope:** Azure, Runtime, Universal

**Category:** AI Governance

**Related ADRs:** 125

## Context

APOL reasoning traces contain operational details (metric values, hostnames, IPs, token names). Writing raw details to Sovereign Ledger creates security and privacy risks.

## Candidates

| Content | Ledger (redacted) | Forensic WORM (full) |
|---|---|---|
| Metrics | Deviation ratios only (e.g., "3.2x P95") | Raw values |
| Playbooks | Reference ID only | Full playbook YAML |
| Actions | Summary (e.g., "restarted pod X") | Full kubectl command |
| Infrastructure | No hostnames/IPs/secrets/PII | Full details |
| Linked by | Incident ID | Incident ID |

## Decision

Traces in Sovereign Ledger redacted before write: metric IDs + deviation ratios (not raw values), playbook references (not inline payloads), action summaries (not hostnames/IPs/secrets/PII). Statistical evidence (deviation ratios) permitted — sufficient for post-mortem without exposing operational details. Full forensic context in WORM bucket (§38), linked by incident ID. Redaction enforced by cave-ctl MCP layer.

## Rejected

- **Raw CoT dump to Ledger:** Leaks operational details (IPs, hostnames, secret names, internal URLs). Ledger is immutable — leaked data cannot be removed. Security risk.
- **No redaction (full data in Ledger):** Ledger storage explosion. Attestation size grows 10-50x with raw data.
- **No reasoning trace:** Opaque AI. Can't audit decisions. Compliance failure.

## Consequences

**Positive:**
- Ledger contains structured, queryable, redacted traces — sufficient for audit and post-mortem.
- No operational secrets in immutable WORM storage.
- Full forensic detail available via WORM link when needed.
- Compact attestations keep Ledger storage manageable.

**Negative:**
- Redaction logic must be maintained as new metric types and action types are added.
- Redaction could accidentally remove important context — deviation ratios may not always capture the full picture.
- Two-tier storage (Ledger + WORM) adds complexity to forensic investigation workflows.

## Compliance Mapping

SOC2 CC7.2 (monitoring evidence — appropriate detail level). GDPR Art.25 (data minimisation in audit logs). ISO A.8.15 (logging — proportionate detail). NIS2 Art.21 (audit trail security).
