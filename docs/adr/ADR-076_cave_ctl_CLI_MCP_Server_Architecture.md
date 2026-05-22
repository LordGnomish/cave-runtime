# ADR-076: cave-ctl CLI & MCP Server Architecture

**Status:** Accepted

**Scope:** Sovereign, Runtime

**Category:** Platform

**Related ADRs:** 092, 112

## Context

CAVE needs a unified CLI for platform operations, automation integration (MCP for AI agents), and developer self-service. Must wrap underlying tools (kubectl, talosctl, ArgoCD, Crossplane, etc.) with platform-aware semantics.

## Candidates

| Approach | Unified cave-ctl CLI + MCP (chosen) | Multiple specialized CLIs | Backstage-only (no CLI) | kubectl + shell scripts |
|---|---|---|---|---|
| Single UX | ✅ One tool, all operations | ❌ Context switching | ⚠️ Web-only, no automation | ❌ Scripts sprawl |
| MCP/AI integration | ✅ Native MCP server | ❌ Per-tool custom integration | ❌ | ❌ |
| Ledger attestation | ✅ Built-in per command | ❌ Per-tool implementation | ⚠️ Backstage events only | ❌ |
| RBAC scoping | ✅ Profile-aware | ⚠️ Per-tool config | ✅ Backstage-enforced | ❌ None |
| Break-glass capability | ✅ Emergency CLI works offline | ⚠️ | ❌ Depends on portal | ⚠️ Limited |

## Decision

**cave-ctl** as a Go-based CLI + MCP Server. CLI wraps underlying tools with platform context (profile-awareness, RBAC enforcement, Ledger attestation). MCP Server enables AI agents (APOL) to invoke operations programmatically.

**Design:** cave-ctl is an ergonomic wrapper, not the enforcement mechanism. Governance is enforced by underlying toolchain (OPA admission, ArgoCD reconciliation, GitHub Actions, Crossplane). cave-ctl provides: profile-aware context switching, RBAC-scoped operations, Sovereign Ledger attestation for every operation, human-readable error messages referencing ADRs/runbook sections.

**Command groups:** platform (upgrade, promote, doctor), tenant (create, delete, budget, migrate), identity (dormant, drift, jit), pam (request, connect), apol (freeze, unfreeze, fallback, override, audit), compliance (export, verify), local (up, policy-gap), gitops (force-sync), network (test), observability (fallback).

## Rejected

- **kubectl wrappers (shell scripts):** No type safety, no MCP integration, no Ledger attestation, no RBAC awareness.
- **Backstage-only (no CLI):** CLI needed for automation, break-glass, and power-user operations.
- **Multiple specialized CLIs:** Fragmented UX. Single CLI with subcommands is more discoverable.

## Consequences

**Positive:**
- Single CLI for all platform operations — no switching between kubectl, talosctl, argocd, crossplane CLI.
- MCP Server enables AI agent integration (APOL) with same RBAC enforcement.
- Profile-aware context switching — developer doesn't need to manage kubeconfig manually.
- Sovereign Ledger attestation for every operation — complete audit trail.
- Human-readable errors reference specific ADRs and runbook sections.

**Negative:**
- Go CLI development and maintenance effort.
- cave-ctl is an ergonomic wrapper, not the enforcement mechanism — governance lives in underlying tools. cave-ctl failure doesn't break governance.
- MCP Server adds API surface to secure (allowlist/denylist per ADR-092).
- CLI version must be compatible with cluster version — versioning discipline required.

## Compliance Mapping

SOC2 CC6.1 (unified access surface with RBAC). SOC2 CC7.2 (operational audit trail — every cave-ctl command logged). ISO A.8.9 (configuration management via CLI).
