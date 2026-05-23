# cave-cert-manager — PARITY_REPORT (Charter v2)

Status: **PASS** (8/8 gates, 2026-05-23 v2 re-run)

| Gate | Requirement | Evidence |
| ---- | ----------- | -------- |
| **G1 — always-latest** | upstream pinned to latest stable cert-manager (v1.20.2, 2026-04-11) | `parity.manifest.toml::[upstream] version`; re-verified 2026-05-23 via `gh release view cert-manager/cert-manager` (still latest, no bump needed) |
| **G2 — source_sha pin** | inline `source_sha` = annotated-tag commit | `parity.manifest.toml::[upstream] source_sha = "e5b7b18450dd2c4b993b95bcd680b1a057205b00"`; cross-verified via `gh api repos/cert-manager/cert-manager/git/tags/<tag-sha>` deref |
| **G3 — honest fill_ratio ≥ 0.95** | `(mapped + partial + skipped) / total` | `0.9500 = (23 + 5 + 10) / 40`; honest_ratio `0.7000 = (23 + 5) / 40` |
| **G4 — parity_ratio_source = "manifest"** | manifest count is the source of truth (not LOC, not heuristic) | `parity.manifest.toml::[parity] parity_ratio_source = "manifest"` |
| **G5 — AGPL SPDX header coverage 100%** | every `.rs` file in this crate carries the AGPL-3.0-or-later SPDX line | self-audit assertion 7 walks `crates/cave-cert-manager/`, asserts 0 missing across ≥ 15 `.rs` files |
| **G6 — no stub macros in src/** | no `todo!()`, `unimplemented!()`, `panic!("stub")`, `panic!("todo")` in `src/` (comments excluded) | self-audit assertion 8 walks `src/`, scans every non-comment line |
| **G7 — counts sum to total + ≥ 15 mapped** | `mapped + partial + skipped + unmapped == total` AND `mapped ≥ 15` | self-audit assertion 6: `23 + 5 + 10 + 2 == 40`; `23 ≥ 15` ✅ |
| **G8 — TDD + 4-track + ≥ 0.95 floor** | tests-first; backend modules + cavectl + HTTP routes + observability + smoke; floor 0.95 cleared | 105 lib tests + 11 self-audit + 5 smoke = **121 PASS** (v2 baseline; edge-case suite lands in subsequent commits) |

## v2 delta vs v1 (2026-05-22 → 2026-05-23)

* **+2 mapped subsystems** — `prometheus-metrics` (src/metrics.rs, 11 lib tests) + `certificate-revocation` (src/revocation.rs, 14 lib tests).
* **+27 lib tests** — 78 → 105.
* **+2 self-audit assertions** — 9 → 11 (metrics 5-family check + RFC 5280 reasonCode round-trip).
* **+1 audit floor** — fill_ratio floor 0.65 → 0.95.
* **+observability track** — 8 Grafana panels (`observability/grafana-dashboard.json`) + 5 Prometheus alert rules (`observability/prometheus-alerts.yaml`) driven off the new metrics module.
* **+README + CHANGELOG** — public-facing docs for v0.1.0 launch.
* **last_audit** — 2026-05-22 → 2026-05-23.

## Scope summary

**Mapped (23)** — Certificate / CertificateRequest / Issuer / ClusterIssuer CRDs; status conditions; ACME (issuer + HTTP-01 + DNS-01 with cave-dns); CA (3-tier hierarchy through cave-pki, one tenant intermediate per tenant_id); Vault (keychain-handle-only, token never in PEM); SelfSigned; issuer registry; renewal scheduler; secret materializer (kubernetes.io/tls shape, tls.key is a keychain handle); in-memory store with cross-tenant denial; reconcile loop; HTTP API surface (8 endpoints under `/api/cert/*`); cavectl URL builders; ReconcileEvent emitter; dnsNames validation (empty / slash / whitespace / 253-char RFC 1035); tenant scoping; **prometheus-metrics** (5-family exposition, deterministic ordering, per-tenant cardinality); **certificate-revocation** (RFC 5280 reasonCode ledger, idempotent revoke, hold/un-hold path, CRL-line render).

**Partial (5)** — Venafi issuer (model only, runtime Phase 2); ACME TLS-ALPN-01 (cave-acme generates the challenge, the solver lands with cave-gateway listener); ACME External Account Binding (cave-acme accepts EAB, AcmeIssuer currently passes None); secret-reconciler private-key bytes (currently emits keychain handle, raw bytes path lands with the keychain bridge); rotation policy `Always` (spec honoured, keymanager swap lands with the keychain bridge).

**Skipped (10) — `[[scope_cuts]]`** —
* `cave-cert-manager-cloud-issuers` — AWS PCA + Google CloudKMS adapters
* `cave-cert-manager-venafi-runtime` — Venafi TPP / Cloud SDK
* `cave-gateway` — Gateway-API HTTPRoute + Listener cert binding
* `cave-mesh` — istio-csr (gRPC SVID issuance)
* `cave-net` — mTLS certificate distribution
* `cave-admission` — webhook validation deep
* `cave-metrics` — central Prometheus aggregator + cross-crate Grafana dashboards (per-crate exposition lives here in `prometheus-metrics`)
* `cavectl` — cmctl binary (absorbed)
* `cave-deploy` — Helm chart installer
* `cave-cert-manager-acme-webhook-dns` — out-of-tree DNS solver gRPC webhook

**Unmapped (2) — honest gaps** —
* `multi-issuer-failover` — operator-requested fall-through; not upstream
* `experimental-clusterscoped-secret-rotation` — cert-manager v1.20 alpha gate

## Test summary

```
cargo test -p cave-cert-manager --lib                          → 105 PASS
cargo test -p cave-cert-manager --test parity_self_audit       →  11 PASS
cargo test -p cave-cert-manager --test smoke_end_to_end        →   5 PASS
                                                                 -------
                                                                  121 PASS (v1 baseline: 92)
```

(Edge-case test gap suite lands in `tests/test_gap_close_edges.rs` in subsequent commits — see CHANGELOG.)

## Security gates honoured

| Gate | Where |
| ---- | ----- |
| No API keys in code | `IssuerSpec::Acme.account_key_keychain_handle` + `IssuerSpec::Vault.token_keychain_handle` rejected if they don't start with `keychain:` |
| Secret material never in process memory beyond resolution | `VaultIssuer.seed_keychain` is the only entry point; `tls.key` materialised as a keychain reference, NEVER as raw bytes |
| Secret value never in logs / emitted PEM | `token_value_not_in_emitted_pem` test asserts the Vault token does not appear in `certificate_chain_pem` or `ca_pem` |
| Cross-tenant denied | every store + controller + revocation-ledger op runs through `check_tenant`; structured `CrossTenantDenied { owner_tenant, request_tenant }` error |
| dnsNames validation | empty, slash, whitespace, > 253 chars rejected per RFC 1035 |
| Metrics cardinality bounded | `forget_certificate()` drops all per-Certificate gauges on delete; per-tenant_id labels prevent cross-tenant aggregation leaks |
| Revocation reason 7 rejected | RFC 5280 reserves reasonCode 7 — `RevocationReason::from_reason_code(7)` returns `InvalidSpec` |

## Observability (v2 add-on)

* `observability/grafana-dashboard.json` — 8 panels: Ready/NotReady cert count, Cert expiration heatmap (notAfter), Renewal queue (next-renewal timestamps), ACME request rate per host, ACME error ratio (status 4xx/5xx), Controller sync rate per controller, Revocation events per tenant, Issuer health table.
* `observability/prometheus-alerts.yaml` — 5 rules: `CertManagerCertExpiringSoon` (within 7 d), `CertManagerCertExpired` (notAfter passed but Ready=True), `CertManagerAcmeErrorRateHigh` (>10% 5xx over 10 m), `CertManagerControllerStalled` (sync rate < 1/h), `CertManagerRevocationSpike` (>5 revokes/min per tenant).
* Both backed by the `certmanager_*` exposition emitted from `src/metrics.rs` — no external metric source.

## Integration notes (for downstream wiring)

* `cave-vault` → `cave-cert-manager`: production wiring populates `VaultIssuer.keychain` from the cave-vault keychain client at startup; per-handle reads are explicit lookups, NEVER fall-throughs.
* `cave-net` → `cave-cert-manager`: receives `Http01Plan` records to publish under `/.well-known/acme-challenge/`; receives `tls.key` keychain handles to resolve at TLS-handshake time.
* `cave-dns` → `cave-cert-manager`: receives `Dns01Plan` records (record_name = `_acme-challenge.<domain>`, digest = base64url(SHA-256(key_auth))) to publish as TXT entries.
* `cave-knative` → `cave-cert-manager`: previously parked as a Phase 2 backlog item (per memory `cave-knative` close); this crate now provides the surface (`IssuerSpec` + `Certificate` CRDs) the cave-knative cert-bridge can call directly.
* `cave-metrics` → `cave-cert-manager`: scrapes the `certmanager_*` exposition through the existing per-crate Prometheus pipeline; no extra wiring beyond the standard `/metrics` route.

## Closure

ADR-155 (Cert-Manager Adoption) documents the design choice + alternatives.
2026-05-23 — Charter v2 8-gate close v2 re-run. Workspace ≥ 0.95 count grows by 1.
