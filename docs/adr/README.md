<!--
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Cave Runtime — Architecture Decision Records

This directory contains the architecture decisions that are **normative
for the Cave Runtime OSS release**. Each ADR describes a design choice
made for the Rust workspace itself (cave-runtime crates, cave-portal,
`cavectl`, the resident agent, governance contracts).

Decisions about **how to deploy** Cave Runtime onto a particular cloud,
which managed services to substitute, which CI/CD tooling to use, and
which third-party SaaS to integrate — those are *deployment-profile*
decisions and live in the upstream Platform repository, not here. The
inheritance contract between the two is described in ADR-005.

## Index

| # | Title | Status |
|---|-------|--------|
| [ADR-001](ADR-001-sovereign-bare-metal-hosting.md) | Sovereign bare-metal hosting reference profile | Accepted |
| [ADR-002](ADR-002-cave-runtime-stack-architecture.md) | Cave Runtime stack architecture (Layers 1–4) | Accepted |
| [ADR-003](ADR-003-multi-tenant-by-construction.md) | Cave Runtime is multi-tenant by construction | Accepted |
| [ADR-004](ADR-004-zero-trust-runtime-principle.md) | Zero-trust as a Runtime architectural principle | Accepted |
| [ADR-005](ADR-005-platform-runtime-upstream-mirror.md) | Platform ↔ Runtime upstream mirror contract | Accepted |
| [ADR-006](ADR-006-persistence-consolidation.md) | Data persistence — multi-upstream consolidation | Accepted |
| [ADR-007](ADR-007-streaming-consolidation.md) | Streaming — Kafka + Pulsar consolidation into `cave-streams` | Accepted |
| [ADR-008](ADR-008-api-gateway-consolidation.md) | API gateway — Kong + Gravitee consolidation into `cave-gateway` | Accepted |
| [ADR-009](ADR-009-cavectl-cli-consolidation.md) | Single `cavectl` binary, native + compatibility surfaces | Accepted |
| [ADR-010](ADR-010-sovereign-cert-lifecycle.md) | Sovereign certificate hierarchy + PQC-ready + multi-DNS ACMEv2 | Accepted |
| [ADR-011](ADR-011-upstream-watch-daemon.md) | Upstream watch daemon (`cave-upstream-watchd`) | Accepted |
| [ADR-012](ADR-012-cavectl-mcp-server-architecture.md) | `cavectl` CLI + MCP server architecture | Accepted |
| [ADR-013](ADR-013-cave-webapplication-composition.md) | Cave WebApplication composition pattern | Accepted |
| [ADR-014](ADR-014-data-persistence-crate-naming.md) | Data persistence crate naming + lakehouse consolidation | Accepted |
| [ADR-015](ADR-015-kubevirt-sovereign-vm-workloads.md) | KubeVirt for sovereign VM workloads | Accepted |
| [ADR-016](ADR-016-portal-authentication.md) | Portal authentication gate — `cave-auth` OIDC first | Accepted |
| [ADR-017](ADR-017-portal-personas.md) | Portal personas — admin vs tenant | Accepted |
| [ADR-018](ADR-018-portal-desktop-shell.md) | GPUI native desktop shell alongside web portal | Accepted |
| [ADR-019](ADR-019-cave-communication-hub.md) | Cave Communication Hub — multi-surface team + LLM chat | Accepted |
| [ADR-020](ADR-020-crm-upstream-twenty.md) | CRM upstream selection — Twenty | Accepted |
| [ADR-021](ADR-021-passwordless-authentication.md) | Passwordless authentication strategy | Accepted |
| [ADR-022](ADR-022-self-improving-cave-agent.md) | `cave-agent` — runtime-resident self-improvement | Accepted |
| [ADR-023](ADR-023-resurrection-protocol.md) | Resurrection protocol — restore-from-cold | Accepted |
| [ADR-024](ADR-024-constitutional-tiering.md) | Constitutional tiering of governed configuration | Accepted |
| [ADR-025](ADR-025-evidence-tiering.md) | Evidence tiering for Runtime changes | Accepted |
| [ADR-026](ADR-026-waiver-framework.md) | Waiver framework for temporary policy exceptions | Accepted |
| [ADR-027](ADR-027-contribution-attribution.md) | Contribution attribution — commit author + trailers | Accepted |
| [ADR-028](ADR-028-oss-launch-history-strategy.md) | OSS launch history strategy | Accepted |

**Total: 28 ADRs.**

## Reading order

* New to the project? Read **001 → 005** in order. They cover the
  hosting model, the stack architecture, the multi-tenancy invariant,
  the zero-trust posture, and how Cave Runtime relates to its upstreams.
* Working inside a Runtime crate? Jump to the relevant decision
  (006–020).
* Governing a cluster (compliance, change management)? Read
  **023 → 026**.
* Contributing? Read **027** for trailer conventions; **028** for the
  history-preservation policy used for the 21 May 2026 launch.

## What you will *not* find here

Decisions of the form "we picked Cilium over Calico", "we run Trivy in
CI", "we use Cloudflare for DNS" — those are Platform repository
decisions. Cave Runtime ships **reimplementations** of the relevant
upstream behaviour (cave-net, cave-mesh, cave-vulns, cave-llm-gateway,
etc.); which products an operator chooses to deploy alongside is their
profile decision, not a Runtime ADR. See ADR-005 for the inheritance
contract.

## Historical Platform references

Some surviving ADRs still mention deleted Platform-side ADR numbers
(e.g., `ADR-079` for Secret-Zero Bootstrap, `ADR-129` for Tenant
Identity Federation, `ADR-130` for PAM, etc.). Those numeric tags refer
to the **historical Platform repository numbering** and are kept
verbatim for archival traceability. The corresponding Platform ADRs are
not in this OSS repository.

## Archive

`internal/` holds the original Platform deployment-profile ADRs from
pre-OSS Cave Runtime (Hetzner / Azure / managed-service variants). They
are **not normative** for the OSS release and exist for historical
reference only. See `internal/README.md`.

## Process

1. **Proposal.** A new ADR starts as `Proposed`, with context,
   alternatives, decision, and consequences.
2. **Review.** Maintainers review against the principles in ADR-002,
   ADR-003, ADR-004 (no implicit trust, no backcompat shims).
3. **Acceptance.** `Accepted` means the decision is in force —
   implementation may already exist or be in progress.
4. **Deprecation.** Deprecated ADRs are retained (history preserves
   reasoning); they are not removed.

---

Generated 2026-05-18 for the 21 May 2026 OSS launch.
