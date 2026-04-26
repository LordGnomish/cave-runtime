# ADR-092: AI Privilege Guardrails (MCP Allowlist/Denylist)

**Status:** Accepted

**Scope:** Runtime, Universal

**Category:** AI Governance

**Related ADRs:** 112, 136

## Context

AI agents (APOL roles, Backstage AI, LibreChat) interact with the platform via cave-ctl MCP. Without guardrails, AI can perform destructive operations.

## Candidates

| Control | Mechanism | Scope |
|---|---|---|
| Allowlist | MCP method whitelist per AI role | AI SRE: pod restart, HPA scale, namespace quarantine |
| Denylist | MCP method blacklist (all roles) | namespace delete, terraform apply, raw secret access, policy mutation |
| Privilege ceiling | User RBAC maximum | AI privilege ≤ initiating user's RBAC |
| Rate limiting | Per-role request cap | Prevents runaway automation loops |
| Environment scoping | Prod stricter than dev | Prod AI SRE max 2x scale without human approval |

## Decision

AI assistants via MCP restricted by: (1) Allowlist per role — specific operations each role can invoke. (2) Denylist — operations no role can ever invoke. (3) Privilege ceiling — AI never exceeds initiating user's RBAC. (4) Rate limiting per role. (5) Environment scoping (prod stricter). Every MCP call logged to Sovereign Ledger with caller identity, action, parameters, result.

## Rejected

- **No guardrails:** Uncontrolled AI. APOL could delete namespaces, modify security policies, access raw secrets.
- **Per-request human approval:** Defeats automation purpose. Operator fatigue leads to rubber-stamp approvals.
- **Full autonomy with post-hoc audit:** Damage already done before audit detects it. Prevention > detection.

## Consequences

**Positive:**
- AI agents cannot perform destructive operations outside bounded authority.
- Privilege ceiling prevents privilege escalation via AI.
- Immutable audit trail for every AI action.
- Rate limiting prevents automation loops.

**Negative:**
- Allowlist maintenance as new MCP methods are added.
- Denylist must be comprehensive — missing a destructive operation creates gap.
- Legitimate operations may be blocked if allowlist is too narrow.

## Compliance Mapping

SOC2 CC6.1 (AI access controls). ISO A.5.23 (information security for cloud services — AI as cloud service). NIS2 Art.21 (security of automated systems).
