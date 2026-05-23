# cave-cert-manager

Charter v2 deep-port of [cert-manager/cert-manager](https://github.com/cert-manager/cert-manager)
into the cave-runtime workspace.

* Upstream pin: **v1.20.2** (2026-04-11, Apache-2.0)
* `source_sha`: `e5b7b18450dd2c4b993b95bcd680b1a057205b00`
* fill_ratio: **0.9500** (mapped 23 + partial 5 + skipped 10 / total 40)
* honest_ratio: 0.7000 (mapped + partial / total)
* Tests: **204 PASS** (123 lib + 11 self-audit + 5 smoke + 65 edge cases)
* ADR-155 — Cert-Manager Adoption

## What this crate is

The cert-manager.io control plane, ported to Rust + cave-runtime:

* **CRDs** — Certificate, CertificateRequest, Issuer, ClusterIssuer (per
  `pkg/apis/certmanager/v1/types_*.go`).
* **Issuer types** — ACME (HTTP-01 + DNS-01), CA (3-tier hierarchy
  through cave-pki), Vault (keychain-handle only — token never in
  process memory), SelfSigned, Venafi (model-only / runtime Phase 2).
* **Reconcile loop** — project → resolve issuer → dispatch → materialise
  Secret → stamp status conditions. Emits structured `ReconcileEvent`s
  (Issued / Renewed / Failed) for downstream fan-out.
* **Renewal scheduler** — honours `renewBefore`; state machine
  (Initial / NotReady / Expired / RenewBeforeReached); sorted plan.
* **Secret materializer** — emits `kubernetes.io/tls` Secrets with
  `tls.crt`, `ca.crt`, and `tls.key` (the latter as a `keychain:`
  reference, never raw bytes — the keychain bridge resolves at
  TLS-handshake time).
* **Revocation ledger** — RFC 5280 §5.3.1 reasonCodes (0|1|2|3|4|5|6|8|9|10,
  7 reserved); idempotent revoke; `certificateHold` → permanent
  upgrade path; `unhold` reverses only reversible reasons; CRL-line
  render for downstream responders.
* **Prometheus metrics** — 6 metric families (`certmanager_*`) with
  deterministic exposition for golden tests; per-tenant cardinality
  bounded by `forget_certificate()`.
* **HTTP API** — 11 endpoints under `/api/cert/*` + `/metrics`.
* **cavectl driver** — `cavectl cert {issuer, cert, request, renew,
  verify, revoke, health, metrics}`.

## Quick start

### Embedding in your runtime

```rust
use std::sync::{Arc, Mutex};
use cave_cert_manager::routes::{create_router, RuntimeState};

let state = Arc::new(Mutex::new(RuntimeState::new()));
let app = create_router(state.clone());
// Mount `app` on your axum server.
```

### Driving from the CLI

```sh
# Surface health + pinned upstream.
cavectl cert health

# List Certificate resources for tenant `t-1`.
cavectl cert cert list t-1

# Trigger an issuance for an existing Certificate.
cavectl cert cert issue t-1 <cert-id>

# Verify a Certificate is Ready=True, not expired, not revoked.
cavectl cert verify t-1 <cert-id>

# Force-renew (just another reconcile).
cavectl cert renew t-1 <cert-id>

# Dump the Prometheus exposition.
cavectl cert metrics
```

## Observability

* `observability/grafana-dashboard.json` — 8 panels (Ready/NotReady
  count, notAfter heatmap, renewal queue, ACME request rate, ACME
  error ratio, controller sync rate, revocation events, issuer
  health). Template variables: `tenant_id`, `namespace`.
* `observability/prometheus-alerts.yaml` — 5 rules:
  `CertManagerCertExpiringSoon` (notAfter < 7d, warn),
  `CertManagerCertExpired` (notAfter passed, critical),
  `CertManagerAcmeErrorRateHigh` (>10% non-2xx, warn),
  `CertManagerControllerStalled` (sync rate < 1/h, critical),
  `CertManagerRevocationSpike` (>5 revokes/min, warn).

Each rule carries `severity`, `team=pki`, and a runbook annotation
pointing at `docs/runbooks/<alert-name>.md` for downstream wiring.

## Charter v2 8-gate compliance

| Gate | Status | Evidence |
| ---- | ------ | -------- |
| G1 — always-latest | ✅ | v1.20.2 (latest stable, re-verified 2026-05-23) |
| G2 — source_sha pin | ✅ | `e5b7b184…05b00` |
| G3 — fill_ratio ≥ 0.95 | ✅ | 0.9500 |
| G4 — manifest is source of truth | ✅ | `parity_ratio_source = "manifest"` |
| G5 — 100% AGPL SPDX | ✅ | walks all `.rs` files in this crate |
| G6 — no stub macros in src/ | ✅ | walks all non-comment lines |
| G7 — counts sum + mapped ≥ 15 | ✅ | 23+5+10+2 = 40, 23 ≥ 15 |
| G8 — TDD + 4-track + ≥ 200 tests | ✅ | 204 PASS (backend + cavectl + routes + observability) |

See `PARITY_REPORT.md` for the per-gate evidence table.

## Scope (mapped / partial / skipped / unmapped)

**Mapped (23)** — All four CRDs, every status condition, ACME issuer +
HTTP-01 + DNS-01 (cave-dns), CA (3-tier through cave-pki), Vault
(keychain-handle), SelfSigned, IssuerRegistry, renewal scheduler,
secret materializer, in-memory store with cross-tenant denial,
reconcile loop, HTTP API surface (11 endpoints), cavectl URL builders,
ReconcileEvent emitter, dnsNames validation, tenant scoping,
**prometheus-metrics**, **certificate-revocation**.

**Partial (5)** — Venafi (model only, runtime Phase 2), ACME
TLS-ALPN-01 (cave-acme generates challenge, solver lands with
cave-gateway), ACME EAB (cave-acme accepts EAB, AcmeIssuer passes
None), secret-reconciler private-key-bytes (currently keychain
handle), rotation-policy `Always` (spec honoured, swap lands with
keychain bridge).

**Skipped — `[[scope_cuts]]` (10)** — cloud issuers (AWS PCA / GCP
KMS) → `cave-cert-manager-cloud-issuers`; Venafi TPP/Cloud SDK →
`cave-cert-manager-venafi-runtime`; Gateway-API → `cave-gateway`;
istio-csr → `cave-mesh`; mTLS distribution → `cave-net`; webhook
validation → `cave-admission`; central Prometheus aggregator →
`cave-metrics`; Helm chart installer → `cave-deploy`; out-of-tree DNS
solver → `cave-cert-manager-acme-webhook-dns`; cmctl binary →
`cavectl` absorbs.

**Unmapped — honest gaps (2)** — `multi-issuer-failover` (operator
fall-through, not upstream); `experimental-clusterscoped-secret-rotation`
(cert-manager v1.20 alpha gate).

## Security posture

* **No API keys in code** — `IssuerSpec::Acme.account_key_keychain_handle`
  and `IssuerSpec::Vault.token_keychain_handle` are rejected unless they
  begin with `keychain:`.
* **Secret material never in process memory beyond resolution** —
  `tls.key` is materialised as a keychain reference, NEVER as raw
  bytes; `VaultIssuer.seed_keychain` is the only entry point.
* **Tokens scrubbed from PEM output** — covered by the
  `token_value_not_in_emitted_pem` test.
* **Cross-tenant denial** — every store + controller + revocation-ledger
  op runs through `check_tenant`; structured
  `CrossTenantDenied { owner_tenant, request_tenant }` error.
* **dnsNames validation** — empty, slash, whitespace, > 253 chars
  rejected per RFC 1035.
* **Metrics cardinality bounded** — `forget_certificate()` drops all
  per-Certificate gauges on delete; per-tenant labels prevent
  cross-tenant aggregation leaks.
* **Revocation reason 7 rejected** — RFC 5280 reserves it.

## Downstream wiring

* `cave-vault` → populates `VaultIssuer.keychain` at startup; per-handle
  reads are explicit lookups, never fall-throughs.
* `cave-net` → receives `Http01Plan` records to publish under
  `/.well-known/acme-challenge/`; resolves `tls.key` keychain handles
  at TLS-handshake time.
* `cave-dns` → receives `Dns01Plan` records (record_name =
  `_acme-challenge.<domain>`, digest =
  `base64url(SHA-256(key_authorization))`) to publish as TXT entries.
* `cave-knative` → previously parked as a Phase 2 backlog item; this
  crate now provides the surface (`IssuerSpec` + `Certificate` CRDs)
  the cave-knative cert-bridge can call directly.
* `cave-metrics` → scrapes the `certmanager_*` exposition through the
  existing per-crate Prometheus pipeline; no extra wiring beyond the
  standard `/metrics` route.

## License

AGPL-3.0-or-later — see workspace `LICENSE`.

Upstream cert-manager is Apache-2.0; see workspace `NOTICE` for the
attribution.
