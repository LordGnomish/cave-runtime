# Cave Runtime — OSS launch repository metadata

This document is the source of truth for the GitHub repository's
**externally-visible metadata** at OSS launch (21 May 2026). Anything
on this page is intended for the GitHub repo settings UI, the social
preview card, or the v0.1.0 release notes.

> **Branding mandate:** No emoji in any of the strings below. No
> hyperscaler-specific provider branding (Hetzner / AWS / GCP / Azure)
> in user-facing text — Cave Runtime is provider-agnostic, sovereign
> by construction.

---

## 1. Repository description (single line)

Used in the GitHub repo header, search results, and most embed cards.
Hard cap is 350 characters but the practical render limit is ~150.

> **Sovereign Cloud OS in Rust — line-by-line TDD reimplementation of 80+ cloud-native upstreams (Kubernetes, etcd, Cilium, Istio, Kong, Keycloak, Prometheus, Loki, Grafana, Kafka, …) as a single AGPL binary. Multi-tenant by construction.**

(297 characters)

## 2. Repository topics

GitHub allows up to 20 topics. Listed in priority order — keep the
first ~5 strong because they dominate search ranking:

```
rust
cloud-native
kubernetes
sovereign-cloud
multi-tenant
agpl
observability
service-mesh
cilium
istio
keycloak
prometheus
loki
grafana
kafka
post-quantum-crypto
ebpf
io-uring
self-hosted
charter
```

Notes:

- `sovereign-cloud` is a deliberate Cave-coined topic; we want to own it.
- `agpl` makes the licensing posture immediately visible in search.
- `post-quantum-crypto` and `charter` are intentional Cave differentiators.

## 3. Homepage URL

```
https://cave-runtime.dev
```

(Placeholder — the dev domain points to a holding page until launch
day; flip to the real site at 2026-05-21 00:00 UTC.)

## 4. Social preview card

GitHub renders a 1280×640 PNG above the repo on social shares.
Specification for the card (no emoji per mandate):

| Slot | Content |
|------|---------|
| Top-left wordmark | `cave-runtime` |
| Headline | **Sovereign Cloud OS in Rust** |
| Sub-headline | Line-by-line TDD parity. 100+ crates. One binary. AGPL-3.0-or-later. |
| Bottom-left badge | `Charter v2 8-gate` |
| Bottom-right badge | `Honest measured fill_ratio` |
| Background | Dark slate (#0e1116 / #161b22), single-colour, no decorative imagery |
| Type | Inter or system-ui equivalent; no decorative glyphs |
| Accent | Single accent line — Cave palette `#3b82f6` |

The PNG itself is produced by `scripts/make-social-preview.sh` (to be
added in the launch-eve sweep); the spec above is the brief.

## 5. v0.1.0 release notes (draft)

```markdown
# Cave Runtime v0.1.0 — first public release

21 May 2026 — Sovereign Cloud OS in Rust.

This is the first public, AGPL-3.0-or-later licensed release of Cave
Runtime. It is **pre-1.0** — the API surface is stabilising, the parity
matrix is published live, and the Charter v2 8-gate is enforced in CI.

## Highlights

- **One binary, 100+ Rust crates.** Single workspace; `cargo build
  --release` produces one executable that contains the Kubernetes
  control plane, data plane, observability stack, mesh, gateway,
  identity, secrets, registry, scan, streams, autoscaling — every
  layer of a cloud-native platform under one license.
- **Line-by-line upstream parity.** Every crate ports a named upstream
  at a pinned `source_sha`. Test vectors are the upstream's own. No
  "core-path MVP", no behavioural approximation.
- **Honest measured `fill_ratio`.** No self-graded percentages. Live
  per-crate matrix at `/admin/parity` (or
  `docs/parity/parity-index.json` offline).
- **Multi-tenant by construction.** Every persisted resource carries a
  `tenant_id`. Default-deny between tenants. Per-tenant quota, SLO,
  billing — all first-class.
- **Post-quantum crypto migration in progress.** ML-KEM / ML-DSA /
  SLH-DSA at the primitives layer; hybrid handshakes by default in
  PQC-required modes.
- **Charter v2 8-gate** machine-enforced via `tests/parity_self_audit.rs`
  and `PARITY_REPORT.md` per crate.

## Recent parity highlights (Phase 2 deep-port)

- `cave-keda` 0.65 → **0.95** (KEDA v2.16.1 — 7 scalers, ScalingModifiers,
  hibernation)
- `cave-karpenter` 0.50 → **0.95** (Karpenter v1.4.0 — disruption,
  lifecycle, batcher, binpack)
- `cave-kamaji` 0.64 → **0.93** (Kamaji v1.0.0 — datastore, konnectivity,
  webhook, pod-mgmt, kubeadm)
- `cave-gitleaks` 0.66 → **0.94** (Gitleaks v8.29.1 — baseline, decoders,
  protect, stopwords, Extend, CSV, JUnit)
- `cave-knative` 0.75 → **0.93** (knative v1.22.0)
- `cave-metrics` 0.80 → **0.97** (Prometheus v3.3.0)
- `cave-dashboard` 0.80 → **0.95** (Grafana v11.5.0)
- `cave-streams` 0.87 → **0.96** (Kafka 4.2.0 + Pulsar v4.2.0)
- `cave-mesh` rebaselined as **Ambient-only** (Istio 1.30.0); 2710 LOC of
  sidecar / xDS v3 / Envoy WASM legacy removed.

## Known gaps

- Some Tier 2 crates remain at `fill_ratio < 0.90` and are tracked in
  `[[scope_cuts]]` per manifest. See `docs/parity/parity-index.json`.
- `cave-runtime cluster init` end-to-end multi-node bootstrap is being
  smoke-tested in production environments; the single-node path is
  fully ready.

## Getting started

- 10-line recipe: `docs/quickstart.md`.
- Run locally: `CAVE_JWT_SECRET=dev cargo run -p cave-runtime` → portal
  at <http://localhost:8080>.

## License

AGPL-3.0-or-later. See `LICENSE`, `NOTICE`, `CREDITS.md`. AGPL §13
closes the SaaS loophole: hyperscalers and other large SaaS vendors
cannot take Cave Runtime, modify it behind closed doors, and re-sell
the result as a closed managed service.

Sovereign-by-default. *insanlığa hizmet* — service to humanity.
```

## 6. Pre-launch checklist

- [ ] `README.md` polished — pitch + charter + license + 5-line
      quickstart + arch summary + parity matrix link + honest
      fill_ratio mandate + ecosystem cross-links + contributors CTA.
- [ ] `CONTRIBUTING.md` — TDD-strict workflow, Charter v2 8-gate,
      PR checklist, Conventional Commits, DCO sign-off.
- [ ] `CODE_OF_CONDUCT.md` — Contributor Covenant v2.1, TR+EN headings.
- [ ] `SECURITY.md` — disclosure, supported versions, 3-day ACK / 30-day fix SLA.
- [ ] `.github/workflows/ci.yml` — fmt + check (1.85 + stable) + clippy + test + parity_self_audit + doc.
- [ ] `.github/workflows/parity-audit.yml` — daily regen + stale detection.
- [ ] `.github/ISSUE_TEMPLATE/{bug_report,feature_request,parity_gap}.md` + `config.yml`.
- [ ] `.github/PULL_REQUEST_TEMPLATE.md` — 8-gate checklist.
- [ ] `docs/quickstart.md` — 10-line recipe + multi-node + sovereign-prod outline.
- [ ] `NOTICE` — all recent merges attributed; no Hetzner branding.
- [ ] `docs/oss-launch-metadata.md` (this file) — repo description, topics, social preview, release notes draft.
- [ ] Repo settings: description, topics, homepage — applied via GitHub UI/`gh repo edit`.
- [ ] Branch protection: `main` requires PR + status checks `fmt`, `check`, `clippy`, `test`, `parity-self-audit`, `doc`.
- [ ] `gh repo edit` invoked (manual step by Burak):

```bash
gh repo edit LordGnomish/cave-runtime \
  --description "Sovereign Cloud OS in Rust — line-by-line TDD reimplementation of 80+ cloud-native upstreams (Kubernetes, etcd, Cilium, Istio, Kong, Keycloak, Prometheus, Loki, Grafana, Kafka, …) as a single AGPL binary. Multi-tenant by construction." \
  --homepage "https://cave-runtime.dev" \
  --add-topic rust,cloud-native,kubernetes,sovereign-cloud,multi-tenant,agpl,observability,service-mesh,cilium,istio,keycloak,prometheus,loki,grafana,kafka,post-quantum-crypto,ebpf,io-uring,self-hosted,charter
```

## 7. Branding mandate (operator-only — do not surface publicly)

- No emoji anywhere in user-facing text.
- No hyperscaler provider branding (Hetzner / AWS / GCP / Azure) in
  README, NOTICE, CONTRIBUTING, social card, or release notes. The
  `Provider::Hetzner` enum variant in code is fine; user-facing prose
  must read "sovereign cloud" / "sovereign infrastructure" / "your own
  hardware".
- The Turkish phrase "insanlığa hizmet" is intentional and stays.
