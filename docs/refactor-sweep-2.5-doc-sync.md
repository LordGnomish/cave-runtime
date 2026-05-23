# Refactor sweep — Phase 2.5 doc/ADR sync report (2026-05-23)

Status: report-only audit. No mass-generation of placeholder docs.

## ADR catalogue

- `docs/adr/README.md` index entries: **14**
- `docs/adr/ADR-NNN-*.md` files on disk: **14**
- **In sync.** No gap to fix.

Numbered sequence on disk: 001, 031, 076, 143, 145, 146, 147, 148, 149,
150, 151, 152, 153, 157. Gaps are historical — ADR-154, 158, 159 live
on unmerged feature branches (cave-deploy, cave-keycloak, cave-k8s) and
will land when those merge.

## Per-crate docs coverage

| Doc          | Present | Missing | Notes |
|--------------|---------|---------|-------|
| README.md    |    24   |    87   | Most crates ship without a crate-level README |
| CHANGELOG.md |     0   |   111   | No crate ships a CHANGELOG; release notes live in GitHub releases |
| parity.manifest.toml | 108 |   3   | The 3 missing are orphan sibling-ray dirs (cave-apigw / cave-cilium / cave-dependency-track) not in workspace |

**Decision (this sweep):** do NOT auto-generate placeholder README /
CHANGELOG files. Templated stubs (`# cave-X\n\nTODO`) are noise that
hides which crates actually have prose. Leave to follow-up where each
crate's owner adds purposeful content.

## parity-index.json sync

- `docs/parity/parity-index.json`: **108 crates indexed.**
- `crates/cave-*/parity.manifest.toml`: **108 manifests on disk.**
- **In sync** by count. Per-crate `fill_ratio` reconciliation happens in
  Phase 2.8 via `scripts/build-parity-index.py`.

## synergy/chain

- `docs/synergy/` does not exist in this branch. Cross-crate dependency
  graph regeneration deferred — `cargo tree --workspace --depth 1`
  output captured in `.metrics/deps-before.txt` as the raw artefact.

## Items left for a later pass

- Generate `README.md` per crate (87 crates) with purposeful prose —
  out of scope for a sweep; needs per-crate context.
- Generate `CHANGELOG.md` per crate (111 crates) — same.
- `docs/synergy/chain.md` cross-crate dep graph from `cargo tree`.
- Reconcile sibling-ray ADR-154/158/159 once those branches merge.
