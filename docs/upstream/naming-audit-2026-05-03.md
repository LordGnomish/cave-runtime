# Naming + Phantom Audit (2026-05-03)

> ⚠️ **CORRECTION (2026-05-03 23:35Z):** Sections 1 & 2 were wrong about
> ADR-147. The audit `find` ran against `$HOME/Code/cave-runtime/crates/`
> while the main repo dir was checked out to `feat/cave-upstream-watchd-001`
> (1478 commits behind main). On main HEAD, ADR-147 is **fully executed**:
> `cave-rdbms-operator/` and `cave-lakehouse/` exist as real workspace
> members; `cave-pg/` was deleted in commit `d7d6cb61`. Only
> `cave-vector-search` remains a genuine phantom (tracker target with no
> workspace crate). Section 1 still applies for `cave-vector-search`;
> Section 2's ADR-147 status is obsolete.
>
> Lesson: when the parent repo dir is on a non-main branch, run audits
> from a worktree that has main checked out, or use `git ls-tree -r main`
> instead of `find`.

Comprehensive cross-check of three sources of truth, after the cave-vcluster
drop (commit `17eafb2c`).

| Source | Count | What it says |
|---|---:|---|
| `crates/cave-upstream/src/projects.rs` `TRACKED_PROJECTS[].cave_module` | **54 unique** (68 rows; some upstreams share a module) | Which cave module each upstream port is supposed to land in |
| `Cargo.toml` `[workspace.members]` | **99** | Which crates `cargo` actually compiles |
| `crates/cave-*/` directories on disk | **103** | Which crates have skeletons (4 orphan dirs not in workspace) |
| `crates/cave-*/parity.manifest.toml` files | **103** | All FS dirs have a manifest (some are deprecated-alias stubs) |

Snapshot files used for diffing live in `/tmp/{tracker_modules,workspace_members,fs_crates,has_parity}.txt`.

---

## Section 1 — Phantom drops (tracker-side rows where the named module does not exist as a workspace member)

These are the rows that cause the `cargo check -p <X>` "package ID specification did not match" error in the qwen-pump cycle log. **Each row represents wasted pump CPU.**

| Tracker `cave_module` | Upstream | On FS? | Workspace member? | Verdict |
|---|---|:---:|:---:|---|
| `cave-rdbms-operator` | `cloudnative-pg/cloudnative-pg` | ❌ | ❌ | **ADR-147 incomplete rename** — target name has no crate. The actual port lives in `cave-pg/` (which still maps to `pgbouncer/pgbouncer` in its manifest, also wrong). See Section 2. |
| `cave-lakehouse` | `apache/iceberg-rust` + `apache/datafusion` | ❌ | ❌ | **ADR-147 incomplete rename** — target name has no crate. Old `cave-iceberg` and `cave-datafusion` still on disk as DEPRECATED ALIAS stubs (manifest header explicitly says "Bumps should go to cave-lakehouse, not here"), but `cave-lakehouse/` itself never created. |
| `cave-vector-search` | `qdrant/qdrant` | ❌ | ❌ | Target name has no crate. `cave-search/` on disk handles OpenSearch + Qdrant + Faiss + Milvus together — vector-search was apparently split out in tracker but never split out in workspace. |
| (cave-vcluster) | (loft-sh/vcluster) | (yes) | (no) | Already handled — commit `17eafb2c`. Listed for completeness. |

**Cat-1 phantoms total: 3 active (excluding the already-dropped vcluster).**

---

## Section 2 — ADR-147 (function-based naming) status — **rename is HALF-DONE**

Memory entry `adr-147-data-persistence-rename.md` says:
> old `cave-pg`/`cave-iceberg`/`cave-datafusion` are gone; use `cave-rdbms-operator` and `cave-lakehouse`

**Reality check:**

| Old name (ADR-147 says "gone") | Still on disk? | Still workspace member? | Manifest upstream | Notes |
|---|:---:|:---:|---|---|
| `cave-pg/` | ✅ | ✅ | `pgbouncer/pgbouncer` v1.21.0 | Manifest's upstream is *also* wrong — should be cloudnative-pg if this dir is meant to be the cave-rdbms-operator port; or pgbouncer if it's a connection-pool sidecar. Hybrid bug. |
| `cave-iceberg/` | ✅ | ✅ | DEPRECATED-ALIAS stub | Self-acknowledged stub. Safe to leave (or delete with all dependent re-exports). |
| `cave-datafusion/` | ✅ | ✅ | DEPRECATED-ALIAS stub | Same. |
| `cave-rdbms/` | ✅ | ✅ | `postgres/postgres` v16.0 | Confusion add-on: a *third* persistence-stack name nobody mentioned. Manifest points at upstream Postgres. |

| New target name | On disk? | Workspace member? | Tracker entry? |
|---|:---:|:---:|:---:|
| `cave-rdbms-operator/` | ❌ | ❌ | ✅ |
| `cave-lakehouse/` | ❌ | ❌ | ✅ |

**Bottom line:** Memory was accurate about the *intent*, wrong about the *execution state*. The rename happened in the tracker (`projects.rs`), but no `cave-rdbms-operator/` or `cave-lakehouse/` directory was created and no workspace.members entry exists. Pump bridge picks the new names, `cargo check` fails with `package ID specification did not match`, recurring forever.

### ADR-147 remediation choices

**Option A (true rename — preferred per ADR-147):**
1. `git mv crates/cave-pg crates/cave-rdbms-operator` (and fix Cargo.toml `name = "cave-rdbms-operator"`)
2. Create new `crates/cave-lakehouse/` with `Cargo.toml` re-exporting `cave-iceberg` + `cave-datafusion` (or merge their src/ trees)
3. Delete `cave-rdbms/` (third name, no clear charter purpose)
4. Update `[workspace.members]` accordingly
5. Manifest `cave-pg` upstream is wrong (`pgbouncer` instead of `cloudnative-pg`) — fix while renaming.

**Option B (rollback tracker — pragmatic for OSS launch):**
Revert tracker rows to use `cave-pg` and decide between `cave-iceberg`/`cave-datafusion` (or roll up to a single name). Memory was wrong; either rewrite memory to match reality, or honor memory by doing Option A.

Burak's call: A or B. ADR-147 itself is the authority here — if ADR-147 is in `docs/adr/` as Accepted, Option A is the only honest path.

---

## Section 3 — Composition meta-crates (correctly NOT in tracker)

Verified by reading each crate's `parity.manifest.toml`. These map to either `cave-runtime/cave-runtime` self-ref or carry a "First-party / no external upstream" header. **They are correctly excluded from `TRACKED_PROJECTS`.**

| Crate | Manifest says | Role |
|---|---|---|
| `cave-runtime` | "First-party meta-crate. No external upstream — umbrella composition" | Daemon binary that wires every cave-* module |
| `cave-kernel` | "First-party primitives crate. No external upstream" | Shared event bus, scheduler hooks, supervisor trees |
| `cave-cli` | "First-party CLI (`cavectl`). No external upstream" | Sovereign CLI; UX patterns mirror kubectl |
| `cave-core` | self-ref skeleton | Internal shared types |
| `cave-db` | self-ref skeleton | Internal DB layer |
| `cave-ledger` | self-ref skeleton | Internal append-only log |
| `cave-runbook` | self-ref skeleton | Internal runbook automation primitives |
| `cave-upstream` | (the tracker crate itself) | Internal |

**No action.** Nothing to add to tracker. (Pump phantom for `cave-runtime` is a different problem — see Section 6.)

---

## Section 4 — Genuine workspace members missing from tracker (real ports without tracking)

Crates that have a *real external upstream* in their parity.manifest.toml but no row in `TRACKED_PROJECTS`. **Tracker is incomplete; bump-task automation will skip these.**

| Crate | Manifest upstream | Action |
|---|---|---|
| `cave-portal-api` | parent `cave-portal` (Backstage backend API) | Either add row or document as "tracked via parent". Manifest already says "Bump dispatch happens against cave-portal" → leave OUT of tracker, but add a `# Sub-modules tracked via cave-portal: portal-api, portal-web, techdocs` comment in projects.rs near the cave-portal row. |
| `cave-portal-web` | parent `cave-portal` (Backstage frontend) | Same — sub-of cave-portal. |
| `cave-techdocs` | parent `cave-portal` (Backstage TechDocs plugin) | Same. |
| `cave-desktop` | `zed-industries/zed` (GPUI framework, not editor) | **Add tracker row.** GPUI scaffold for native desktop app. |
| `cave-tracing` | `jaegertracing/jaeger` v2 | **Conflict** — `cave-trace` already maps to jaeger/tempo. Either merge cave-tracing into cave-trace, or split tracker (cave-trace = backend, cave-tracing = SDK). Burak decision. |
| `cave-net` | `cilium/cilium` v1.19.3 | **Conflict** — `cave-ebpf-common` already maps to `cilium/cilium`. Same kind of split as tracing. Likely cave-ebpf-common = the eBPF object loader, cave-net = the L2/L3 cilium control plane. Need separate tracker rows. |
| `cave-permission` | `casbin/casbin` v3.10.0 | **Add tracker row.** New port, Casbin authorization. |
| `cave-pki` | `smallstep/certificates` v0.30.2 | **Add tracker row.** step-ca PKI. |
| `cave-acme` | `smallstep/certificates` v0.30.2 (same) | **Add tracker row** (or note as sub-of cave-pki). |
| `cave-cdc` | `debezium/debezium-server` v3.5.0.Final | **Add tracker row.** New port. |
| `cave-kamaji` | (probably clastix/kamaji per Charter) | **Add tracker row.** Replaces the dropped `cave-vcluster` semantically — cave-cluster's multi-tenant CP. |
| `cave-local-llm` | (probably ollama/ollama) | **Add tracker row.** Already running as the qwen-pump engine; track upstream. |
| `cave-oncall` | (probably grafana/oncall) | **Add tracker row.** Note: tracker already has Grafana OnCall mapped to `cave-incidents` — overlap; pick one and remove the other. |
| `cave-pam` | (gravitational/teleport per memory) | **Add tracker row.** Currently upstream-named (PAM = Privileged Access Mgmt is a function name, but reads like Linux PAM); consider renaming to `cave-bastion` or `cave-priv-access` separately. |
| `cave-status` | `louislam/uptime-kuma` | **Add tracker row.** Already tracker has uptime-kuma → `cave-uptime`; this is the second crate sharing same upstream. |
| `cave-changelog` | `towncrier/towncrier` (per the deleted bump-task list on feat branch) | **Add tracker row.** |
| `cave-tracker` | `linear-app/linear` (per audit-2026-05-02 row, marked DEAD) | **Add tracker row** — but flagged DEAD in last audit (linear repo missing). Consider drop. |
| `cave-secrets` | `trufflesecurity/trufflehog` | **Add tracker row.** Tracker has nothing for secret-scanning currently. |
| `cave-security` | `falcosecurity/falco` | **Add tracker row.** Tracker has nothing for runtime threat-detection currently. |
| `cave-cluster` | `kubernetes-sigs/cluster-api` (per audit-2026-05-02) | **Add tracker row.** This is the multi-cluster API; `cave-kamaji` is the multi-tenant CP. Two separate things. |
| `cave-compliance` | `open-policy-agent/gatekeeper` | **Add tracker row.** Tracker has OPA → `cave-policy` already; gatekeeper is the K8s admission wrapper. Distinct. |
| `cave-container-scan` | `aquasecurity/trivy` | **Add tracker row.** |
| `cave-cost-alloc` | `opencost/opencost` | **Conflict-or-split** — tracker already has `cave-cost ← opencost`. Decide: one or two? |
| `cave-crossplane` | `crossplane/crossplane` | **Conflict** — tracker already has `cave-infra ← crossplane/crossplane`. Two crates same upstream — pick one. |
| `cave-erp` | `erpnext/erpnext` | **Add tracker row.** Currently upstream-named; rename candidate (`cave-business-mgmt`?) — Burak decision. |
| `cave-gitops-config` | `fluxcd/flux2` | **Add tracker row.** |
| `cave-keda` | `kedacore/keda` | **Conflict** — tracker maps KEDA to `cave-ha`. Decide one or two. |
| `cave-knative` | `knative/serving` | **Conflict** — tracker maps Knative to `cave-deploy` (alongside argo-cd!). Decide. |
| `cave-kube-proxy` | `kubernetes/kubernetes` | **Add tracker row.** Like cave-kubelet — function-named OK (matches K8s component name). |
| `cave-lint` | `SonarSource/sonarqube` | **Add tracker row.** Conflict-or-split with `cave-scan ← sonarqube`. |
| `cave-pipelines` | `tektoncd/pipeline` | **Add tracker row.** |
| `cave-scaffold` | `backstage/backstage` | **Add tracker row.** Note: separate from cave-portal even though both touch Backstage. |
| `cave-artifacts` | `pulp/pulpcore` | **Add tracker row.** |
| `cave-alerts` | `prometheus/alertmanager` | **Add tracker row.** Distinct from `cave-metrics ← prometheus/prometheus`. |

**Genuine missing-tracker count: ~30 rows worth of work** to get tracker coverage to 100% of workspace members (excluding meta-crates).

---

## Section 5 — Crates on FS but NOT in workspace (orphan scaffolds)

| Crate dir | Manifest exists? | Workspace? | Verdict |
|---|:---:|:---:|---|
| `crates/cave-vcluster/` | ✅ | ❌ | Already dropped from tracker. Dir can be deleted (post-launch). |
| `crates/cave-spire/` | ✅ | ❌ | Charter overlap with cave-mesh / cave-auth identity surfaces. Decide: add member or delete. |
| `crates/cave-hubble/` | ✅ | ❌ | Tracker has `cilium/hubble → cave-forensics`. The dir name follows upstream (cave-hubble) — ADR-147-style violation. Delete dir, fold any code into cave-forensics. |
| `crates/cave-external-secrets/` | ✅ | ❌ | Tracker has `external-secrets/external-secrets → cave-vault`. Same story — delete dir, fold into cave-vault. |

**Recommended action: delete all 4 orphan dirs.** None are workspace members, none are referenced by any other crate (verified via `cargo check` not erroring on missing deps). Each represents an upstream-named scaffold that should have been folded into a function-named home (cave-forensics / cave-vault) per ADR-147 spirit.

---

## Section 6 — Pump rotation cleanup (separate from tracker — bridge-side)

Names that show up in `~/Library/Application Support/cave-qwen-pump/queue.txt` but are not appropriate for the pump's "5 red test scaffolds" gate:

| Phantom queue name | Reason it can't pass pump gate | Action |
|---|---|---|
| `cave-runtime` | Bin-only meta-crate (modules=`main`); no public surface for qwen to cover with 5 tests. | Drop from queue feeder + add to bridge deny-list. |
| `cave-cli` | Cargo.toml `name = "cavectl"` (per ADR-RUNTIME-CLI-CONSOLIDATION-001). Tracker source emits `cave-cli`, but `cargo check -p cave-cli` fails — need to emit `cavectl`. | Rename source-of-truth in projects.rs (it already says `cave-cli`, doesn't appear in tracker actually — comes from bridge). |
| `cave-desktop` | GPUI scaffold, modules=`main`, same too-few-tests gate. | Drop from queue feeder until real UI surface lands. |
| `cave-portal-api` | Sub-of-cave-portal stub with no public API surface yet. | Drop from queue feeder. |
| `cave-loki` | No FS dir, no Cargo.toml — truly orphan. **(Tracker mapped Loki → cave-logs at some point; cave-loki is bridge-side phantom.)** | Drop from bridge state. |
| `cave-lakehouse` | Target name doesn't have a crate yet (Section 2). | Drop from queue feeder until ADR-147 rename completes. |
| `cave-local-llm` | Has surface but `[[bin]]` named cave-local-llm-daemon; `name=cave-local-llm` lib has minimal exposed surface → too-few-tests. | Investigate prompt or drop. |
| `cave-tracing` | New jaeger-v2 port; tracker conflict with cave-trace (Section 4). | Drop from queue feeder until Section 4 conflict resolved. |
| `cave-spire` / `cave-hubble` / `cave-external-secrets` | Orphan scaffolds (Section 5). | Drop after Section 5 delete. |

**These are bridge-side, not tracker-side.** The bridge inbox `~/Library/Application Support/cave-qwen-pump/queue/` keeps `processed/` and `skipped/` JSONs; the source-of-truth for *what gets enqueued in the first place* is `cave-upstream-watchd` (PID 3222). After tracker cleanup (Sections 1+4), the watchd binary needs a rebuild + reload.

---

## Section 7 — Final cleanup script template (do NOT run unattended)

```bash
#!/usr/bin/env bash
# naming-cleanup-2026-05-03.sh — REVIEW THE PER-STEP COMMENTS FIRST.
# This script encodes the audit's recommendations. It is not idempotent
# and several steps depend on Burak per-row decisions (Sections 2, 4, 5).

set -euo pipefail
cd $HOME/Code/cave-runtime

# ---------------------------------------------------------------------------
# Step 1 — Section 1 phantom drops (tracker-side rows pointing to non-crates).
# Skip rows that Burak wants to fix-by-creating-the-crate (Option A) instead.
# ---------------------------------------------------------------------------
# Edit crates/cave-upstream/src/projects.rs:
#   - Remove or comment-out: cave_module: "cave-rdbms-operator" rows (1)
#                            cave_module: "cave-lakehouse"      rows (2)
#                            cave_module: "cave-vector-search"  rows (1)
# until the corresponding workspace crate exists.

# ---------------------------------------------------------------------------
# Step 2 — Section 2 ADR-147 reconciliation. CHOOSE Option A or Option B.
# ---------------------------------------------------------------------------
# Option A (rename to honor ADR-147):
#   git mv crates/cave-pg crates/cave-rdbms-operator
#   sed -i '' 's/name = "cave-pg"/name = "cave-rdbms-operator"/' \
#     crates/cave-rdbms-operator/Cargo.toml
#   # Cargo.toml [workspace.members]: rename "crates/cave-pg" → "crates/cave-rdbms-operator"
#   # (Manual edit — also fix the manifest upstream from pgbouncer to cloudnative-pg)
#
#   mkdir -p crates/cave-lakehouse/src
#   # Create cave-lakehouse Cargo.toml that re-exports cave-iceberg + cave-datafusion
#   # OR: merge their src/ trees into cave-lakehouse and delete cave-iceberg/, cave-datafusion/
#
#   git rm -r crates/cave-rdbms       # third confused name
#
# Option B (rollback tracker to old names):
#   Edit projects.rs cave_module values:
#     cave-rdbms-operator → cave-pg
#     cave-lakehouse       → ??? (cave-iceberg or new)
#   and update memory/adr-147-data-persistence-rename.md to say "REVERTED".

# ---------------------------------------------------------------------------
# Step 3 — Section 4 tracker fills (~30 rows). Manual edits to projects.rs.
# Group: networking (cave-net), persistence (cave-pki/cave-acme), policy
# (cave-permission, cave-compliance), supply-chain (cave-secrets, cave-sign,
# cave-container-scan, cave-cdc), platform (cave-pipelines, cave-knative,
# cave-keda, cave-crossplane, cave-kamaji, cave-local-llm, cave-cluster,
# cave-kube-proxy), observability (cave-status, cave-tracing, cave-alerts,
# cave-oncall, cave-cost-alloc), product (cave-erp, cave-tracker, cave-pam,
# cave-changelog, cave-scaffold, cave-artifacts, cave-lint, cave-security,
# cave-desktop, cave-gitops-config, cave-portal-api, cave-portal-web,
# cave-techdocs).
#
# Resolve conflicts row-by-row (cave-knative vs cave-deploy, cave-keda vs
# cave-ha, etc. — Burak per-pair decision).

# ---------------------------------------------------------------------------
# Step 4 — Section 5 orphan dir deletions.
# Only run after confirming `cargo check --workspace` is green WITHOUT them.
# ---------------------------------------------------------------------------
# git rm -r crates/cave-vcluster crates/cave-spire crates/cave-hubble \
#           crates/cave-external-secrets

# ---------------------------------------------------------------------------
# Step 5 — Section 6 bridge deny-list. After cave-upstream-watchd code change.
# ---------------------------------------------------------------------------
# Edit crates/cave-upstream/src/projects.rs (or watchd config):
#   add a const PUMP_DENY_LIST: &[&str] = &[
#       "cave-runtime", "cave-cli" /* until cavectl rename */,
#       "cave-desktop", "cave-portal-api", "cave-portal-web", "cave-techdocs",
#       "cave-loki", "cave-lakehouse", "cave-rdbms-operator",
#       "cave-vector-search",
#   ];
# Filter in the bridge JSON-emit step.

# ---------------------------------------------------------------------------
# Step 6 — Rebuild + reload watchd to pick up new tracker.
# ---------------------------------------------------------------------------
# cargo build --release -p cave-upstream
# launchctl unload ~/Library/LaunchAgents/com.btartan.cave-upstream-watchd.plist
# launchctl load   ~/Library/LaunchAgents/com.btartan.cave-upstream-watchd.plist

# ---------------------------------------------------------------------------
# Step 7 — Memory updates.
# ---------------------------------------------------------------------------
# Edit ~/.claude/.../memory/adr-147-data-persistence-rename.md:
#   Replace "executed; old crates are gone" with the actual end state
#   (Option A or B). Memory must match reality.
```

---

## Honest assessment

| Bucket | Count | Effort |
|---|---:|---|
| **Section 1 — tracker rows pointing to non-existent crates** | 3 | 30 min (drop or fill via Section 2) |
| **Section 2 — ADR-147 incomplete (memory was wrong about execution state)** | 4 dirs (cave-pg, cave-iceberg, cave-datafusion, cave-rdbms) + 2 missing target dirs | 2-4h Option A; 30 min Option B; **needs Burak decision** |
| **Section 3 — composition meta-crates** | 8 | 0 — already correct |
| **Section 4 — genuine missing tracker rows** | ~30 | 4-6h editing projects.rs + per-pair conflict decisions |
| **Section 5 — orphan FS dirs (no workspace)** | 4 | 30 min (`git rm -r`, verify cargo green) |
| **Section 6 — bridge deny-list / watchd rebuild** | 1 code change + 1 reload | 30-60 min |

**Total cleanup scope: 8-12 hours of focused work** + a couple of Burak per-row decisions (Options A/B in §2; conflict-or-split pairs in §4).

**OSS launch impact:** Sections 1, 2, 5, 6 are pump-CPU-recovery (phantom yangını kalıcı çözümü). Section 4 is parity-coverage completeness — important for OSS launch credibility (so the README's "we track upstream X for module Y" claims are honest), but pump phantom-fire goes out without it.

**Critical realization:** Memory entry `adr-147-data-persistence-rename.md` says the rename "executed". Filesystem says "tracker side renamed; workspace side untouched." This is the root cause of the entire pump phantom yangını — and it's been masked by the memory claim. **Memory must be corrected after this audit lands**, regardless of which option is taken.

---

## Source data used

Snapshot files (regenerable):
- `/tmp/tracker_modules.txt` — 54 unique cave_module names from projects.rs
- `/tmp/workspace_members.txt` — 99 names from Cargo.toml
- `/tmp/fs_crates.txt` — 103 names from `find crates/cave-*`
- `/tmp/has_parity.txt` — 103 dirs with parity.manifest.toml

Diff one-liners used:
```bash
comm -23 /tmp/tracker_modules.txt /tmp/workspace_members.txt   # Section 1
comm -13 /tmp/tracker_modules.txt /tmp/workspace_members.txt   # Section 4
comm -13 /tmp/workspace_members.txt /tmp/fs_crates.txt         # Section 5
comm -23 /tmp/has_parity.txt /tmp/tracker_modules.txt          # Section 4 expanded
```
