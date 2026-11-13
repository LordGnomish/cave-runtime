# cave-certs TDD coverage audit vs cert-manager v1.17.2

- **Crate**: `cave-certs` (theme: security)
- **Upstream**: https://github.com/cert-manager/cert-manager @ `v1.17.2`
- **Upstream test inventory**: 177 test files, 415 test symbols (`/tmp/tdd-audit/cave-certs-upstream-tests.txt`)
- **Cave test functions**: 111 `#[test]` across `src/*` inline modules + `tests/*` integration files
- **Audit date**: 2026-05-30
- **Verdict**: cave-certs already has *substantial* coverage. The pure validation
  engine (`validate_certificate_spec`, `validate_duration`, `validate_cr_approval_condition`,
  `validate_update_cr_approval_condition`), issuers, CSR, solvers, PQC container,
  renewal, store, ACME workflow and CRD validation are all well-exercised. Only a
  small number of genuinely uncovered, portable behavioral units remain.

## Classification of upstream behavioral units

| Upstream test / behavior | cave fn | Classification | Notes |
|---|---|---|---|
| `TestValidateCertificate` (SAN required, CN>64, IP, RSA/ECDSA size, revisionHistoryLimit) | `validate_certificate_spec` | covered | 20 tests in `webhook_validation_tdd.rs` |
| `TestValidateDuration` (min duration, renewBefore<duration, renewBefore min) | `validate_duration` (via spec) | covered | duration_below_minimum / renew_before tests |
| `validateIssuerRef` (name required, kind for cert-manager.io, group hint) | `validate_certificate_spec` | covered | issuer_ref_* tests |
| `validateUsages` (unknown keyusage) | `validate_certificate_spec` | covered | unknown_usage_rejected |
| `validateEmailAddresses` (reject bad / reject name-form) | `validate_certificate_spec` | covered (rejection only) | happy-path bare-email acceptance NOT asserted (minor) |
| `TestValidateCertificateRequest*` approval-condition rules | `validate_cr_approval_condition`, `validate_update_cr_approval_condition` | covered | 11 tests in `cr_approval_validation_tdd.rs` |
| CertificateRequest state machine (Pending→Approved→Issued / Denied) | `approve`, `deny`, `issue`, `try_deny` | **partially covered** | `try_deny` only exercises the Issued-state error branch; Approved + already-Denied branches uncovered |
| cert expiry / state computation | `compute_cert_state`, `covers_domain`, `expiring_soon` | covered | engine inline tests |
| days-until-expiry duration math (`pkg/util/pki` duration helpers) | `days_until_expiry` | **portable-coverage (uncovered)** | public fn, zero tests anywhere |
| `IssuerSpec` per-variant validation (CA secret, ACME email, Vault fields) | `IssuerSpec::validate` | covered (CA + ACME) / **partial (Vault)** | empty-Vault-field rejection branch not asserted |
| SelfSigned + CA issuance (`pkg/issuer/selfsigned`, `pkg/issuer/ca`) | `SelfSignedIssuer::issue`, `CaIssuer::sign`, `from_pem` | covered | issuers tests |
| `GenerateCSR` (`pkg/util/pki/csr.go`) | `CsrBuilder::build` | covered | csr + issuers tests |
| DNS-01 / HTTP-01 solver Present/CleanUp (RFC 8555 §8.3/§8.4) | `Dns01Solver`, `Http01Solver` | covered | solvers tests |
| ACME order workflow (register / new_order / finalize) | `acme_client` fns | covered | acme_client tests |
| Renewal trigger `shouldReissue` | `RenewalController::evaluate` | covered | 4-state test |
| Composite PQC container (assemble / split / verify) | `dual_sign`, `split_composite`, `verify_dual` | covered | pqc tests |
| `TestEncodeJKS/PKCS12Keystore/Truststore` | — | scope-cut | keystore encoding not in cave-certs (cave-vault / external) |
| `TestAppendCertificatesToBundle` (cainjector bundle) | — | scope-cut | cainjector controller, not ported |
| `TestValidNameserver`, `TestValidateTLSConfig`, `TestValidateLeaderElectionConfig`, config-defaults, roundtrip-types | — | scope-cut | controller/webhook config + apimachinery plumbing |
| `Test_SecretManagedLabels...`, secrets-manager, lister, gateway-listener, buildOrder controller internals | — | scope-cut | K8s controller reconcile loops (cross-crate: cave-admission / controllers) |
| DNS provider acme-dns/azure/route53 integration tests | — | scope-cut | external DNS vendor plumbing |
| cainjector / webhook / controller CLI flag parsing | — | scope-cut | binary entrypoint config |

## Recommended TDD fills (portable-coverage first)

These exercise existing public cave functions whose behavior is faithfully ported
from cert-manager but has no current test asserting it.

1. **`cave_certs::engine::days_until_expiry`** — PRIORITY. Public, zero coverage.
   Assert a positive count for a future `not_after`, a negative count for an
   already-expired cert, and ~0 at the boundary. Mirrors cert-manager
   `pkg/util/pki` duration math used by the trigger controller.

2. **`cave_certs::cert_request::CertificateRequest::try_deny`** (Approved branch) —
   PRIORITY. The Issued-state error branch is tested, but the documented
   "cannot deny an already-approved CertificateRequest; revoke instead" branch
   (and the already-`Denied` branch) are uncovered. Build a CR, `approve()`,
   then assert `try_deny(...).is_err()` and that state stays `Approved`; likewise
   `deny` then `try_deny` again returns the "already denied" error. Mirrors the
   cert-manager approval controller terminal-state guard.

### Secondary (thinner / partial) fills — optional

3. `cave_certs::crds::IssuerSpec::validate` (Vault variant) — current tests cover
   CA-empty-secret and ACME-bad-email rejection but not the
   `vault.server/path/role must all be non-empty` branch. A test passing an empty
   Vault `path` (or `role`) asserting `validate().is_err()` would close it.

4. `cave_certs::webhook_validation::validate_certificate_spec` (email happy path) —
   only rejection of bad/name-form emails is asserted. A test with a *valid* bare
   `emailAddresses` entry asserting no `spec.emailAddresses[*]` error would cover
   the accept path of `validateEmailAddresses`.

## Honest notes

The bulk of cert-manager's 415 test symbols are controller reconcile loops,
keystore encoding, DNS-vendor integrations, K8s apimachinery roundtrips, and CLI
flag parsing — all genuinely scope-cut (cross-crate cave-admission/controllers,
cave-vault, or vendor plumbing). The pure admission-validation algorithms that
cave ports are already densely tested. The two PRIORITY fills above are the only
solid uncovered portable units; items 3–4 are thin partials.
