# ADR-155: Cert-Manager Adoption — cave-cert-manager

**Status:** Accepted

**Date:** 2026-05-22

**Owner:** Burak (btartan@gmail.com)

**Scope:** `crates/cave-cert-manager` (new control-plane crate inside the cave-runtime workspace)

**Category:** PKI, Certificate Lifecycle, Kubernetes, OSS Adoption

**Related ADRs:** ADR-147 (Persistence + Naming), ADR-152 (cave-llm-tracker), ADR-153 (cave-llm-gateway), ADR-154 (ArgoCD Adoption / cave-deploy)

---

## Context

The cave-runtime workspace already carries the storage + sovereignty
layer for certificates: `cave-pki` owns the Root → Platform
Intermediate → per-tenant Intermediate hierarchy, `cave-acme` ships
an RFC 8555 ACMEv2 server, `cave-certs` carries the cert lifecycle
storage, and `cave-vault` ships the keychain bridge. What was missing
is the cert-manager.io **control-plane**: the Issuer / ClusterIssuer /
Certificate / CertificateRequest CRDs, the per-backend dispatch
(ACME / CA / Vault / SelfSigned), the renewal scheduler that obeys
`renewBefore`, and the secret reconciler that materialises
`kubernetes.io/tls` Secrets into the namespaces workloads consume.

The cave-knative close (memory `cave-knative deep port`) and the
cave-deploy ArgoCD port (ADR-154) both park cert-manager wiring as a
Phase 2 backlog item — cave-knative's `cert_bridge` and ArgoCD's
Application health for `cert-manager.io/Certificate` resources both
expect a first-party CRD surface to call into. This ADR lands that
surface so neither has to drift back to an external cert-manager
binary at deploy time.

The MVP needs to land a deep port — not a wrapper — so that:

1. The four-issuer registry (ACME / CA / Vault / SelfSigned) is
   exercised by Charter v2's self-audit gates rather than tunnelled
   through Go.
2. The renewal scheduler + reconcile loop + secret materializer can
   be reused by `cave-portal-api`, `cave-deploy`, `cave-knative`, and
   `cave-net` without a process boundary.
3. Vault credentials live behind a `keychain:` handle and never as a
   plaintext field in the manifest — matching the keychain-first
   invariant established for cave-llm-tracker (ADR-152) and
   cave-llm-gateway (ADR-153).
4. The upstream parity is honestly measured against the latest stable
   cert-manager release, with `source_sha` pinned and
   `parity_ratio_source = "manifest"` so the daily parity-index regen
   (`com.cave.parity-index-regen`) picks it up.

## Decision

Adopt **cert-manager/cert-manager v1.20.2** (Apache-2.0, source_sha
`e5b7b18450dd2c4b993b95bcd680b1a057205b00`, published 2026-04-11) as
the deep-port upstream for the new `cave-cert-manager` crate.

The crate carries:

* `models.rs` — Certificate / CertificateRequest / Issuer /
  ClusterIssuer CRDs + status conditions + `IssuerSpec` as a typed
  sum type (Acme / Ca / Vault / SelfSigned / Venafi) so the type
  system enforces "exactly one issuer kind"
* `issuer.rs` — `IssuerRegistry` routes by spec variant; Venafi
  runtime is rejected at the dispatcher (model only, Phase 2)
* `acme_issuer.rs` — drives `cave_acme::AcmeServer` through
  newAccount → newOrder → solve(authz) → finalize; HTTP-01 publishes
  `Http01Plan` records for cave-net; DNS-01 publishes per-zone
  `Dns01Plan` records (record_name = `_acme-challenge.<domain>`,
  digest = base64url(SHA-256(key_authorization))) for cave-dns;
  longest-zone solver match
* `ca_issuer.rs` — lazy-bootstraps Cave Sovereign Root + Platform
  Intermediate then reuses or generates per-tenant intermediates via
  `cave_pki::Ca` (one intermediate per `tenant_id`, cave invariant)
* `vault_issuer.rs` — rejects handles without `keychain:` scheme +
  requires the entry to be present; mixes the resolved token into the
  synthetic serial without leaking it into the emitted PEM
  (token-value scrub test)
* `selfsigned_issuer.rs` — signs against its own request; CRL
  distribution points + `isCA` round-trip through the emitted PEM
* `renewal.rs` — `RenewalScheduler.plan` returns sorted
  `RenewalPlan`s with `InitialIssuance / NotReady / Expired /
  RenewBeforeReached` reasons
* `secret.rs` — `SecretMaterializer` emits `kubernetes.io/tls`
  Secrets; `tls.key` is materialised as a `keychain:` reference,
  NEVER as raw bytes; secretTemplate labels + annotations propagate
* `store.rs` — namespaced indexers w/ cross-tenant denial
  (`CertManagerError::CrossTenantDenied { owner_tenant, request_tenant }`)
* `controller.rs` — five-step reconcile (`project → resolve_issuer
  → dispatch → materialise → stamp`); emits `ReconcileEvent::{Issued,
  Renewed, Failed}`
* `routes.rs` — 8 endpoints under `/api/cert/*` (health, certificates
  list/create/get/issue/renew, certificate-requests list, issuers
  list/create, cluster-issuers list/create)
* `cli.rs` — URL builders used by `cavectl cert {issuer, cert,
  request, renew, health}` in `crates/cave-cli/src/main.rs`

cave-acme grows exactly one new public accessor — `authorization(
tenant, id)` — which is the only API needed for external solvers to
walk an Order's per-identifier Challenge list. No other cave-acme /
cave-pki surface was modified.

## Out of MVP (skipped via `[[scope_cuts]]`)

* **AWS PCA + Google CloudKMS issuers** → cave-cert-manager-aws /
  cave-cert-manager-gcp Phase 2 crates so the core MVP stays
  cloud-agnostic and dependency-light
* **Venafi issuer runtime** → cave-cert-manager-venafi Phase 2
  (Venafi TPP/Cloud SDKs are GPL/commercial)
* **Gateway-API HTTPRoute + Listener cert binding** → cave-gateway
* **istio-csr gRPC SVID issuance** → cave-mesh
* **mTLS certificate distribution** → cave-net (data-plane runtime)
* **Webhook validation deep** → cave-admission
* **Prometheus metrics exporter + Grafana dashboards** →
  cave-metrics + cave-dashboard
* **cmctl binary** → absorbed into cavectl `cert` subcommand
* **Helm chart installer** → cave-deploy + Helm runtime
* **ACME webhook DNS provider runtime** → Phase 2 once cave-net
  carries the gRPC server scaffold

## Out of MVP (honest gaps preserved via `[[unmapped]]`)

* `multi-issuer-failover` — operator-requested fall-through; not in
  cert-manager upstream
* `experimental-clusterscoped-secret-rotation` — cert-manager v1.20
  alpha feature gate

## Alternatives considered

| Alternative | Why rejected |
| ----------- | ------------ |
| Shell out to upstream `cert-manager` Go binary | breaks the AGPL boundary, breaks the Charter v2 self-audit (no honest LOC), needs a separate process + service account |
| Generate Rust bindings off the cert-manager OpenAPI CRDs | the controller logic is the value — generating the schemas leaves the dispatcher + renewal scheduler + secret reconciler unimplemented |
| Repurpose `cave-certs` (already at 0.0 against cert-manager) | cave-certs is the storage layer; conflating storage with the CRD control-plane buys nothing and breaks the per-crate parity floor model — better to keep one cert-manager-shaped crate (`cave-cert-manager`) and one cert lifecycle storage crate (`cave-certs`) |
| Wait for a community Rust port | cert-manager is the de-facto standard; no production-grade Rust port exists |

## Consequences

* `cave-cert-manager` carries the cert-manager CRD surface end-to-end,
  reachable from cave-portal-ui + cavectl + cave-knative cert-bridge
  without an external binary
* `cave-acme` exposes one new public accessor (`authorization(tenant,
  id)`) — fully additive, no breaking changes
* Vault token + ACME account key material NEVER appears in process
  memory beyond keychain resolution; NEVER in emitted PEM; NEVER in
  manifest YAML
* Per-tenant CA intermediates remain one-per-tenant_id (cave
  invariant); cross-tenant reads return a structured
  `CrossTenantDenied { owner_tenant, request_tenant }` error
* `cavectl cert` parallels the existing `cavectl certs` (cave-certs)
  surface without collision
* Workspace ≥ 0.95 count grows by 1 (new crate above the threshold)

## Charter v2 8-gate close

| Gate | Status |
| ---- | ------ |
| G1 — always-latest (v1.20.2) | ✅ |
| G2 — source_sha pinned | ✅ |
| G3 — fill_ratio ≥ 0.65 (got 0.9474) | ✅ |
| G4 — parity_ratio_source = "manifest" | ✅ |
| G5 — AGPL SPDX 100% | ✅ |
| G6 — no stub macros in src/ | ✅ |
| G7 — counts sum + ≥ 15 mapped (21 mapped) | ✅ |
| G8 — TDD + 4-track + ≥ 0.65 floor (92 PASS) | ✅ |

## Smoke

`cargo test -p cave-cert-manager` → **92 PASS** (78 lib + 9 self-audit + 5 smoke).
Smoke covers: SelfSigned issuance → secret materialisation → scheduler-driven renewal (revision 1 → 2); ACME HTTP-01 mock challenge → exactly-one `Http01Plan` per identifier; ACME DNS-01 → 43-char base64url-no-pad digest never reveals the raw token; renewal scheduler ordering ascending; ACME state isolated from other issuer backends.
