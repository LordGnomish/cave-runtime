# Changelog

All notable changes to `cave-cert-manager` are recorded here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning tracks the parent workspace's Charter-v2 numbering.

## [Unreleased]

## [0.1.0] — 2026-05-23 — v2 deep-port close (Charter v2)

Charter v2 8-gate deep-port of cert-manager v1.20.2 — first public
release as part of the cave-runtime workspace v0.1.0 OSS launch line.

### Added

* **Core control plane** — Certificate / CertificateRequest / Issuer /
  ClusterIssuer CRDs, status conditions, IssuerRegistry, renewal
  scheduler that honours `renewBefore`, secret materializer that emits
  `kubernetes.io/tls` Secrets (tls.key as `keychain:` reference, never
  raw bytes), reconcile loop with structured `ReconcileEvent` emission.
* **Issuer backends** — ACME (HTTP-01 via cave-net plan / DNS-01 via
  cave-dns plan), CA (3-tier hierarchy through cave-pki — Sovereign
  Root → Platform Intermediate → per-tenant Intermediate), Vault
  (keychain-handle only — token never in process memory), SelfSigned.
  Venafi carried as model-only (runtime Phase 2).
* **HTTP API surface** — 11 endpoints under `/api/cert/*` plus
  `/metrics`. Tenant-scoped on every read; cross-tenant denial through
  `CertManagerError::CrossTenantDenied { owner_tenant, request_tenant }`.
* **cavectl driver** — `cavectl cert {issuer, cert, request, renew,
  verify, revoke, health, metrics}` subcommand tree.
* **Prometheus exposition** — 6 metric families
  (`certmanager_certificate_ready_status`,
  `certmanager_certificate_expiration_timestamp_seconds`,
  `certmanager_certificate_renewal_timestamp_seconds`,
  `certmanager_acme_client_request_count`,
  `certmanager_controller_sync_call_count`,
  `certmanager_certificate_revocation_total`). Deterministic BTreeMap
  ordering for golden tests; per-tenant cardinality bounded by
  `forget_certificate()`.
* **Revocation ledger** — RFC 5280 §5.3.1 reasonCodes
  (0|1|2|3|4|5|6|8|9|10 with 7 reserved); idempotent revoke;
  `certificateHold` → permanent upgrade path; `unhold` reverses only
  reversible reasons (`certificateHold` / `removeFromCrl`); CRL-line
  render for downstream responders.
* **Observability assets** — `observability/grafana-dashboard.json`
  (8 panels) + `observability/prometheus-alerts.yaml` (5 rules), both
  pinned to the `certmanager_*` families this crate emits.
* **Charter v2 self-audit** — `tests/parity_self_audit.rs` (11
  assertions); 0.95 floor; AGPL SPDX coverage walk; no-stub-macros
  walk; counts-sum-to-total invariant; metrics 5-family coverage; RFC
  5280 reasonCode round-trip.
* **Smoke suite** — `tests/smoke_end_to_end.rs` (5 scenarios):
  RenewalScheduler ascending order; SelfSigned issuance + renewal via
  scheduler; mock ACME HTTP-01 challenge; ACME DNS-01 digest-only
  emission; ACME issuer state isolated across issuer instances.
* **Edge-case suite** — `tests/test_gap_close_edges.rs` (65
  adversarial + boundary tests across validation, store, renewal,
  revocation, metrics, registry, controller, secrets, URL builders,
  error stability).

### Parity

* fill_ratio: 0.9500 ((23 + 5 + 10) / 40, manifest)
* honest_ratio: 0.7000
* mapped 23 / partial 5 / skipped 10 / unmapped 2
* upstream pinned to cert-manager v1.20.2 (`source_sha =
  e5b7b18450dd2c4b993b95bcd680b1a057205b00`)

### Test counts (Charter v2 four-track close)

* lib                  : 123 PASS
* parity_self_audit    :  11 PASS
* smoke_end_to_end     :   5 PASS
* test_gap_close_edges :  65 PASS
* ─────────────────────────────────
* TOTAL                : 204 PASS

### Honest gaps (carried as `[[unmapped]]`)

* `multi-issuer-failover` — operator-requested fall-through; not
  upstream.
* `experimental-clusterscoped-secret-rotation` — cert-manager v1.20
  alpha gate; deferred until the upstream flag GAs.

### Scope cuts (deferred to other crates / Phase 2)

* cloud issuers (AWS PCA + GCP KMS) → `cave-cert-manager-cloud-issuers`
* Venafi TPP / Cloud SDK runtime → `cave-cert-manager-venafi-runtime`
* Gateway-API HTTPRoute + Listener cert binding → `cave-gateway`
* istio-csr (gRPC SVID issuance) → `cave-mesh`
* mTLS certificate distribution → `cave-net`
* deep webhook validation → `cave-admission`
* central Prometheus aggregator + cross-crate Grafana dashboards →
  `cave-metrics`
* Helm chart installer → `cave-deploy`
* out-of-tree DNS solver gRPC webhook →
  `cave-cert-manager-acme-webhook-dns`
* cmctl binary → absorbed by `cavectl`

### Related ADRs

* ADR-155 — Cert-Manager Adoption (this crate)
* ADR-152 — cave-llm-tracker (keychain-first invariant)
* ADR-153 — cave-llm-gateway (keychain-first invariant)
* ADR-154 — ArgoCD Adoption / cave-deploy (downstream caller for
  cert-manager.io/Certificate health)
* ADR-147 — Persistence + Naming
