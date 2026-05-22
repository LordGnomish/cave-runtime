# ADR-104: Identity Lifecycle Governance

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** Identity

**Related ADRs:** 006, 007, 064, 129, 130

## Context

Identity sprawl (dormant accounts, stale roles, orphaned identities) is a persistent security risk. Manual lifecycle management doesn't scale.

## Candidates

| Control | Mechanism | Cycle |
|---|---|---|
| Dormant detection | `cave-ctl identity dormant --since 90d` (daily CronJob) | 90d no-login → auto-disable |
| RBAC recertification | Quarterly review (Tenant Admin confirms all roles) | Unconfirmed roles auto-revoked after 14d |
| JIT admin access | `cave-ctl pam request create` (dual-approval, n8n) | 4h TTL, PAM-recorded |
| Break-glass | `cave-ctl pam request create --break-glass` (dual-approval) | Duration-bounded, Ledger-attested |
| Identity drift | `cave-ctl identity drift --profile <p>` (daily scan) | Orphaned identities flagged, P3 alert |

## Decision

Automated identity lifecycle: dormant detection (90d → disable), quarterly RBAC recertification, JIT admin (4h TTL, dual-approval), break-glass (dual-approval, PAM-recorded, Ledger-attested), RBAC drift detection. Canonical identity: `cave_uid` (stable UUID across IdP migrations). Token contract: sub (IdP-specific), cave_uid (stable), tenant_id, env.

## Rejected

- **No dormant detection:** Stale admin accounts accumulate. Attacker can compromise unused accounts undetected.
- **Annual recertification:** SOC2 CC6.2 recommends more frequent review. Annual cycles miss role changes.
- **Permanent admin access:** Standing privilege. Attacker who compromises admin has indefinite access. JIT limits exposure window to 4 hours.
- **sub-based identity:** IdP-specific `sub` claim changes when IdP migrates. cave_uid provides stable identity.

## Consequences

**Positive:**
- No stale accounts. Dormant detection is automated and continuous.
- Least-privilege enforced via JIT (4h TTL) — no permanent admin.
- Quarterly recertification catches role drift before audit.
- cave_uid survives IdP migration — identity is portable.

**Negative:**
- Quarterly recertification is operational overhead for Tenant Admins.
- JIT approval adds latency for urgent admin access (mitigated: break-glass for emergencies).
- cave_uid mapping must be maintained and backed up.
- Dormant auto-disable can lock out legitimate users returning from extended leave (mitigated: re-enablement via JIT process).

## Compliance Mapping

SOC2 CC6.2-6.3 (user lifecycle). ISO A.5.16-18 (identity management, authentication, access rights). NIS2 Art.21 (access control policies).
