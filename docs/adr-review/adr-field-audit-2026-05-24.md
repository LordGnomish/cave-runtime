# ADR field audit — 2026-05-24

**Recorder:** doc-sync ray (Opus) | **Scope:** `docs/adr/*.md` on `main`

Audit covers the canonical fields per ADR README (`docs/adr/README.md`):
**Title** (`# ...`), **Status**, **Date**, **Context**, **Decision**,
**Consequences**, **Alternatives Considered**.

This file is **report-only** — no ADR body was edited. Numerical gaps in
the sequence are documented under [ADR numbering policy](#adr-numbering-policy)
and remain stable by design (per `docs/adr/README.md`).

---

## Numbering inventory

Files present on `main`:

| ID | File | Notes |
|----|------|-------|
| ADR-001 | `ADR-001-sovereign-bare-metal-hosting.md` | Foundational |
| ADR-031 | `ADR-031-cave-webapplication-composition-pattern.md` | Crate-level |
| ADR-076 | `ADR-076_cave_ctl_CLI_MCP_Server_Architecture.md` | CLI |
| ADR-143 | `ADR-143-cave-communication-hub.md` | Draft — see field gaps below |
| ADR-145 | `ADR-145_CRM_Upstream_Selection_Twenty.md` | Crate-level |
| ADR-146 | `ADR-146_Karpenter_Node_Autoscaling.md` | Crate-level |
| ADR-147 | `ADR-147_Data_Persistence_Crate_Naming_and_Lakehouse_Consolidation.md` | Consolidation |
| ADR-148 | `ADR-148_OSS_Launch_History_Strategy.md` | OSS launch |
| ADR-149 | `ADR-149_KubeVirt_Sovereign_VM_Workloads.md` | Crate-level |
| ADR-150 | `ADR-150_Hermes_Agent_Adoption_AC_Path.md` | Crate-level |
| ADR-151 | `ADR-151_Phantom_Crate_Audit_Cleanup.md` | Hygiene |
| ADR-152 | `ADR-152_LLM_Tracker_Daily_Always_Latest.md` | Crate-level |
| ADR-153 | `ADR-153_LLM_Gateway_MVP.md` | Crate-level |
| ADR-154 | `ADR-154_ArgoCD_GitOps_Adoption.md` | Crate-level |
| ADR-155 | `ADR-155_ArgoCD_Image_Updater_Adoption.md` | Crate-level (module ext) |
| ADR-157 | `ADR-157_Sigstore_Cosign_Adoption.md` | Crate-level |

**Topic-prefixed ADRs** (`-RUNTIME-`, `-PORTAL-`, `-MULTI-TENANT-`,
`-CONTRIB-`, `-SELF-IMPROVE-`): all carry their own numbering and are
not part of the numeric sequence above.

## Gaps in the numeric sequence

| Number | Status on `main` | Notes |
|--------|------------------|-------|
| ADR-144 | absent | Pre-OSS purge per ADR-148 — number not reused |
| ADR-156 | absent | Reserved/free (memory: "ADR-156 still free") |
| ADR-158 | absent on main | Held on feature branch `claude/cave-keycloak-2026-05-23-deep` (cave-keycloak deep port). Not yet merged. |
| ADR-159 | absent on main | Held on feature branch `claude/cave-k8s-2026-05-23-deep` (cave-k8s umbrella). Not yet merged. |

Per `docs/adr/README.md` — *"Gaps in the sequence are expected …
their numbers are not reused"* — gaps for ADR-144 and ADR-156 are
intentional. ADR-158 / ADR-159 are in-flight on feature branches and
will land in numeric order if/when those branches merge.

## Field-completeness audit

Legend — `.` = field present, `x` = field missing or unconventional header.

| ID / File | Title | Status | Date | Context | Decision | Consequences | Alternatives |
|-----------|:-----:|:------:|:----:|:-------:|:--------:|:------------:|:------------:|
| ADR-001 | . | x | x | . | . | . | x |
| ADR-031 | . | x | x | . | . | . | x |
| ADR-076 | . | . | x | . | . | . | x |
| ADR-143 | . | . (draft) | x | x | x | x | x |
| ADR-145 | . | x | x | . | . | . | . |
| ADR-146 | . | x | x | . | . | x | x |
| ADR-147 | . | . | . | . | . | x | x |
| ADR-148 | . | . | . | . | . | . | . |
| ADR-149 | . | x | x | . | . | x | x |
| ADR-150 | . | . | . | . | . | . | x |
| ADR-151 | . | . | . | . | . | . | x |
| ADR-152 | . | x | x | . | . | . | . |
| ADR-153 | . | x | x | . | . | . | . |
| ADR-154 | . | . | . | . | . | . | . |
| ADR-155 | . | x | x | . | . | x | x |
| ADR-157 | . | x | x | . | . | . | . |
| ADR-CONTRIB-ATTRIBUTION-001 | . | . | . | . | . | . | x |
| ADR-MULTI-TENANT-001 | . | . | . | . | . | . | . |
| ADR-PORTAL-AUTH-001 | . | . | . | . | . | . | . |
| ADR-PORTAL-DESKTOP-001 | . | . | . | . | . | . | x |
| ADR-PORTAL-PERSONAS-001 | . | . | . | . | . | . | . |
| ADR-RUNTIME-API-GATEWAY-CONSOLIDATION-001 | . | . | x | . | . | . | x |
| ADR-RUNTIME-CERT-LIFECYCLE-001 | . | x | x | . | . | . | x |
| ADR-RUNTIME-CLI-CONSOLIDATION-001 | . | . | x | . | . | . | x |
| ADR-RUNTIME-PARITY-100-PCT-001 | . | . | x | . | . | . | x |
| ADR-RUNTIME-PERSISTENCE-CONSOLIDATION-001 | . | . (draft) | x | . | . | . | x |
| ADR-RUNTIME-SANDBOX-NO-FFI-001 | . | . | x | . | . | . | x |
| ADR-RUNTIME-STACK-001 | . | . | x | . | . | . | x |
| ADR-RUNTIME-STREAMING-CONSOLIDATION-001 | . | . | x | . | . | . | x |
| ADR-RUNTIME-UPSTREAM-MIRROR-001 | . | . | x | . | . | . | x |
| ADR-RUNTIME-UPSTREAM-WATCH-001 | . | x | x | . | . | . | . |
| ADR-SELF-IMPROVE-001 | . | . | . | . | . | . | . |

### Observations

- **Date headers** are commonly carried inline with the Status field
  (e.g. `**Status:** Accepted (2026-04-26)`) rather than as a standalone
  `**Date:** ...` line. Both are conventionally accepted; the audit table
  flags only the standalone-`Date:` form for consistency.
- **Alternatives Considered** is the field most frequently omitted (≈18/32).
  This reflects the runtime-stack ADRs being upstream-driven (the choice
  *is* the upstream selection) rather than an open design space.
- **ADR-143** has none of the standard headers — it carries `Status:
  Proposed (Burak finalize edecek)` and is best treated as a draft until
  Burak finalises the structure.

## ADR numbering policy

Numbers are **stable** — no renumbering after merge. Gaps in the sequence
are expected (decisions that lived in the prior platform catalogue were
removed from this OSS repository; their numbers are not reused). See
`docs/adr/README.md` for the canonical statement.

## No edits applied

This audit is **report-only**. No ADR body was modified. Promoting any of
the field-completeness items above into an actual edit requires a decision
on whether the author intends to backfill the missing headers — that's a
content decision, not a hygiene one, and is left to Burak.
